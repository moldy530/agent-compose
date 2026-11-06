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
use crate::ast::definition::{AgentAccess, Builtin, StoreKind};
use crate::ast::flow::{BATCH_MAX_ENTRIES, GET_MANY_MAX_KEYS, StoreOp};
use crate::ast::schema::{Number, ScalarKind, Surface};
use crate::cel::ty::{ObjectShape, Origin, Property, Type, UnionShape, UnionVariant};
use crate::diag::{Span, Spanned};
use crate::ir::schema::{
    ArrayType, EnumType, Field, FieldMap, ObjectType, Scalar, TypeForm, TypeNode, UnionType,
};

/// Why one type node does not satisfy another.
///
/// The same [`Mismatch`](crate::cel::ty::Mismatch) the expression-level
/// relation reports: grammar 8.0 splits a wire into two typings, and an author
/// meeting one of them should not have to learn a second vocabulary for the
/// other.
pub(crate) use crate::cel::ty::Mismatch;

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

/// Whether two declarations are the **same type**: the same form, the same
/// constraints, the same members — wherever each was written.
///
/// [`TypeNode`] derives `PartialEq`, and every node carries the [`Span`] it was
/// written at (as does every `default:` literal inside one), so `==` answers
/// "is this the same *declaration*" and never "are these the same *type*": two
/// fields spelled identically in two places are never equal under it. The rule
/// that asks the second question is the narrowing a `default:` route gets — the
/// discriminator plus the fields **every** unrouted variant declares (grammar
/// 8.6 rule 4, Decision D30) — where the two declarations are two variants of
/// one union and so are never one source region.
///
/// The relation is deliberately strict: declaration order counts, because it is
/// the order a structured-output schema carries the members in (grammar 3.8),
/// and every constraint counts, because two fields that admit different values
/// are two fields whatever they are named.
pub(crate) fn identical(left: &TypeNode, right: &TypeNode) -> bool {
    match (&left.form, &right.form) {
        (TypeForm::Scalar(left), TypeForm::Scalar(right)) => {
            left.kind == right.kind
                && left.min_length == right.min_length
                && left.max_length == right.max_length
                && left.pattern == right.pattern
                && left.format == right.format
                && left.minimum == right.minimum
                && left.maximum == right.maximum
                && left.exclusive_minimum == right.exclusive_minimum
                && left.exclusive_maximum == right.exclusive_maximum
                && left.multiple_of == right.multiple_of
                && same_default(left.default.as_ref(), right.default.as_ref())
        }
        (TypeForm::Enum(left), TypeForm::Enum(right)) => {
            same_values(&left.variants, &right.variants)
                && same_default(left.default.as_ref(), right.default.as_ref())
        }
        (TypeForm::Object(left), TypeForm::Object(right)) => {
            same_field_maps(&left.properties, &right.properties)
                && same_values(&left.optional, &right.optional)
                && same_default(left.default.as_ref(), right.default.as_ref())
        }
        (TypeForm::Array(left), TypeForm::Array(right)) => {
            left.max_items == right.max_items
                && left.min_items == right.min_items
                && left.unique_items == right.unique_items
                && identical(&left.items, &right.items)
                && same_default(left.default.as_ref(), right.default.as_ref())
        }
        (TypeForm::Union(left), TypeForm::Union(right)) => {
            left.discriminator.value == right.discriminator.value
                && left.variants.len() == right.variants.len()
                && left
                    .variants
                    .iter()
                    .zip(&right.variants)
                    .all(|(left, right)| {
                        left.tag.value == right.tag.value
                            && same_field_maps(&left.fields, &right.fields)
                    })
        }
        _ => false,
    }
}

fn same_field_maps(left: &FieldMap, right: &FieldMap) -> bool {
    left.fields.len() == right.fields.len()
        && left.fields.iter().zip(&right.fields).all(|(left, right)| {
            left.name.value == right.name.value && identical(&left.ty, &right.ty)
        })
}

/// Two spanned values, compared by what was written rather than by where.
fn same_values<T: PartialEq>(left: &[Spanned<T>], right: &[Spanned<T>]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(left, right)| left.value == right.value)
}

fn same_default(left: Option<&Spanned<Literal>>, right: Option<&Spanned<Literal>>) -> bool {
    match (left, right) {
        (None, None) => true,
        (Some(left), Some(right)) => same_literal(&left.value, &right.value),
        _ => false,
    }
}

/// A literal's own `PartialEq` reaches the spans its sequence entries and
/// mapping keys carry, so it needs the same span-free reading.
fn same_literal(left: &Literal, right: &Literal) -> bool {
    match (left, right) {
        (Literal::Null, Literal::Null) => true,
        (Literal::Bool(left), Literal::Bool(right)) => left == right,
        (Literal::Int(left), Literal::Int(right)) => left == right,
        (Literal::Float(left), Literal::Float(right)) => left == right,
        (Literal::String(left), Literal::String(right)) => left == right,
        (Literal::Sequence(left), Literal::Sequence(right)) => {
            left.len() == right.len()
                && left
                    .iter()
                    .zip(right)
                    .all(|(left, right)| same_literal(&left.value, &right.value))
        }
        (Literal::Mapping(left), Literal::Mapping(right)) => {
            left.len() == right.len()
                && left.iter().zip(right).all(|(left, right)| {
                    left.key.value == right.key.value
                        && same_literal(&left.value.value, &right.value.value)
                })
        }
        _ => false,
    }
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
            if unsupplied(want, target_optional) {
                continue;
            }
            return Err(Mismatch::new(
                format!("an object declaring `{}`", want.name.value),
                "one that does not".to_string(),
            ));
        };
        if !unsupplied(want, target_optional) && absent(field, source_optional) {
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
        for end in [End::Lower, End::Upper] {
            let (have, want) = (bound_at(source, end), bound_at(target, end));
            if !have.within(want) {
                return Err(Mismatch::new(want.expected(), have.found(want)));
            }
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

/// Which end of a numeric range a bound is.
#[derive(Clone, Copy)]
enum End {
    Lower,
    Upper,
}

/// One end of the range a scalar admits: the value, and whether the value
/// itself is one of them.
///
/// The strictness is half the bound and cannot be folded away: `minimum: 0` and
/// `exclusive_minimum: 0` name the same number and admit different sets, so a
/// source declaring the first does not satisfy a destination declaring the
/// second (grammar 3.3, Decision D111).
#[derive(Clone, Copy)]
struct Bound {
    value: f64,
    inclusive: bool,
    end: End,
}

impl Bound {
    /// Whether every value this bound admits at its end is admitted by
    /// `other` — a tighter bound, or the same one no less strict.
    fn within(self, other: Self) -> bool {
        let tighter = match self.end {
            End::Lower => self.value > other.value,
            End::Upper => self.value < other.value,
        };
        tighter || (self.value == other.value && (other.inclusive || !self.inclusive))
    }

    /// How a destination's bound reads in a diagnostic.
    fn expected(self) -> String {
        let relation = match (self.end, self.inclusive) {
            (End::Lower, true) => "at or above",
            (End::Lower, false) => "above",
            (End::Upper, true) => "at or below",
            (End::Upper, false) => "below",
        };
        format!("a number {relation} {}", self.value)
    }

    /// How a source's own bound reads against the destination's: the two ways
    /// it can be too wide are reaching past the value and admitting it.
    fn found(self, target: Self) -> String {
        if self.value == target.value {
            format!("one that admits {}", self.value)
        } else {
            "one that is not bounded there".to_string()
        }
    }
}

/// A scalar's bound at one end. A scalar may declare both spellings, and the
/// range it admits is then the tighter of the two — with `exclusive_*` winning
/// a tie, being the stricter reading of the same number.
fn bound_at(scalar: &Scalar, end: End) -> Bound {
    let (inclusive, exclusive, unbounded) = match end {
        End::Lower => (scalar.minimum, scalar.exclusive_minimum, f64::NEG_INFINITY),
        End::Upper => (scalar.maximum, scalar.exclusive_maximum, f64::INFINITY),
    };
    let mut bound = Bound {
        value: unbounded,
        inclusive: true,
        end,
    };
    // The exclusive spelling is considered second, so it takes a tie.
    for declared in [
        inclusive.map(|value| Bound {
            value: number(value),
            inclusive: true,
            end,
        }),
        exclusive.map(|value| Bound {
            value: number(value),
            inclusive: false,
            end,
        }),
    ]
    .into_iter()
    .flatten()
    {
        if declared.within(bound) {
            bound = declared;
        }
    }
    bound
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

/// Whether a **value** of the object may legally omit this property: it is one
/// its `optional:` names (grammar 3.4, Decision D110).
///
/// A `default:` is deliberately not this. Grammar 3.6 makes a defaulted
/// property "implicitly optional **at its surface**" — a *binding* need not
/// supply it, and grammar 8.0's step 4 then supplies the default — so the value
/// the declaration describes carries the property either way. Reading the two
/// as one concept is what would make a channel unreadable by name for the
/// crime of declaring a convenience default on one of its properties.
fn absent(field: &Field, optional: &[Spanned<Ident>]) -> bool {
    optional.iter().any(|name| name.value == field.name.value)
}

/// Whether a value need not **supply** this property: it is `optional:`, or it
/// declares a `default:` the surface fills in (grammar 3.4, 3.6, 8.0 step 4).
///
/// This is the destination's question, and the only one a `default:` answers.
fn unsupplied(field: &Field, optional: &[Spanned<Ident>]) -> bool {
    default_of(&field.ty).is_some() || absent(field, optional)
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
pub(crate) fn describe_form(form: &TypeForm) -> String {
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
/// shape. An array passes the name down to its `items:`, because the object a
/// path reaches through an index is the one the reader named — `matches[0].id`
/// looks its member up in an item of `matches`.
pub(crate) fn type_of_named(node: &TypeNode, label: impl Into<String>) -> Type {
    match &node.form {
        TypeForm::Object(object) => Type::Object(Arc::new(ObjectShape {
            origin: Origin::Declared(label.into()),
            properties: properties(&object.properties, &object.optional),
        })),
        TypeForm::Array(array) => Type::list(type_of_named(
            &array.items,
            format!("an item of {}", label.into()),
        )),
        _ => type_of(node),
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
            optional: absent(field, optional_names),
            defaulted: default_of(&field.ty).is_some(),
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
/// Three of the row's shapes are decided by the node rather than by the store:
/// a `search`'s `matches` and a `list`'s `keys` (and `entries`) are bounded by
/// the `top_k` and `limit` the node declares, which is the only bound either
/// result has and is what lets one be written to a bounded channel at all
/// (grammar 3.5); a `get_many`'s `entries` by [`GET_MANY_MAX_KEYS`], the most
/// one reads; a `batch`'s `results` by its entry count, one each; and whether a `kv` `list` answers `entries` at all by the
/// `include_values` literal the node declares (Decision D154) — the same
/// presence-by-literal Decision D114 gives a match's `metadata`, decided by the
/// node here rather than by the store.
///
/// The catalog marks seven fields optional — `value` on a `kv`/`blob` `get`,
/// absent on a miss; `version` on every `kv` row that carries one, absent where
/// the key holds nothing; `value` on a partial update, absent where its
/// condition did not hold; a `list`'s `next`, absent on the last page; a
/// `get_many` entry's `value` and `version`, absent where `found` is `false`;
/// and a `batch`'s `failed`, absent where it applied, and its results'
/// `version`, absent on a `delete` entry — and that is a runtime fact with no static consequence (Decision D110: the
/// read fails, the write does not happen), so the synthesized map carries every
/// field alike. A store row is never parsed against it (`codegen::graph`), so
/// nothing refuses the answer that leaves one out.
///
/// A `kv` **write**'s row is PRD resolved q68's: `applied` says whether the
/// write's condition held and it landed, `version` is the key's version after
/// it (or the one it still holds, where it declined — a `delete` included, and
/// absent after a `delete` that applied, since the key then holds nothing), and
/// `deduped` says the answer is a first attempt's, handed back by the
/// idempotency ledger rather than performed again.
pub(crate) fn store_output(
    kind: StoreKind,
    op: StoreOp,
    value_schema: Option<&FieldMap>,
    metadata_schema: Option<&FieldMap>,
    bound: Option<i64>,
    include_values: bool,
    span: &Span,
) -> FieldMap {
    let string = || scalar_node(ScalarKind::String, span);
    let boolean = || scalar_node(ScalarKind::Boolean, span);
    let version = || scalar_node(ScalarKind::Integer, span);
    let stored = || {
        value_schema.map_or_else(
            || object_node(Vec::new(), span),
            |schema| object_node_of(schema.clone(), span),
        )
    };
    match (kind, op) {
        (StoreKind::Kv, StoreOp::Get) => field_map(
            vec![
                ("value", stored()),
                ("found", boolean()),
                ("version", version()),
            ],
            span,
        ),
        (StoreKind::Blob, StoreOp::Get) => {
            field_map(vec![("value", string()), ("found", boolean())], span)
        }
        (StoreKind::Kv, StoreOp::Set) => field_map(
            vec![
                ("key", string()),
                ("applied", boolean()),
                ("version", version()),
                ("deduped", boolean()),
            ],
            span,
        ),
        (StoreKind::Blob, StoreOp::Put) => field_map(vec![("key", string())], span),
        (StoreKind::Kv, StoreOp::Delete) => field_map(
            vec![
                ("deleted", boolean()),
                ("applied", boolean()),
                ("version", version()),
                ("deduped", boolean()),
            ],
            span,
        ),
        (_, StoreOp::Delete) => field_map(vec![("deleted", boolean())], span),
        (StoreKind::Kv, StoreOp::Merge | StoreOp::Append | StoreOp::Incr) => field_map(
            vec![
                ("key", string()),
                ("applied", boolean()),
                ("version", version()),
                ("value", stored()),
                ("deduped", boolean()),
            ],
            span,
        ),
        // One page of a `kv` partition's keys, past an exclusive cursor (PRD
        // resolved q70): `next` is the cursor of the page after it, present
        // exactly when one remains, and `entries` is there exactly when the
        // node's `include_values:` literal is `true` (Decision D154).
        (StoreKind::Kv, StoreOp::List) => {
            let mut fields = vec![
                ("keys", array_node(string(), bound, span)),
                ("next", string()),
            ];
            if include_values {
                fields.push((
                    "entries",
                    array_node(
                        object_node(
                            vec![
                                ("key", string()),
                                ("value", stored()),
                                ("version", version()),
                            ],
                            span,
                        ),
                        bound,
                        span,
                    ),
                ));
            }
            field_map(fields, span)
        }
        (StoreKind::Kv, StoreOp::Count) => field_map(
            vec![("count", scalar_node(ScalarKind::Integer, span))],
            span,
        ),
        // One entry per key asked for, in the order asked — so its bound is the
        // most keys one `get_many` reads, not anything the store holds.
        (StoreKind::Kv, StoreOp::GetMany) => field_map(
            vec![(
                "entries",
                array_node(
                    object_node(
                        vec![
                            ("key", string()),
                            ("value", stored()),
                            ("found", boolean()),
                            ("version", version()),
                        ],
                        span,
                    ),
                    Some(GET_MANY_MAX_KEYS),
                    span,
                ),
            )],
            span,
        ),
        // A batch answers once for all of its entries (PRD resolved q71):
        // whether they landed, whether the ledger handed the answer back, one
        // `{ key, version }` per entry in the order they applied — `version`
        // absent on a `delete` entry, whose key then holds nothing — and the
        // index of the entry whose condition did not hold, present exactly
        // when `applied` is `false`, which is also when `results` is empty.
        (StoreKind::Kv, StoreOp::Batch) => field_map(
            vec![
                ("applied", boolean()),
                ("deduped", boolean()),
                (
                    "results",
                    array_node(
                        object_node(vec![("key", string()), ("version", version())], span),
                        bound.or(i64::try_from(BATCH_MAX_ENTRIES).ok()),
                        span,
                    ),
                ),
                ("failed", scalar_node(ScalarKind::Integer, span)),
            ],
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
            if let Some(schema) = metadata_schema {
                properties.push(("metadata", object_node_of(schema.clone(), span)));
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
        // Every remaining pair is an op its store's kind does not take, which
        // the validator refuses against the store (`check::stores`, grammar
        // 11.4): there is no row to derive, and the node is already an error.
        _ => field_map(Vec::new(), span),
    }
}

/// Whether a store of this kind takes this op — grammar 11.4's table, read
/// by row through [`StoreKind::operations`], which is the one place it is
/// written.
pub(crate) fn kind_takes(kind: StoreKind, op: StoreOp) -> bool {
    kind.operations().contains(&op.as_str())
}

/// The fields of `V` an `append` may name: its array fields, in declaration
/// order, each with the item type it holds.
pub(crate) fn array_fields(value_schema: Option<&FieldMap>) -> Vec<(&Field, &TypeNode)> {
    value_schema
        .map(|schema| {
            schema
                .fields
                .iter()
                .filter_map(|field| match &field.ty.form {
                    TypeForm::Array(array) => Some((field, array.items.as_ref())),
                    _ => None,
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The fields of `V` an `incr` may name: its `integer` and `number` fields, in
/// declaration order.
pub(crate) fn numeric_fields(value_schema: Option<&FieldMap>) -> Vec<&Field> {
    value_schema
        .map(|schema| {
            schema
                .fields
                .iter()
                .filter(|field| {
                    matches!(
                        &field.ty.form,
                        TypeForm::Scalar(Scalar {
                            kind: ScalarKind::Integer | ScalarKind::Number,
                            ..
                        })
                    )
                })
                .collect()
        })
        .unwrap_or_default()
}

/// The tools an agent-attached store synthesizes, in grammar 11.5's own order.
///
/// The table is 11.5's, read through `agent_access:` (Decisions D37, D152):
/// `read` grants the reading ops, `read_write` — the default — adds the
/// whole-value write, and `full` adds the rest of a `kv` store's writes:
/// `merge`, `append`, `incr` and `delete` (PRD resolved q68). All four, on
/// every `kv` store, because that is the set the ruling names: the tool set is
/// the access level's and not the schema's, so a `V` with no array field still
/// offers `<name>_append` — whose `item` then has no field to name, so every
/// call is refused back to the model ([`store_tool_input`]) — and one with no
/// numeric field still offers `<name>_incr`.
///
/// No level offers a `batch` (PRD resolved q71): the ruling withholds it from
/// agents at every access level, so it is a store-op node's alone, and the
/// unit tests below pin that no row here ever names it.
///
/// `full` is a `kv` level alone: the ruling defines it by `kv`'s tools, and the
/// parser refuses it on a `vector` or `blob` store (`parse::definition`), so no
/// validated composition reaches those two arms with it. They answer
/// `read_write`'s set — never more — because the match has to be total, and an
/// IR that skipped the parser is still not handed a write it did not ask for.
pub(crate) fn store_tools(kind: StoreKind, access: AgentAccess) -> Vec<StoreOp> {
    match (kind, access) {
        (StoreKind::Kv, AgentAccess::Read) => vec![StoreOp::Get],
        (StoreKind::Kv, AgentAccess::ReadWrite) => vec![StoreOp::Get, StoreOp::Set],
        (StoreKind::Kv, AgentAccess::Full) => vec![
            StoreOp::Get,
            StoreOp::Set,
            StoreOp::Merge,
            StoreOp::Append,
            StoreOp::Incr,
            StoreOp::Delete,
        ],
        (StoreKind::Vector, AgentAccess::Read) => vec![StoreOp::Search],
        (StoreKind::Vector, AgentAccess::ReadWrite | AgentAccess::Full) => {
            vec![StoreOp::Search, StoreOp::Upsert]
        }
        (StoreKind::Blob, AgentAccess::Read) => vec![StoreOp::Get, StoreOp::List],
        (StoreKind::Blob, AgentAccess::ReadWrite | AgentAccess::Full) => {
            vec![StoreOp::Get, StoreOp::List, StoreOp::Put]
        }
    }
}

/// The name a synthesized tool takes: `<store's local name>_<op>` (grammar 11.5).
pub(crate) fn store_tool_name(local: &str, op: StoreOp) -> String {
    format!("{local}_{}", op.as_str())
}

/// The `if` argument a synthesized `kv` write offers: `absent`, `present`, or
/// the empty string — the default, meaning no condition (PRD resolved q68).
///
/// A string with a pattern rather than an enum because a tool's optional
/// argument is spelled with a `default:` (grammar 3.6, and see
/// [`store_tool_input`]) and an enum's default has to be one of its variants,
/// which "no condition" is not. The empty string is the same "nothing asked"
/// sentinel `builtin.bash`'s `command` defaults to.
pub(crate) const STORE_TOOL_CONDITION_PATTERN: &str = "^(absent|present)?$";

/// The `if_version` a synthesized `kv` write is called without: no version
/// condition. Every version a key can hold is `0` or more (PRD resolved q68).
pub(crate) const STORE_TOOL_NO_VERSION: i64 = -1;

/// The **arguments** one synthesized store tool takes (grammar 11.5, 11.4).
///
/// Grammar 11.5 fixes the tool *names* and leaves their argument schemas to
/// codegen, so this is the compiler's answer and the reasoning is worth stating
/// once: a synthesized tool is its op's own row in §11.4 with the CEL positions
/// replaced by values the model supplies. Two consequences follow, and both are
/// deliberate.
///
/// * The parameters §11.4 marks optional carry a `default:` here rather than
///   being required, because that is how this schema language spells optional at
///   a declaration surface (grammar 3.6) — so a model may leave a `filter:` out
///   and mean "no filter" rather than having to invent an empty object. The
///   conditions of PRD resolved q68 are the two whose "left out" has no value of
///   the parameter's own type to default to, so each defaults to a sentinel that
///   means "no condition": `if` to the empty string
///   ([`STORE_TOOL_CONDITION_PATTERN`]) and `if_version` to
///   [`STORE_TOOL_NO_VERSION`]. A call carrying **both** is refused by the tool
///   with a sentence saying so, and goes back to the model (Decision D119).
/// * Two parameters of the node surface are **not** offered to the model.
///   `top_k`/`limit` are, because §11.4 requires them and how many results to
///   ask for is the caller's question — but `content_type:` is not: grammar 8.8
///   puts it in the row of *literals* rather than expressions, so it is a media
///   type the author writes down, and a model choosing what media type a stored
///   object claims to be is exactly the authority `agent_access:` exists to
///   withhold. A store whose blobs need one writes a `store:` node.
///
/// One row is folded rather than copied, and the reason is a type. An
/// `append`'s `field:` and `value:` are a pair whose second half is typed by the
/// first — the item type of the array the field names — and a field map has no
/// way to say that across two of its properties. So the tool takes the pair as
/// **one** argument, `item`: an object whose properties are `V`'s array fields,
/// each typed as its item, of which the call names exactly one —
/// `{"item": {"tags": "urgent"}}`. Every property is optional and a call naming
/// none, or two, is refused by the tool (Decision D119). `merge`'s `fields` is
/// the same object over all of `V`, every property optional, for the reason a
/// `filter:` is: a merge names the fields it changes.
///
/// A `vector` store that declares no `metadata_schema:` has no metadata at all
/// (Decision D114), so its `filter:` and `metadata:` have no legal key and the
/// parameter is absent rather than an object that can only ever be empty.
pub(crate) fn store_tool_input(
    kind: StoreKind,
    op: StoreOp,
    value_schema: Option<&FieldMap>,
    metadata_schema: Option<&FieldMap>,
    span: &Span,
) -> FieldMap {
    let string = || scalar_node(ScalarKind::String, span);
    let described = |mut ty: TypeNode, text: &str| {
        ty.description = Some(Spanned::new(text.to_string(), span.clone()));
        ty
    };
    let schema = || {
        value_schema
            .cloned()
            .unwrap_or_else(|| field_map(Vec::new(), span))
    };
    let condition = || {
        let mut ty = string();
        if let TypeForm::Scalar(scalar) = &mut ty.form {
            scalar.pattern = Some(STORE_TOOL_CONDITION_PATTERN.to_string());
            scalar.default = Some(Spanned::new(Literal::String(String::new()), span.clone()));
        }
        described(
            ty,
            "Apply this write only if the key holds no value (`absent`) or only if it holds \
             one (`present`). Leave it empty for a write that applies either way. A write \
             whose condition does not hold changes nothing and answers `applied: false`.",
        )
    };
    let if_version = || {
        let mut ty = bounded_integer(STORE_TOOL_NO_VERSION, i64::MAX, span);
        if let TypeForm::Scalar(scalar) = &mut ty.form {
            scalar.maximum = None;
            scalar.default = Some(Spanned::new(
                Literal::Int(STORE_TOOL_NO_VERSION),
                span.clone(),
            ));
        }
        described(
            ty,
            "Apply this write only if the key's current `version` is this one, as a `get` or \
             an earlier write answered it. `-1`, the default, asks for no version. A write \
             whose version does not match changes nothing and answers `applied: false` with \
             the version the key holds.",
        )
    };
    let mut fields: Vec<(&str, TypeNode)> = Vec::new();
    match (kind, op) {
        (StoreKind::Kv, StoreOp::Get) | (StoreKind::Blob, StoreOp::Get) => {
            fields.push(("key", string()));
        }
        (StoreKind::Kv, StoreOp::Set) => {
            fields.push(("key", string()));
            fields.push(("value", object_node_of(schema(), span)));
            fields.push(("if", condition()));
            fields.push(("if_version", if_version()));
        }
        (StoreKind::Kv, StoreOp::Merge) => {
            fields.push(("key", string()));
            fields.push((
                "fields",
                described(
                    optional_object(schema(), span),
                    "The fields to change. Every other field of the stored value is kept, and \
                     the value that results has to be one the store's schema admits.",
                ),
            ));
            fields.push(("if", condition()));
            fields.push(("if_version", if_version()));
        }
        (StoreKind::Kv, StoreOp::Append) => {
            let items: Vec<(&str, TypeNode)> = array_fields(value_schema)
                .into_iter()
                .map(|(field, item)| (field.name.value.as_str(), item.clone()))
                .collect();
            // A `V` with no array field still has the tool (`full` grants it
            // on every `kv` store, [`store_tools`]), and its `item` an object
            // with nothing to name — so the description says so, rather than
            // inviting a call the tool can only refuse.
            let about = if items.is_empty() {
                "The stored value declares no array field, so there is nothing to append to: \
                 every call is refused. Write the whole value with the `set` tool, or change \
                 fields with the `merge` one."
            } else {
                "Name exactly one array field of the stored value, with the item to add to its \
                 end: `{\"tags\": \"urgent\"}`. A key holding nothing starts from an empty \
                 array."
            };
            fields.push(("key", string()));
            fields.push((
                "item",
                described(optional_object(field_map(items, span), span), about),
            ));
            fields.push(("unique", {
                let mut ty = scalar_node(ScalarKind::Boolean, span);
                if let TypeForm::Scalar(scalar) = &mut ty.form {
                    scalar.default = Some(Spanned::new(Literal::Bool(false), span.clone()));
                }
                described(
                    ty,
                    "Set to `true` to add the item only if the array does not already hold an \
                     equal one.",
                )
            }));
            fields.push(("if", condition()));
            fields.push(("if_version", if_version()));
        }
        (StoreKind::Kv, StoreOp::Incr) => {
            let names: Vec<&str> = numeric_fields(value_schema)
                .into_iter()
                .map(|field| field.name.value.as_str())
                .collect();
            fields.push(("key", string()));
            // An enum of `V`'s numeric fields where it declares any. One that
            // declares none still has the tool ([`store_tools`]), and an enum
            // of no variants is not a schema — `z.enum([])` is a `tsc` error
            // and a provider refuses `"enum": []` — so the field is then a
            // string the tool refuses: whatever it names, the result is one `V`
            // does not admit, and that goes back to the model (Decision D119).
            let field = if names.is_empty() {
                described(
                    string(),
                    "The stored value declares no `integer` or `number` field, so there is \
                     nothing to add to: every call is refused. Change fields with the `merge` \
                     tool instead.",
                )
            } else {
                described(
                    enum_node(&names, span),
                    "The numeric field to add to. A key holding nothing starts from `0`.",
                )
            };
            fields.push(("field", field));
            fields.push(("by", {
                let mut ty = scalar_node(ScalarKind::Number, span);
                if let TypeForm::Scalar(scalar) = &mut ty.form {
                    scalar.default = Some(Spanned::new(Literal::Int(1), span.clone()));
                }
                described(ty, "What to add; `1` by default, and negative to subtract.")
            }));
            fields.push(("if", condition()));
            fields.push(("if_version", if_version()));
        }
        (StoreKind::Kv, StoreOp::Delete) => {
            fields.push(("key", string()));
            fields.push(("if_version", if_version()));
        }
        (StoreKind::Blob, StoreOp::Put) => {
            fields.push(("key", string()));
            fields.push(("value", string()));
        }
        (StoreKind::Vector, StoreOp::Search) => {
            fields.push(("query", string()));
            fields.push(("top_k", bounded_integer(1, 100, span)));
            if let Some(metadata) = metadata_schema {
                // A filter is a **partial** match over the metadata — naming
                // every declared key would not be filtering — so every property
                // is optional and the whole object defaults to none of them.
                fields.push(("filter", partial_object(metadata.clone(), span)));
            }
        }
        (StoreKind::Vector, StoreOp::Upsert) => {
            fields.push(("key", string()));
            fields.push(("value", string()));
            if let Some(metadata) = metadata_schema {
                // The metadata a stored document carries is the store's
                // declared shape, whole: an upsert that filled in half of it
                // would leave a document no `filter:` over the rest can find.
                fields.push(("metadata", object_node_of(metadata.clone(), span)));
            }
        }
        (_, StoreOp::List) => {
            fields.push(("prefix", defaulted_string(span)));
            fields.push(("limit", bounded_integer(1, 1000, span)));
        }
        _ => {}
    }
    let mut map = field_map(fields, span);
    // A tool's arguments are an input surface, which is what makes a `default:`
    // on one of them legal (grammar 3.9).
    map.surface = Surface::Input;
    map
}

/// `{ type: object, properties: <fields> }` over an already-built field map.
fn object_node_of(properties: FieldMap, span: &Span) -> TypeNode {
    node(
        TypeForm::Object(ObjectType {
            properties,
            optional: Vec::new(),
            default: None,
        }),
        span,
    )
}

/// The same object with **every** property optional and `default: {}` — a
/// partial match rather than a value of the shape.
fn partial_object(properties: FieldMap, span: &Span) -> TypeNode {
    let optional = properties
        .fields
        .iter()
        .map(|field| Spanned::new(field.name.value.clone(), span.clone()))
        .collect();
    node(
        TypeForm::Object(ObjectType {
            properties,
            optional,
            default: Some(Spanned::new(Literal::Mapping(Vec::new()), span.clone())),
        }),
        span,
    )
}

/// The same object with every property optional and **no** default — an
/// argument the call has to supply, naming the properties it means.
fn optional_object(properties: FieldMap, span: &Span) -> TypeNode {
    let optional = properties
        .fields
        .iter()
        .map(|field| Spanned::new(field.name.value.clone(), span.clone()))
        .collect();
    node(
        TypeForm::Object(ObjectType {
            properties,
            optional,
            default: None,
        }),
        span,
    )
}

/// The **arguments** one built-in tool takes (grammar 5.5, 6.1, Decision D135,
/// PRD resolved q54).
///
/// Written here beside [`store_tool_input`] for the same reason: both are tool
/// surfaces the composition does not spell out, and both have to be one field
/// map, so that the JSON the model is asked for and the Zod the arguments are
/// parsed with are emitted from one document (PRD §9.16). *Emitted* from one:
/// what a request carries is the runtime's business, and on the one wire that
/// declares `strict` over a function's `parameters` it is that document lowered
/// to the keywords the decoder compiles (PRD §9 resolved q55, and
/// `codegen::runtime`'s divergence ledger). The parse is unchanged either way,
/// which is the whole of why one field map is the requirement here.
///
/// The parameter **names** are the ones Anthropic's text-editor and bash tools
/// carry, because on the Messages wire these go out as those provider-defined
/// tool types and a trained model fills exactly those names. Every other wire
/// declares the same names as an ordinary function tool, so one set of handlers
/// serves all of them.
///
/// What is **not** here is the bound. `workspace:`, `timeout:` and the child
/// environment belong to the binding rather than to the call — a model that
/// could name its own workspace would hold the capability the binding exists to
/// bound — so none is a parameter, and every path is read relative to the
/// workspace the binding declared.
pub(crate) fn builtin_tool_input(builtin: Builtin, span: &Span) -> FieldMap {
    let described = |description: &str| {
        let mut ty = scalar_node(ScalarKind::String, span);
        ty.description = Some(Spanned::new(description.to_string(), span.clone()));
        ty
    };
    let required = |description: &str| {
        let mut ty = described(description);
        if let TypeForm::Scalar(scalar) = &mut ty.form {
            scalar.min_length = Some(1);
        }
        ty
    };
    let optional = |description: &str| {
        let mut ty = described(description);
        if let TypeForm::Scalar(scalar) = &mut ty.form {
            scalar.default = Some(Spanned::new(Literal::String(String::new()), span.clone()));
        }
        ty
    };
    let described_enum = |variants: &[&str], description: &str| {
        let mut ty = enum_node(variants, span);
        ty.description = Some(Spanned::new(description.to_string(), span.clone()));
        ty
    };
    let boolean = |description: &str| {
        let mut ty = scalar_node(ScalarKind::Boolean, span);
        ty.description = Some(Spanned::new(description.to_string(), span.clone()));
        if let TypeForm::Scalar(scalar) = &mut ty.form {
            scalar.default = Some(Spanned::new(Literal::Bool(false), span.clone()));
        }
        ty
    };
    let fields: Vec<(&str, TypeNode)> = match builtin {
        Builtin::Bash => vec![
            // Optional, because `restart` is the one call that carries no
            // command — the provider-defined `bash` tool takes exactly this pair
            // and a trained model sends `{"restart": true}` alone. A call
            // carrying neither is refused by the tool with a sentence saying
            // which to send (Decision D119), rather than by a schema that would
            // have refused the restart too; a call carrying **both** runs the
            // command in the session the restart opened, because a command the
            // runtime dropped would be one the model was told had run.
            (
                "command",
                optional("The shell command to run, as one line of `bash`."),
            ),
            (
                "restart",
                boolean(
                    "Set to `true` to end this shell session and start a fresh one in the \
                     workspace, which is how a wedged shell is recovered. Sent alone it runs \
                     nothing; sent with a `command`, that command runs in the fresh session.",
                ),
            ),
        ],
        Builtin::Files => vec![
            (
                "command",
                described_enum(
                    FILE_COMMANDS,
                    "The file operation to perform: `view` reads a file or lists a directory, \
                     `create` writes a whole file, `str_replace` swaps one occurrence of a \
                     string, `insert` adds text at a line.",
                ),
            ),
            (
                "path",
                required("The file or directory, relative to this tool's workspace."),
            ),
            // The one argument that is here because the *provider-defined* tool
            // has it. `view` of a long file is read in windows, and a model
            // trained on `text_editor_20250728` reaches for this on the second
            // read of one; declaring a narrower `view` would spend a turn of
            // `max_tool_iterations` on a bounce every time it did, and leave the
            // tail of a file past the runtime's answer bound reachable only
            // through `bash` — which an agent holding this tool alone does not
            // have.
            ("view_range", {
                let mut ty = array_node(bounded_integer(-1, 1_000_000, span), Some(2), span);
                ty.description = Some(Spanned::new(
                    "The first and last line to show, for `view` of a file: `[10, 40]`. \
                     Lines count from 1 and both ends are included; `-1` as the last line \
                     reads to the end of the file. Omitted, the whole file is shown."
                        .to_string(),
                    span.clone(),
                ));
                // Defaulted to the empty range for `insert_line`'s reason — the
                // three commands that never read it stay callable without it —
                // and *empty* rather than `[1, -1]` so the default is the whole
                // file however long the file turns out to be.
                if let TypeForm::Array(array) = &mut ty.form {
                    array.default = Some(Spanned::new(Literal::Sequence(Vec::new()), span.clone()));
                }
                ty
            }),
            (
                "file_text",
                optional("The whole contents of the file, for `create`."),
            ),
            (
                "old_str",
                optional(
                    "The exact text to replace, for `str_replace`. It must appear exactly once.",
                ),
            ),
            (
                "new_str",
                optional("The text to put in its place, for `str_replace` and `insert`."),
            ),
            ("insert_line", {
                let mut ty = bounded_integer(0, 1_000_000, span);
                ty.description = Some(Spanned::new(
                    "The line to insert after, for `insert`; `0` inserts at the top of the file."
                        .to_string(),
                    span.clone(),
                ));
                // Defaulted, so the three commands that are not `insert` are
                // callable without it — a required parameter only one operation
                // reads is one a model has to guess at on every other call.
                if let TypeForm::Scalar(scalar) = &mut ty.form {
                    scalar.default = Some(Spanned::new(Literal::Int(0), span.clone()));
                }
                ty
            }),
        ],
    };
    let mut map = field_map(fields, span);
    // A tool's arguments are an input surface, which is what makes a `default:`
    // on one of them legal (grammar 3.9).
    map.surface = Surface::Input;
    map
}

/// The `command:` a `builtin.files` call names, in the order grammar 6.1 lists
/// them — the text-editor operations this runtime implements.
pub(crate) const FILE_COMMANDS: &[&str] = &["view", "create", "str_replace", "insert"];

/// What a built-in tells the model it does, where the composition wrote no
/// `description:` of its own.
///
/// The compiler's own text, because the contract is the compiler's: a shorthand
/// entry has nowhere to write one, and a `builtin:` binding declares no
/// `input:`/`output:` for a description to describe. Each says the thing a model
/// has to know to call it correctly — that paths are relative to a workspace it
/// cannot see and cannot leave, and that a command is bounded by a deadline —
/// because that is the difference between a model correcting itself and a model
/// spending the loop's budget guessing (PRD G3, Decision D119).
pub(crate) fn builtin_description(builtin: Builtin) -> String {
    match builtin {
        Builtin::Bash => "Run a `bash` command in a persistent shell session and return what it \
             printed, with its exit status. The working directory and any shell state carry over \
             from one call to the next, and each command runs under a deadline."
            .to_string(),
        Builtin::Files => "View, create and edit files inside this agent's workspace. Every path \
             is relative to that workspace, and a path that resolves outside it is refused."
            .to_string(),
    }
}

/// `{ type: string, default: "" }` — an optional string parameter.
fn defaulted_string(span: &Span) -> TypeNode {
    let mut ty = scalar_node(ScalarKind::String, span);
    if let TypeForm::Scalar(scalar) = &mut ty.form {
        scalar.default = Some(Spanned::new(Literal::String(String::new()), span.clone()));
    }
    ty
}

/// `{ type: integer, minimum: <low>, maximum: <high> }`.
fn bounded_integer(low: i64, high: i64, span: &Span) -> TypeNode {
    let mut ty = scalar_node(ScalarKind::Integer, span);
    if let TypeForm::Scalar(scalar) = &mut ty.form {
        scalar.minimum = Some(crate::ast::schema::Number::Int(low));
        scalar.maximum = Some(crate::ast::schema::Number::Int(high));
    }
    ty
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

    /// The boundary value is the whole difference between the two spellings, so
    /// the relation has to read it: a source that admits it does not satisfy a
    /// destination that excludes it, at either end (grammar 3.3, Decision
    /// D111).
    #[test]
    fn an_exclusive_bound_is_not_satisfied_by_one_that_admits_the_boundary() {
        let bounded = |minimum: Option<i64>,
                       exclusive_minimum: Option<i64>,
                       maximum: Option<i64>,
                       exclusive_maximum: Option<i64>| {
            let mut node = scalar_node(ScalarKind::Integer, &span());
            if let TypeForm::Scalar(scalar) = &mut node.form {
                scalar.minimum = minimum.map(Number::Int);
                scalar.exclusive_minimum = exclusive_minimum.map(Number::Int);
                scalar.maximum = maximum.map(Number::Int);
                scalar.exclusive_maximum = exclusive_maximum.map(Number::Int);
            }
            node
        };
        let at_zero = bounded(Some(0), None, None, None);
        let above_zero = bounded(None, Some(0), None, None);
        let mismatch = satisfies(&at_zero, &above_zero).unwrap_err();
        assert_eq!(
            mismatch.describe(),
            "expected a number above 0, found one that admits 0"
        );
        // The same number, said the same way, still satisfies itself — and a
        // bound that clears the excluded value satisfies it too.
        assert!(satisfies(&above_zero, &above_zero).is_ok());
        assert!(satisfies(&above_zero, &at_zero).is_ok());
        assert!(satisfies(&bounded(Some(1), None, None, None), &above_zero).is_ok());

        let to_ten = bounded(None, None, Some(10), None);
        let below_ten = bounded(None, None, None, Some(10));
        let mismatch = satisfies(&to_ten, &below_ten).unwrap_err();
        assert_eq!(
            mismatch.describe(),
            "expected a number below 10, found one that admits 10"
        );
        assert!(satisfies(&below_ten, &to_ten).is_ok());
        assert!(satisfies(&bounded(None, None, Some(9), None), &below_ten).is_ok());

        // Where a scalar declares both spellings, the range it admits is the
        // tighter of the two, so it is the tighter one that must be satisfied.
        let both = bounded(Some(0), Some(1), None, None);
        assert!(satisfies(&both, &bounded(None, Some(1), None, None)).is_ok());
        assert!(satisfies(&bounded(Some(1), None, None, None), &both).is_err());
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

    /// The relation the `default:` narrowing needs: two spellings of one type,
    /// written in two places, are the same type — which `==` cannot say,
    /// because every node carries the span it was written at.
    #[test]
    fn identity_is_about_the_type_and_not_about_the_declaration() {
        let here = span();
        let there = Span::new(
            SourceName::new("other.yml"),
            40..41,
            Position::new(9, 3),
            Position::new(9, 4),
        );
        let left = object_node(
            vec![("summary", scalar_node(ScalarKind::String, &here))],
            &here,
        );
        let right = object_node(
            vec![("summary", scalar_node(ScalarKind::String, &there))],
            &there,
        );
        assert_ne!(left, right, "the two declarations are two source regions");
        assert!(identical(&left, &right));
        // …and identity still reads every constraint, every name, and the
        // order they were declared in.
        assert!(!identical(&string_with(Some(1)), &string_with(None)));
        assert!(!identical(
            &object_node(vec![("a", string_with(None))], &here),
            &object_node(vec![("b", string_with(None))], &here)
        ));
        assert!(!identical(
            &object_node(
                vec![("a", string_with(None)), ("b", string_with(None))],
                &here
            ),
            &object_node(
                vec![("b", string_with(None)), ("a", string_with(None))],
                &here
            )
        ));
        assert!(!identical(
            &array_node(scalar_node(ScalarKind::String, &here), Some(5), &here),
            &array_node(scalar_node(ScalarKind::String, &here), Some(6), &here)
        ));
        assert!(!identical(
            &scalar_node(ScalarKind::Integer, &here),
            &scalar_node(ScalarKind::Number, &here)
        ));
    }

    /// A `default:` is part of what a declaration says, and its literal carries
    /// spans of its own — so it needs the same span-free reading.
    #[test]
    fn identity_reads_a_default_by_its_value() {
        let here = span();
        let there = Span::new(
            SourceName::new("other.yml"),
            40..41,
            Position::new(9, 3),
            Position::new(9, 4),
        );
        let with = |value: &str, at: &Span| {
            let mut node = scalar_node(ScalarKind::String, at);
            if let TypeForm::Scalar(scalar) = &mut node.form {
                scalar.default = Some(Spanned::new(Literal::String(value.to_string()), at.clone()));
            }
            node
        };
        assert!(identical(&with("x", &here), &with("x", &there)));
        assert!(!identical(&with("x", &here), &with("y", &here)));
        assert!(!identical(
            &with("x", &here),
            &scalar_node(ScalarKind::String, &here)
        ));
    }

    /// Grammar 11.5's table read through `agent_access:`: `full` adds a `kv`
    /// store's partial updates and `delete` — all four, whatever `V` declares,
    /// which is the set PRD resolved q68 names — and `read_write` keeps exactly
    /// the two tools it always had. `full` on a `vector` or `blob` store is a
    /// parse error (`invalid-parse/store-blob-with-full-agent-access.yml`), so
    /// all this asserts of those arms is that they grant `read_write`'s set and
    /// nothing past it.
    #[test]
    fn full_access_adds_the_partial_updates_and_delete_and_read_write_does_not() {
        assert_eq!(
            store_tools(StoreKind::Kv, AgentAccess::Full),
            [
                StoreOp::Get,
                StoreOp::Set,
                StoreOp::Merge,
                StoreOp::Append,
                StoreOp::Incr,
                StoreOp::Delete
            ]
        );
        assert_eq!(
            store_tools(StoreKind::Kv, AgentAccess::ReadWrite),
            [StoreOp::Get, StoreOp::Set],
            "an existing composition keeps exactly the tools it had"
        );
        assert_eq!(
            store_tools(StoreKind::Kv, AgentAccess::Read),
            [StoreOp::Get]
        );
        assert_eq!(
            store_tools(StoreKind::Vector, AgentAccess::Full),
            store_tools(StoreKind::Vector, AgentAccess::ReadWrite),
            "an IR that skipped the parser's refusal is handed no write it did not ask for"
        );
        assert_eq!(
            store_tools(StoreKind::Blob, AgentAccess::Full),
            store_tools(StoreKind::Blob, AgentAccess::ReadWrite)
        );
    }

    /// Which kind takes which op is grammar 11.4's table, row by row — the
    /// partial updates are `kv`'s alone.
    #[test]
    fn the_partial_updates_are_ops_of_a_kv_store_alone() {
        for op in [StoreOp::Merge, StoreOp::Append, StoreOp::Incr] {
            assert!(kind_takes(StoreKind::Kv, op), "{}", op.as_str());
            assert!(!kind_takes(StoreKind::Vector, op), "{}", op.as_str());
            assert!(!kind_takes(StoreKind::Blob, op), "{}", op.as_str());
            assert!(op.writes(), "{} is a write", op.as_str());
        }
        assert!(!kind_takes(StoreKind::Kv, StoreOp::Search));
        assert!(kind_takes(StoreKind::Vector, StoreOp::Delete));
    }

    /// The agent tool surface is untouched by PRD resolved q70: a model is
    /// offered exactly the tools it was offered before the reads at scale
    /// existed, at every access level and on every kind — exposing `count`,
    /// `get_many` or a `kv` page to a model is the parity item that entry
    /// deferred, and this is what notices one arriving unannounced.
    #[test]
    fn the_reads_at_scale_add_no_tool_at_any_access_level() {
        let expected: [(StoreKind, AgentAccess, &[StoreOp]); 9] = [
            (StoreKind::Kv, AgentAccess::Read, &[StoreOp::Get]),
            (
                StoreKind::Kv,
                AgentAccess::ReadWrite,
                &[StoreOp::Get, StoreOp::Set],
            ),
            (
                StoreKind::Kv,
                AgentAccess::Full,
                &[
                    StoreOp::Get,
                    StoreOp::Set,
                    StoreOp::Merge,
                    StoreOp::Append,
                    StoreOp::Incr,
                    StoreOp::Delete,
                ],
            ),
            (StoreKind::Vector, AgentAccess::Read, &[StoreOp::Search]),
            (
                StoreKind::Vector,
                AgentAccess::ReadWrite,
                &[StoreOp::Search, StoreOp::Upsert],
            ),
            (
                StoreKind::Vector,
                AgentAccess::Full,
                &[StoreOp::Search, StoreOp::Upsert],
            ),
            (
                StoreKind::Blob,
                AgentAccess::Read,
                &[StoreOp::Get, StoreOp::List],
            ),
            (
                StoreKind::Blob,
                AgentAccess::ReadWrite,
                &[StoreOp::Get, StoreOp::List, StoreOp::Put],
            ),
            (
                StoreKind::Blob,
                AgentAccess::Full,
                &[StoreOp::Get, StoreOp::List, StoreOp::Put],
            ),
        ];
        for (kind, access, tools) in expected {
            let synthesized = store_tools(kind, access);
            assert_eq!(synthesized, tools, "{} at {access:?}", kind.as_str());
            assert!(
                !synthesized.contains(&StoreOp::Count) && !synthesized.contains(&StoreOp::GetMany),
                "no access level offers PRD resolved q70's reads to a model"
            );
        }
        assert!(
            !store_tools(StoreKind::Kv, AgentAccess::Full).contains(&StoreOp::List),
            "nor a `kv` page"
        );
    }

    /// `count` and `get_many` are `kv` ops and reads: recorded and replayed,
    /// carrying no idempotency key (PRD resolved q70, q29).
    #[test]
    fn count_and_get_many_are_reads_of_a_kv_store_alone() {
        for op in [StoreOp::Count, StoreOp::GetMany] {
            assert!(kind_takes(StoreKind::Kv, op), "{}", op.as_str());
            assert!(!kind_takes(StoreKind::Vector, op), "{}", op.as_str());
            assert!(!kind_takes(StoreKind::Blob, op), "{}", op.as_str());
            assert!(!op.writes(), "{} is a read", op.as_str());
        }
        assert!(!StoreOp::List.writes());
        assert_eq!(StoreOp::Count.required_parameters(), &[] as &[&str]);
        assert_eq!(StoreOp::Count.optional_parameters(), &["prefix"]);
        assert_eq!(StoreOp::GetMany.required_parameters(), &["keys"]);
        assert_eq!(
            StoreOp::List.optional_parameters(),
            &["prefix", "after", "include_values"]
        );
    }

    /// A `batch` is offered to no agent, at any access level and on any kind
    /// (PRD resolved q71): it is a store-op node's alone. Every level's set is
    /// otherwise the one it was, so a `batch` arriving in one would be this
    /// test failing rather than a model quietly handed a hundred writes.
    #[test]
    fn a_batch_is_offered_to_no_agent_at_any_access_level() {
        for kind in StoreKind::ALL {
            for access in [AgentAccess::Read, AgentAccess::ReadWrite, AgentAccess::Full] {
                assert!(
                    !store_tools(*kind, access).contains(&StoreOp::Batch),
                    "`{}` at {access:?} synthesizes a `batch` tool",
                    kind.as_str()
                );
            }
        }
        assert_eq!(
            store_tools(StoreKind::Kv, AgentAccess::Full),
            [
                StoreOp::Get,
                StoreOp::Set,
                StoreOp::Merge,
                StoreOp::Append,
                StoreOp::Incr,
                StoreOp::Delete
            ],
            "`full` keeps the six tools it grants"
        );
    }

    /// A `batch` is a `kv` op and a write — one effect carrying the node's
    /// one idempotency key — whose row is `ops:` and, beside it, the authored
    /// key that replaces the derived one (PRD resolved q71, q73).
    #[test]
    fn a_batch_is_a_kv_write_that_takes_ops_and_an_authored_key() {
        assert!(kind_takes(StoreKind::Kv, StoreOp::Batch));
        assert!(!kind_takes(StoreKind::Vector, StoreOp::Batch));
        assert!(!kind_takes(StoreKind::Blob, StoreOp::Batch));
        assert!(StoreOp::Batch.writes());
        assert_eq!(StoreOp::Batch.required_parameters(), &["ops"]);
        assert_eq!(StoreOp::Batch.optional_parameters(), &["idempotency_key"]);
        assert_eq!(StoreOp::Batch.as_str(), "batch");
        assert!(StoreOp::ALL.contains(&StoreOp::Batch));
    }

    /// `idempotency_key:` is in exactly the rows of the six `kv` writes — `set`,
    /// `delete`, `merge`, `append`, `incr` and `batch` — and in no read's, nor
    /// in a `vector` `upsert`'s or a `blob` `put`'s (PRD resolved q73). The
    /// row is what the parser refuses a parameter outside of, so this is the
    /// whole of the rule but the `delete` of another kind, which only the
    /// store answers.
    #[test]
    fn an_authored_key_is_in_exactly_the_six_kv_write_rows() {
        let carrying: Vec<StoreOp> = StoreOp::ALL
            .iter()
            .copied()
            .filter(|op| op.optional_parameters().contains(&"idempotency_key"))
            .collect();
        assert_eq!(
            carrying,
            [
                StoreOp::Set,
                StoreOp::Delete,
                StoreOp::Merge,
                StoreOp::Append,
                StoreOp::Incr,
                StoreOp::Batch
            ]
        );
        for op in carrying {
            assert!(
                op.writes(),
                "`{}` carries a key and is a write",
                op.as_str()
            );
            assert!(
                kind_takes(StoreKind::Kv, op),
                "`{}` is a `kv` op",
                op.as_str()
            );
            assert!(
                !op.required_parameters().contains(&"idempotency_key"),
                "`{}` requires no authored key",
                op.as_str()
            );
        }
    }

    /// A `batch`'s derived row: `applied`, `deduped`, one `{ key, version }`
    /// per entry bounded by the node's entry count, and `failed` — every field
    /// carried, the two the ruling marks optional (`failed`, a result's
    /// `version`) included, under D110's posture: a store row is never parsed
    /// against it.
    #[test]
    fn a_batch_derives_applied_deduped_results_and_failed() {
        let value = field_map(
            vec![("body", scalar_node(ScalarKind::String, &span()))],
            &span(),
        );
        let derive = |bound| {
            store_output(
                StoreKind::Kv,
                StoreOp::Batch,
                Some(&value),
                None,
                bound,
                false,
                &span(),
            )
        };
        let row = derive(Some(3));
        assert_eq!(names(&row), ["applied", "deduped", "results", "failed"]);
        assert_eq!(
            items(&row, "results"),
            (vec!["key".into(), "version".into()], Some(3)),
            "one result per entry, bounded by the entries the node writes"
        );
        let kind_of = |name: &str| {
            row.fields
                .iter()
                .find(|field| field.name.value.as_str() == name)
                .map(|field| match &field.ty.form {
                    TypeForm::Scalar(scalar) => Some(scalar.kind),
                    _ => None,
                })
        };
        assert_eq!(kind_of("applied"), Some(Some(ScalarKind::Boolean)));
        assert_eq!(kind_of("deduped"), Some(Some(ScalarKind::Boolean)));
        assert_eq!(kind_of("failed"), Some(Some(ScalarKind::Integer)));
        assert_eq!(
            items(&derive(None), "results").1,
            i64::try_from(BATCH_MAX_ENTRIES).ok(),
            "with no entry count to read, the bound is the most one batch holds"
        );
        assert!(
            names(&store_output(
                StoreKind::Blob,
                StoreOp::Batch,
                None,
                None,
                Some(3),
                false,
                &span()
            ))
            .is_empty(),
            "a `blob` store takes no batch, so there is no row to derive"
        );
    }

    /// The names of a derived row's fields, in order — and of an array
    /// field's items, where the row's first field is one.
    fn names(fields: &FieldMap) -> Vec<String> {
        fields
            .fields
            .iter()
            .map(|field| field.name.value.to_string())
            .collect()
    }

    /// The item fields of one array field of a derived row, and its bound.
    fn items(fields: &FieldMap, name: &str) -> (Vec<String>, Option<i64>) {
        let field = fields
            .fields
            .iter()
            .find(|field| field.name.value.as_str() == name)
            .unwrap_or_else(|| panic!("no `{name}` in {:?}", names(fields)));
        let TypeForm::Array(array) = &field.ty.form else {
            panic!("`{name}` is an array");
        };
        let item = match &array.items.form {
            TypeForm::Object(object) => names(&object.properties),
            _ => Vec::new(),
        };
        (item, array.max_items)
    }

    /// A `kv` `list`'s row: `keys` and `next` always, and `entries` exactly
    /// when the node's `include_values:` literal is `true` — presence decided
    /// by a literal in the node, Decision D114's pattern (D154). A `blob`
    /// `list` keeps its one field whatever it is asked.
    #[test]
    fn a_kv_page_derives_entries_exactly_when_include_values_is_true() {
        let value = field_map(
            vec![("body", scalar_node(ScalarKind::String, &span()))],
            &span(),
        );
        let page = |include| {
            store_output(
                StoreKind::Kv,
                StoreOp::List,
                Some(&value),
                None,
                Some(25),
                include,
                &span(),
            )
        };
        assert_eq!(names(&page(false)), ["keys", "next"]);
        let valued = page(true);
        assert_eq!(names(&valued), ["keys", "next", "entries"]);
        assert_eq!(
            items(&valued, "entries"),
            (
                vec!["key".into(), "value".into(), "version".into()],
                Some(25)
            ),
            "an entry is a key, its value and its version, bounded like the page"
        );
        assert_eq!(items(&valued, "keys").1, Some(25));
        assert_eq!(
            names(&store_output(
                StoreKind::Blob,
                StoreOp::List,
                None,
                None,
                Some(25),
                true,
                &span()
            )),
            ["keys"]
        );
    }

    /// A `count` answers one integer; a `get_many` one entry per key asked,
    /// bounded by the most one reads rather than by anything the node says.
    #[test]
    fn a_count_and_a_get_many_derive_their_rows() {
        let value = field_map(
            vec![("body", scalar_node(ScalarKind::String, &span()))],
            &span(),
        );
        let counted = store_output(
            StoreKind::Kv,
            StoreOp::Count,
            Some(&value),
            None,
            None,
            false,
            &span(),
        );
        assert_eq!(names(&counted), ["count"]);
        assert!(matches!(
            &counted.fields[0].ty.form,
            TypeForm::Scalar(Scalar {
                kind: ScalarKind::Integer,
                ..
            })
        ));
        let many = store_output(
            StoreKind::Kv,
            StoreOp::GetMany,
            Some(&value),
            None,
            None,
            false,
            &span(),
        );
        assert_eq!(names(&many), ["entries"]);
        assert_eq!(
            items(&many, "entries"),
            (
                vec![
                    "key".into(),
                    "value".into(),
                    "found".into(),
                    "version".into()
                ],
                Some(GET_MANY_MAX_KEYS)
            )
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
            false,
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
