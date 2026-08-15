//! Two things every other check module reads: what a declared schema *is* as a
//! CEL type, and when one declared schema **satisfies** another.
//!
//! # The two relations
//!
//! Grammar 8.0 splits the typing of a wire in two, and so does this module:
//!
//! * an **explicit binding** puts a CEL expression between the source and the
//!   destination, so it is typed by its result — [`cel::Type::assignable_to`],
//!   which knows nothing of `min_length` because a result carries no
//!   declaration to read one from;
//! * a **name-based read** and a **write** put nothing between two
//!   declarations, so the two type nodes meet directly and the source must
//!   satisfy the destination: every value the source can hold is a legal value
//!   of the destination's type node ([`satisfies`], grammar 7.5, 8.0, 10.2,
//!   Decisions D58, D111).
//!
//! [`satisfies`] reads constraints, and reads them as an **implication test
//!   decided from declarations alone**: a destination that declares a bound is
//!   satisfied only by a source that declares the same bound or a tighter one.
//! Nothing is inferred about the values themselves, because the implication
//! between two `pattern:`s — or between a bound and its absence — is not
//! decidable in general. That is the rule Decision D111 names when it says a
//! channel with no `max_items` cannot feed an `outputs:` field: only the
//! channel's own declaration can keep the field's promise.
//!
//! # Derived result schemas
//!
//! Three node kinds have an output nobody wrote down: a store op's is derived
//! from its row in grammar 11.4 and the store's own schemas (Decision D34), and
//! an inline `exec:`/`http:` node with no `output:` takes its kind default
//! (grammar 8.2, 8.3). [`node_output`](super::Ctx::node_output) synthesizes all
//! three as ordinary field maps carrying the node's span, so that every check
//! downstream — a write's typing, a guard's path, an unknown field — reads one
//! shape and reports against the node that produced it.

use std::sync::Arc;

use crate::ast::common::{Ident, Literal};
use crate::ast::definition::StoreKind;
use crate::ast::flow::StoreOp;
use crate::ast::schema::{Number, ScalarKind, Surface};
use crate::cel::ty::{ObjectShape, Origin, Property, Type, UnionShape, UnionVariant};
use crate::diag::{Span, Spanned};
use crate::ir::schema::{
    ArrayType, EnumType, Field, FieldMap, ObjectType, Scalar, TypeForm, TypeNode, UnionType,
};

/// Why one type node does not satisfy another.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Mismatch {
    /// The path from the two nodes that were compared to the ones that
    /// disagree: `.items`, `.author.email`. Empty at the top.
    pub(crate) path: Vec<String>,
    /// What the destination requires.
    pub(crate) expected: String,
    /// What the source offers.
    pub(crate) found: String,
}

impl Mismatch {
    fn at(mut self, segment: impl Into<String>) -> Self {
        self.path.insert(0, segment.into());
        self
    }

    fn new(expected: impl Into<String>, found: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            expected: expected.into(),
            found: found.into(),
        }
    }

    /// The sentence a diagnostic ends with: "expected …, found …" plus the
    /// sub-path when the disagreement is nested.
    pub(crate) fn describe(&self) -> String {
        let expected = &self.expected;
        let found = &self.found;
        if self.path.is_empty() {
            format!("expected {expected}, found {found}")
        } else {
            format!(
                "expected {expected}, found {found} at `{}`",
                self.path.join("")
            )
        }
    }
}

/// Whether every value `source` can hold is a legal value of `target`.
pub(crate) fn satisfies(source: &TypeNode, target: &TypeNode) -> Result<(), Mismatch> {
    match (&source.form, &target.form) {
        (TypeForm::Scalar(source), TypeForm::Scalar(target)) => scalars(source, target),
        (TypeForm::Enum(source), TypeForm::Enum(target)) => {
            let unknown: Vec<&str> = source
                .variants
                .iter()
                .filter(|variant| {
                    !target
                        .variants
                        .iter()
                        .any(|declared| declared.value == variant.value)
                })
                .map(|variant| variant.value.as_str())
                .collect();
            if unknown.is_empty() {
                Ok(())
            } else {
                Err(Mismatch::new(
                    format!("one of [{}]", variants(&target.variants)),
                    format!(
                        "one of [{}], which adds [{}]",
                        variants(&source.variants),
                        unknown.join(", ")
                    ),
                ))
            }
        }
        // Every enum member is a string (grammar 3.3), so an enum satisfies a
        // string destination whose constraints every variant meets.
        (TypeForm::Enum(source), TypeForm::Scalar(target)) if target.kind == ScalarKind::String => {
            if target.pattern.is_some() || target.format.is_some() {
                return Err(Mismatch::new(
                    "a string with the same `pattern`/`format`",
                    "an enum, whose variants cannot be checked against one",
                ));
            }
            for variant in &source.variants {
                let length = variant.value.chars().count() as i64;
                if target.min_length.is_some_and(|min| length < min)
                    || target.max_length.is_some_and(|max| length > max)
                {
                    return Err(Mismatch::new(
                        "a string within the declared length bounds",
                        format!("the variant `{}`, which is outside them", variant.value),
                    ));
                }
            }
            Ok(())
        }
        (TypeForm::Object(source), TypeForm::Object(target)) => objects(
            &source.properties,
            &source.optional,
            &target.properties,
            &target.optional,
        ),
        (TypeForm::Array(source), TypeForm::Array(target)) => {
            arrays(source, target)?;
            satisfies(&source.items, &target.items).map_err(|mismatch| mismatch.at(".items"))
        }
        (TypeForm::Union(source), TypeForm::Union(target)) => unions(source, target),
        (_, target_form) => Err(Mismatch::new(
            describe_form(target_form),
            describe_form(&source.form),
        )),
    }
}

/// Whether a field map satisfies another, as a closed object would.
pub(crate) fn field_maps_satisfy(source: &FieldMap, target: &FieldMap) -> Result<(), Mismatch> {
    objects(source, &[], target, &[])
}

fn objects(
    source: &FieldMap,
    source_optional: &[Spanned<Ident>],
    target: &FieldMap,
    target_optional: &[Spanned<Ident>],
) -> Result<(), Mismatch> {
    for field in &source.fields {
        if target
            .fields
            .iter()
            .all(|want| want.name.value != field.name.value)
        {
            // Objects are closed (Decision D8), so a value carrying a property
            // the destination does not declare is not a legal value of it.
            return Err(Mismatch::new(
                format!(
                    "an object declaring only {}",
                    crate::parse::reader::list(
                        target.fields.iter().map(|field| field.name.value.as_str())
                    )
                ),
                format!("one that also declares `{}`", field.name.value),
            ));
        }
    }
    for want in &target.fields {
        let Some(field) = source
            .fields
            .iter()
            .find(|f| f.name.value == want.name.value)
        else {
            if optional(want, target_optional) {
                continue;
            }
            return Err(Mismatch::new(
                format!("an object declaring `{}`", want.name.value),
                "one that does not".to_string(),
            ));
        };
        if !optional(want, target_optional) && optional(field, source_optional) {
            return Err(Mismatch::new(
                format!("`{}` to be required", want.name.value),
                "an optional property".to_string(),
            ));
        }
        satisfies(&field.ty, &want.ty)
            .map_err(|mismatch| mismatch.at(format!(".{}", want.name.value)))?;
    }
    Ok(())
}

fn unions(source: &UnionType, target: &UnionType) -> Result<(), Mismatch> {
    if source.discriminator.value != target.discriminator.value {
        return Err(Mismatch::new(
            format!("a `{}` union", target.discriminator.value),
            format!("a `{}` union", source.discriminator.value),
        ));
    }
    for variant in &source.variants {
        let Some(want) = target
            .variants
            .iter()
            .find(|declared| declared.tag.value == variant.tag.value)
        else {
            return Err(Mismatch::new(
                format!(
                    "a union of [{}]",
                    target
                        .variants
                        .iter()
                        .map(|v| v.tag.value.as_str())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                format!("one that adds the variant `{}`", variant.tag.value),
            ));
        };
        field_maps_satisfy(&variant.fields, &want.fields)
            .map_err(|mismatch| mismatch.at(format!(".{}", variant.tag.value)))?;
    }
    Ok(())
}

fn scalars(source: &Scalar, target: &Scalar) -> Result<(), Mismatch> {
    let widens = source.kind == target.kind
        || (source.kind == ScalarKind::Integer && target.kind == ScalarKind::Number);
    if !widens {
        return Err(Mismatch::new(
            scalar_name(target.kind),
            scalar_name(source.kind),
        ));
    }
    if target.kind.is_string() {
        if target
            .min_length
            .is_some_and(|min| source.min_length.is_none_or(|have| have < min))
        {
            return Err(bound("min_length", target.min_length, source.min_length));
        }
        if target
            .max_length
            .is_some_and(|max| source.max_length.is_none_or(|have| have > max))
        {
            return Err(bound("max_length", target.max_length, source.max_length));
        }
        if target.pattern.is_some() && target.pattern != source.pattern {
            return Err(Mismatch::new(
                format!(
                    "a string matching `{}`",
                    target.pattern.clone().unwrap_or_default()
                ),
                source.pattern.as_ref().map_or_else(
                    || "one with no `pattern`".to_string(),
                    |pattern| format!("one matching `{pattern}`"),
                ),
            ));
        }
        if target.format.is_some() && target.format != source.format {
            return Err(Mismatch::new(
                format!(
                    "a string in the `{}` format",
                    target.format.map(|f| f.as_str()).unwrap_or_default()
                ),
                source.format.map_or_else(
                    || "one with no `format`".to_string(),
                    |format| format!("one in the `{}` format", format.as_str()),
                ),
            ));
        }
    }
    if target.kind.is_numeric() {
        let lower = |scalar: &Scalar| {
            scalar
                .minimum
                .map(number)
                .into_iter()
                .chain(scalar.exclusive_minimum.map(number))
                .fold(f64::NEG_INFINITY, f64::max)
        };
        let upper = |scalar: &Scalar| {
            scalar
                .maximum
                .map(number)
                .into_iter()
                .chain(scalar.exclusive_maximum.map(number))
                .fold(f64::INFINITY, f64::min)
        };
        if lower(source) < lower(target) {
            return Err(Mismatch::new(
                format!("a number at or above {}", lower(target)),
                "one that is not bounded there".to_string(),
            ));
        }
        if upper(source) > upper(target) {
            return Err(Mismatch::new(
                format!("a number at or below {}", upper(target)),
                "one that is not bounded there".to_string(),
            ));
        }
        if let Some(step) = target.multiple_of.map(number) {
            let source_step = source.multiple_of.map(number);
            if source_step.is_none_or(|have| step == 0.0 || (have / step).fract() != 0.0) {
                return Err(Mismatch::new(
                    format!("a multiple of {step}"),
                    "a value with no compatible `multiple_of`".to_string(),
                ));
            }
        }
    }
    Ok(())
}

fn arrays(source: &ArrayType, target: &ArrayType) -> Result<(), Mismatch> {
    if target
        .max_items
        .is_some_and(|max| source.max_items.is_none_or(|have| have > max))
    {
        return Err(bound("max_items", target.max_items, source.max_items));
    }
    if target
        .min_items
        .is_some_and(|min| source.min_items.is_none_or(|have| have < min))
    {
        return Err(bound("min_items", target.min_items, source.min_items));
    }
    if target.unique_items == Some(true) && source.unique_items != Some(true) {
        return Err(Mismatch::new(
            "an array declaring `unique_items: true`",
            "one that does not",
        ));
    }
    Ok(())
}

fn bound(key: &str, target: Option<i64>, source: Option<i64>) -> Mismatch {
    Mismatch::new(
        format!(
            "a `{key}` of {}",
            target.map_or_else(String::new, |value| value.to_string())
        ),
        source.map_or_else(
            || format!("no declared `{key}`"),
            |value| format!("`{key}: {value}`"),
        ),
    )
}

fn optional(field: &Field, optional: &[Spanned<Ident>]) -> bool {
    default_of(&field.ty).is_some() || optional.iter().any(|name| name.value == field.name.value)
}

fn variants(variants: &[Spanned<String>]) -> String {
    variants
        .iter()
        .map(|variant| variant.value.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A scalar's name with its article, as a diagnostic reads it.
fn scalar_name(kind: ScalarKind) -> String {
    match kind {
        ScalarKind::Integer => "an integer".to_string(),
        other => format!("a {}", other.as_str()),
    }
}

fn number(value: Number) -> f64 {
    match value {
        Number::Int(value) => value as f64,
        Number::Float(value) => value,
    }
}

/// A type node's `default:`, whichever form carries it.
pub(crate) fn default_of(node: &TypeNode) -> Option<&Spanned<Literal>> {
    match &node.form {
        TypeForm::Scalar(scalar) => scalar.default.as_ref(),
        TypeForm::Enum(enumeration) => enumeration.default.as_ref(),
        TypeForm::Object(object) => object.default.as_ref(),
        TypeForm::Array(array) => array.default.as_ref(),
        TypeForm::Union(_) => None,
    }
}

/// How a form is named where the two sides are not even the same shape.
fn describe_form(form: &TypeForm) -> String {
    match form {
        TypeForm::Scalar(scalar) => scalar_name(scalar.kind),
        TypeForm::Enum(enumeration) => format!("one of [{}]", variants(&enumeration.variants)),
        TypeForm::Object(_) => "an object".to_string(),
        TypeForm::Array(array) => format!("an array of {}", type_of(&array.items).bare()),
        TypeForm::Union(union) => format!("a `{}` union", union.discriminator.value),
    }
}

/// The CEL type of a declared type node, named for the diagnostics its members
/// draw.
///
/// Only an object carries a name, and only because an unknown member has to
/// say what it was looked for in: everything else is described by its own
/// shape.
pub(crate) fn type_of_named(node: &TypeNode, label: impl Into<String>) -> Type {
    match type_of(node) {
        Type::Object(shape) => Type::Object(Arc::new(ObjectShape {
            origin: Origin::Declared(label.into()),
            properties: shape.properties.clone(),
        })),
        other => other,
    }
}

/// The CEL type of a declared type node.
pub(crate) fn type_of(node: &TypeNode) -> Type {
    match &node.form {
        TypeForm::Scalar(scalar) => Type::scalar(scalar.kind),
        TypeForm::Enum(enumeration) => Type::Enum(Arc::new(
            enumeration
                .variants
                .iter()
                .map(|variant| variant.value.clone())
                .collect(),
        )),
        TypeForm::Object(object) => Type::Object(Arc::new(ObjectShape {
            origin: Origin::Declared("this object".to_string()),
            properties: properties(&object.properties, &object.optional),
        })),
        TypeForm::Array(array) => Type::list(type_of(&array.items)),
        TypeForm::Union(union) => Type::Union(Arc::new(UnionShape {
            discriminator: union.discriminator.value.to_string(),
            variants: union
                .variants
                .iter()
                .map(|variant| UnionVariant {
                    tag: variant.tag.value.to_string(),
                    properties: properties(&variant.fields, &[]),
                })
                .collect(),
        })),
    }
}

/// The CEL type of a whole field map, named for the diagnostics its members
/// draw.
pub(crate) fn object_of(map: &FieldMap, label: impl Into<String>) -> Type {
    Type::Object(Arc::new(ObjectShape {
        origin: Origin::Declared(label.into()),
        properties: properties(map, &[]),
    }))
}

fn properties(map: &FieldMap, optional_names: &[Spanned<Ident>]) -> Vec<Property> {
    map.fields
        .iter()
        .map(|field| Property {
            name: field.name.value.to_string(),
            // A nested object is named by the field that declares it, so a
            // member it does not have is reported against that name rather
            // than against an anonymous shape.
            ty: type_of_named(&field.ty, format!("`{}`", field.name.value)),
            optional: optional(field, optional_names),
        })
        .collect()
}

// --- synthesized schemas -------------------------------------------------

/// A type node with no constraints and no description, carrying `span`.
pub(crate) fn node(form: TypeForm, span: &Span) -> TypeNode {
    TypeNode {
        description: None,
        span: span.clone(),
        form,
    }
}

/// `{ type: <kind> }`.
pub(crate) fn scalar_node(kind: ScalarKind, span: &Span) -> TypeNode {
    node(
        TypeForm::Scalar(Scalar {
            kind,
            min_length: None,
            max_length: None,
            pattern: None,
            format: None,
            minimum: None,
            maximum: None,
            exclusive_minimum: None,
            exclusive_maximum: None,
            multiple_of: None,
            default: None,
        }),
        span,
    )
}

/// `{ type: array, items: <items>, max_items: <max> }`.
pub(crate) fn array_node(items: TypeNode, max_items: Option<i64>, span: &Span) -> TypeNode {
    node(
        TypeForm::Array(ArrayType {
            items: Box::new(items),
            max_items,
            min_items: None,
            unique_items: None,
            default: None,
        }),
        span,
    )
}

/// `{ type: object, properties: <fields> }`.
pub(crate) fn object_node(fields: Vec<(&str, TypeNode)>, span: &Span) -> TypeNode {
    node(
        TypeForm::Object(ObjectType {
            properties: field_map(fields, span),
            optional: Vec::new(),
            default: None,
        }),
        span,
    )
}

/// A result-surface field map of these fields, carrying `span`.
pub(crate) fn field_map(fields: Vec<(&str, TypeNode)>, span: &Span) -> FieldMap {
    FieldMap {
        surface: Surface::Result,
        fields: fields
            .into_iter()
            .map(|(name, ty)| Field {
                name: Spanned::new(Ident::new(name), span.clone()),
                ty,
            })
            .collect(),
        span: span.clone(),
    }
}

/// An enum type node, for a synthesized field whose values are closed.
pub(crate) fn enum_node(variants: &[&str], span: &Span) -> TypeNode {
    node(
        TypeForm::Enum(EnumType {
            variants: variants
                .iter()
                .map(|variant| Spanned::new((*variant).to_string(), span.clone()))
                .collect(),
            default: None,
        }),
        span,
    )
}

/// The kind default of an inline `exec:` node's `output` (grammar 8.2).
pub(crate) fn exec_default_output(span: &Span) -> FieldMap {
    field_map(
        vec![
            ("exit_code", scalar_node(ScalarKind::Integer, span)),
            ("stdout", scalar_node(ScalarKind::String, span)),
        ],
        span,
    )
}

/// The kind default of an inline `http:` node's `output` (grammar 8.3).
pub(crate) fn http_default_output(span: &Span) -> FieldMap {
    field_map(
        vec![
            ("status", scalar_node(ScalarKind::Integer, span)),
            ("body", scalar_node(ScalarKind::String, span)),
        ],
        span,
    )
}

/// A store op's derived output schema (grammar 11.4, Decision D34).
///
/// Two of the row's shapes are decided by the node rather than by the store:
/// a `search`'s `matches` and a `list`'s `keys` are bounded by the `top_k` and
/// `limit` the node declares, which is the only bound either result has and is
/// what lets one be written to a bounded channel at all (grammar 3.5).
///
/// The catalog marks one field optional — `value` on a `kv`/`blob` `get`,
/// absent on a miss — and that is a runtime fact with no static consequence
/// (Decision D110: the read fails, the write does not happen), so the
/// synthesized map carries every field alike.
pub(crate) fn store_output(
    kind: StoreKind,
    op: StoreOp,
    value_schema: Option<&FieldMap>,
    metadata_schema: Option<&FieldMap>,
    bound: Option<i64>,
    span: &Span,
) -> FieldMap {
    let string = || scalar_node(ScalarKind::String, span);
    match (kind, op) {
        (StoreKind::Kv, StoreOp::Get) => {
            let value = value_schema.map_or_else(
                || object_node(Vec::new(), span),
                |schema| {
                    node(
                        TypeForm::Object(ObjectType {
                            properties: schema.clone(),
                            optional: Vec::new(),
                            default: None,
                        }),
                        span,
                    )
                },
            );
            field_map(
                vec![
                    ("value", value),
                    ("found", scalar_node(ScalarKind::Boolean, span)),
                ],
                span,
            )
        }
        (StoreKind::Blob, StoreOp::Get) => field_map(
            vec![
                ("value", string()),
                ("found", scalar_node(ScalarKind::Boolean, span)),
            ],
            span,
        ),
        (StoreKind::Kv, StoreOp::Set) | (StoreKind::Blob, StoreOp::Put) => {
            field_map(vec![("key", string())], span)
        }
        (_, StoreOp::Delete) => field_map(
            vec![("deleted", scalar_node(ScalarKind::Boolean, span))],
            span,
        ),
        (_, StoreOp::List) => field_map(vec![("keys", array_node(string(), bound, span))], span),
        (StoreKind::Vector, StoreOp::Search) => {
            let mut properties = vec![
                ("id", string()),
                ("score", scalar_node(ScalarKind::Number, span)),
                ("text", string()),
            ];
            // A store that declares no `metadata_schema` has no metadata at
            // all, so a match carries no `metadata` field to read — as against
            // `metadata_schema: {}`, which declares one that is always empty
            // (Decision D114).
            let metadata;
            if let Some(schema) = metadata_schema {
                metadata = node(
                    TypeForm::Object(ObjectType {
                        properties: schema.clone(),
                        optional: Vec::new(),
                        default: None,
                    }),
                    span,
                );
                properties.push(("metadata", metadata));
            }
            field_map(
                vec![(
                    "matches",
                    array_node(object_node(properties, span), bound, span),
                )],
                span,
            )
        }
        (StoreKind::Vector, StoreOp::Upsert) => field_map(vec![("id", string())], span),
        // Every remaining pair is refused by the parser, which reads the op
        // against its kind's row before the IR exists (grammar 11.4).
        _ => field_map(Vec::new(), span),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diag::{Position, SourceName};

    fn span() -> Span {
        Span::new(
            SourceName::new("t.yml"),
            0..1,
            Position::new(1, 1),
            Position::new(1, 2),
        )
    }

    fn string_with(min_length: Option<i64>) -> TypeNode {
        let mut node = scalar_node(ScalarKind::String, &span());
        if let TypeForm::Scalar(scalar) = &mut node.form {
            scalar.min_length = min_length;
        }
        node
    }

    #[test]
    fn a_tighter_source_satisfies_a_looser_destination() {
        assert!(satisfies(&string_with(Some(1)), &string_with(None)).is_ok());
        // …and not the other way round: the source may hold the empty string.
        let mismatch = satisfies(&string_with(None), &string_with(Some(1))).unwrap_err();
        assert!(mismatch.describe().contains("min_length"));
    }

    #[test]
    fn an_unbounded_array_cannot_feed_a_bounded_one() {
        let bounded = array_node(scalar_node(ScalarKind::String, &span()), Some(10), &span());
        let unbounded = array_node(scalar_node(ScalarKind::String, &span()), None, &span());
        assert!(satisfies(&bounded, &unbounded).is_ok());
        assert!(satisfies(&unbounded, &bounded).is_err());
        let wider = array_node(scalar_node(ScalarKind::String, &span()), Some(11), &span());
        assert!(satisfies(&wider, &bounded).is_err());
    }

    #[test]
    fn an_integer_widens_to_a_number_but_not_back() {
        let integer = scalar_node(ScalarKind::Integer, &span());
        let number = scalar_node(ScalarKind::Number, &span());
        assert!(satisfies(&integer, &number).is_ok());
        assert!(satisfies(&number, &integer).is_err());
    }

    #[test]
    fn objects_are_closed_in_both_directions() {
        let one = object_node(vec![("a", string_with(None))], &span());
        let two = object_node(
            vec![("a", string_with(None)), ("b", string_with(None))],
            &span(),
        );
        assert!(satisfies(&one, &one).is_ok());
        // The source may hold `{a, b}`, which is not a legal `{a}`.
        assert!(satisfies(&two, &one).is_err());
        // …and it may hold `{a}`, which is missing the required `b`.
        assert!(satisfies(&one, &two).is_err());
    }

    #[test]
    fn a_nested_mismatch_names_its_path() {
        let source = object_node(
            vec![(
                "author",
                object_node(
                    vec![("name", scalar_node(ScalarKind::Integer, &span()))],
                    &span(),
                ),
            )],
            &span(),
        );
        let target = object_node(
            vec![(
                "author",
                object_node(vec![("name", string_with(None))], &span()),
            )],
            &span(),
        );
        let mismatch = satisfies(&source, &target).unwrap_err();
        assert_eq!(
            mismatch.describe(),
            "expected a string, found an integer at `.author.name`"
        );
    }

    #[test]
    fn a_vector_store_with_no_metadata_schema_derives_no_metadata_field() {
        let derived = store_output(
            StoreKind::Vector,
            StoreOp::Search,
            None,
            None,
            Some(5),
            &span(),
        );
        let Some(matches) = derived.fields.first() else {
            panic!("a search derives `matches`");
        };
        let TypeForm::Array(array) = &matches.ty.form else {
            panic!("`matches` is an array");
        };
        assert_eq!(array.max_items, Some(5));
        let TypeForm::Object(item) = &array.items.form else {
            panic!("a match is an object");
        };
        assert_eq!(
            item.properties
                .fields
                .iter()
                .map(|field| field.name.value.to_string())
                .collect::<Vec<_>>(),
            ["id", "score", "text"]
        );
    }
}
