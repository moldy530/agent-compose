//! The schema language (grammar 3): field maps and type nodes.
//!
//! Which surface a schema sits at decides three of its rules, so [`Surface`] is
//! threaded through every level: `default:` is illegal in result schemas
//! (grammar 3.6), `max_items` is required on their arrays (grammar 3.5 clause
//! 1, Decision D10), and a state channel additionally accepts `reduce:`
//! (grammar 10.1). The second clause of the `max_items` rule — an array a
//! `map.over` path resolves to — needs the resolved composition, so the
//! validator owns it.

use crate::ast::common::{Ident, Literal, LiteralEntry};
use crate::ast::schema::{
    ArrayType, EnumType, Field, FieldMap, Number, ObjectType, ScalarKind, ScalarType, StringFormat,
    Surface, TypeForm, TypeNode, UnionType, UnionVariant,
};
use crate::diag::{Diagnostic, DiagnosticCode, Span, Spanned};
use crate::yaml::{Node, Yaml};

use super::lexical;
use super::reader::{
    Cx, Fields, at_least, expect_finite, expect_mapping, expect_sequence, expect_string, in_range,
    list, suggest,
};

/// The maximum schema nesting depth; the declaration surface counts as one
/// level (grammar 3.4).
///
/// Every way of nesting counts the same level: an object's `properties`, a
/// union variant's payload, and an array's `items`. Grammar 3.4 states the
/// limit under "Objects", but a chain of arrays nests exactly as far, and
/// bounding only the field maps would leave `items: {type: array, items: …}`
/// bounded by nothing but the YAML loader's collection-depth cap.
const MAX_DEPTH: usize = 8;

/// The scalar type keywords.
const SCALAR_KINDS: &[(&str, ScalarKind)] = &[
    ("string", ScalarKind::String),
    ("integer", ScalarKind::Integer),
    ("number", ScalarKind::Number),
    ("boolean", ScalarKind::Boolean),
];

const FORMATS: &[(&str, StringFormat)] = &[
    ("date-time", StringFormat::DateTime),
    ("date", StringFormat::Date),
    ("time", StringFormat::Time),
    ("duration", StringFormat::Duration),
    ("email", StringFormat::Email),
    ("uri", StringFormat::Uri),
    ("uuid", StringFormat::Uuid),
    ("hostname", StringFormat::Hostname),
    ("ipv4", StringFormat::Ipv4),
    ("ipv6", StringFormat::Ipv6),
];

/// The type-node keys that select a form (grammar 3.2).
const DISCRIMINATING_KEYS: &[&str] = &["type", "enum", "discriminator"];

/// Read a declaration surface: `output:`, `input:`, `properties:`, … (grammar
/// 3.1).
///
/// `None` means *this is not a field map the author wrote* — the value is not a
/// mapping, it is a discriminated union, or every entry in it was rejected — as
/// opposed to `Some` of an empty one, which is the author writing `{}`. Callers
/// depend on the difference: the surfaces that require at least one property
/// (`agent.output`, `agent.input`, grammar 5.1 and 5.3) ask that question of the
/// map they get back, and asking it of a synthesized empty one would advise an
/// author to add a property to a surface whose mistake was its shape — a second
/// diagnostic for one mistake.
pub(crate) fn field_map(
    node: &Node,
    subject: &str,
    surface: Surface,
    cx: &mut Cx,
) -> Option<FieldMap> {
    field_map_at(node, subject, surface, 1, cx)
}

/// Whether this level is past [`MAX_DEPTH`], reporting it if so.
fn too_deep(node: &Node, subject: &str, depth: usize, cx: &mut Cx) -> bool {
    if depth <= MAX_DEPTH {
        return false;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            node.span.clone(),
            format!("{subject} nests more than {MAX_DEPTH} levels deep"),
        )
        .with_help(
            "the declaration surface counts as the first level, and an object's `properties`, a union variant, and an array's `items` each count one more (grammar 3.4)",
        ),
    );
    true
}

fn field_map_at(
    node: &Node,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> Option<FieldMap> {
    let mapping = expect_mapping(node, subject, cx)?;
    if too_deep(node, subject, depth, cx) {
        return Some(FieldMap {
            fields: Vec::new(),
            surface,
            span: node.span.clone(),
        });
    }

    if is_union_shape(mapping) {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("{subject} is a discriminated union, but a field map belongs here"),
            )
            .with_help(
                "a union is legal as an array's `items:` or as the type of a named property, never as a whole surface: every declaration surface is a field map, so routing fields have names (grammar 3.7, Decision D11)",
            ),
        );
        // Not an empty field map — not a field map at all. Handing one back
        // would let `agent.output`'s ≥1-property rule fire on top of this, and
        // "must declare at least one property" is wrong-headed advice for a
        // surface whose mistake is that it is a union.
        return None;
    }

    let mut fields = Vec::new();
    for entry in mapping.entries() {
        let Some(name) = lexical::key_identifier(&entry.key, "field name", cx) else {
            continue;
        };
        let ty = type_node(
            &entry.value,
            &format!("the schema of `{}`", name.value),
            surface,
            depth,
            cx,
        );
        fields.push(Field { name, ty });
    }
    // A map whose every field was rejected — by the loader (grammar 1.1) or by
    // the field-name rule above — is not the empty map either: the author
    // declared fields, and each one that did not read has its own diagnostic
    // already. Only a mapping that really is `{}` reaches a caller as an empty
    // [`FieldMap`].
    if fields.is_empty() && !mapping.declares_nothing() {
        return None;
    }
    Some(FieldMap {
        fields,
        surface,
        span: node.span.clone(),
    })
}

/// Whether this mapping is a discriminated union rather than a field map.
///
/// The two are told apart by what `discriminator:` holds: a union names a field
/// with a string, while a field *named* `discriminator` carries a type node,
/// which is a mapping (grammar 3.1, 3.7). Both keys have to be present, so a
/// field map that merely mistypes a field called `discriminator` still gets the
/// diagnostic about that field.
fn is_union_shape(mapping: &crate::yaml::Mapping) -> bool {
    mapping.contains_key("variants")
        && mapping
            .get("discriminator")
            .is_some_and(|node| node.as_mapping().is_none())
}

/// The surface nested schemas sit at.
///
/// A state channel's own keys are channel keys, but the type nodes inside it —
/// an array's `items`, an object's `properties` — are ordinary input-surface
/// nodes.
const fn nested(surface: Surface) -> Surface {
    match surface {
        Surface::Channel => Surface::Input,
        other => other,
    }
}

/// Read a type node (grammar 3.2).
///
/// Always yields a node: a malformed one carries [`TypeForm::Invalid`] after
/// its diagnostic, so the surrounding field map still parses.
pub(crate) fn type_node(
    node: &Node,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> TypeNode {
    let Some(mapping) = node.as_mapping() else {
        cx.wrong_type(node, subject, "a type node (a mapping)");
        return TypeNode {
            form: TypeForm::Invalid,
            description: None,
            span: node.span.clone(),
        };
    };
    // A field map checks its own level, so this only ever fires for a chain of
    // arrays, whose `items` reach no field map to be checked by.
    if too_deep(node, subject, depth, cx) {
        return TypeNode {
            form: TypeForm::Invalid,
            description: None,
            span: node.span.clone(),
        };
    }
    let mut fields = Fields::new(mapping, node.span.clone(), subject);
    let (form, description) = type_body(&mut fields, subject, surface, depth, cx);
    fields.finish(cx);
    TypeNode {
        form,
        description,
        span: node.span.clone(),
    }
}

/// Read a type node's body from an already-opened mapping, so that a state
/// channel can consume its `reduce:` key from the same mapping (grammar 10.1).
pub(crate) fn type_body(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> (TypeForm, Option<Spanned<String>>) {
    let description = fields
        .take("description")
        .and_then(|node| lexical::text(node, &format!("`description` in {subject}"), cx));

    let declared: Vec<&str> = DISCRIMINATING_KEYS
        .iter()
        .copied()
        .filter(|key| fields.contains(key))
        .collect();

    let form = match declared.as_slice() {
        [] => {
            fields.note_known(DISCRIMINATING_KEYS);
            // Whatever else the mapping holds, it is unreadable without a form:
            // one diagnostic about the missing type beats one per stray key.
            fields.consume_rest();
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::MissingKey,
                    fields.span.clone(),
                    format!(
                        "{subject} declares no type: a type node carries exactly one of {}",
                        list(DISCRIMINATING_KEYS)
                    ),
                )
                .with_help(
                    "nested objects are written out with `type: object` and `properties:`; a bare mapping is not a shorthand for one",
                ),
            );
            TypeForm::Invalid
        }
        ["type"] => scalar_object_or_array(fields, subject, surface, depth, cx),
        ["enum"] => enum_form(fields, subject, surface, cx),
        ["discriminator"] => union_form(fields, subject, surface, depth, cx),
        _ => {
            // Consume every discriminating key but the first: the conflict is
            // reported once, and the leftovers must not resurface as unknown
            // keys.
            let mut span = None;
            for key in &declared[1..] {
                let entry = fields.take_entry(key);
                span = span.or_else(|| entry.map(|entry| entry.key.span.clone()));
            }
            let span = span.unwrap_or_else(|| fields.span.clone());
            let message = if declared.contains(&"enum") && declared.contains(&"type") {
                "`enum` implies `type: string`; writing `type:` alongside it is an error".to_owned()
            } else {
                format!(
                    "{subject} declares {}; a type node carries exactly one of {}",
                    list(&declared),
                    list(DISCRIMINATING_KEYS)
                )
            };
            cx.error(DiagnosticCode::ConflictingKeys, &span, message);
            // Parse the first declared form anyway, so the field keeps a shape.
            match declared[0] {
                "type" => scalar_object_or_array(fields, subject, surface, depth, cx),
                "enum" => enum_form(fields, subject, surface, cx),
                _ => union_form(fields, subject, surface, depth, cx),
            }
        }
    };
    (form, description)
}

fn scalar_object_or_array(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> TypeForm {
    let Some(node) = fields.take("type") else {
        return TypeForm::Invalid;
    };
    let Some(keyword) = expect_string(node, &format!("`type` in {subject}"), cx) else {
        return TypeForm::Invalid;
    };
    match keyword.value.as_str() {
        "object" => object_form(fields, subject, surface, depth, cx),
        "array" => array_form(fields, subject, surface, depth, cx),
        _ => match SCALAR_KINDS.iter().find(|(name, _)| *name == keyword.value) {
            Some((_, kind)) => scalar_form(
                fields,
                subject,
                surface,
                Spanned::new(*kind, keyword.span),
                cx,
            ),
            None => {
                let names: Vec<&str> = SCALAR_KINDS
                    .iter()
                    .map(|(name, _)| *name)
                    .chain(["object", "array"])
                    .collect();
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::UnknownVariant,
                        keyword.span,
                        format!("`{}` is not a type in {subject}", keyword.value),
                    )
                    .with_optional_help(
                        suggest(&keyword.value, &names)
                            .map(|name| format!("did you mean `{name}`?"))
                            .or_else(|| Some(format!("`type:` takes one of {}", list(&names)))),
                    ),
                );
                TypeForm::Invalid
            }
        },
    }
}

fn scalar_form(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    kind: Spanned<ScalarKind>,
    cx: &mut Cx,
) -> TypeForm {
    let mut scalar = ScalarType {
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
    };
    let kind = scalar.kind.value;

    for key in ["min_length", "max_length"] {
        let Some(node) = fields.take(key) else {
            continue;
        };
        if !constraint_applies(
            key,
            kind,
            "string",
            ScalarKind::is_string,
            &node.span,
            subject,
            cx,
        ) {
            continue;
        }
        let Some(value) = super::reader::expect_integer(node, &format!("`{key}`"), cx) else {
            continue;
        };
        if !at_least(&value, &format!("`{key}`"), 0, cx) {
            continue;
        }
        if key == "min_length" {
            scalar.min_length = Some(value);
        } else {
            scalar.max_length = Some(value);
        }
    }

    if let Some(node) = fields.take("pattern")
        && constraint_applies(
            "pattern",
            kind,
            "string",
            ScalarKind::is_string,
            &node.span,
            subject,
            cx,
        )
    {
        // RE2 syntax is checked by the validator, which owns the regex engine.
        scalar.pattern = lexical::text(node, &format!("`pattern` in {subject}"), cx);
    }

    if let Some(node) = fields.take("format")
        && constraint_applies(
            "format",
            kind,
            "string",
            ScalarKind::is_string,
            &node.span,
            subject,
            cx,
        )
    {
        scalar.format = lexical::keyword(node, "`format`", FORMATS, cx);
    }

    for key in [
        "minimum",
        "maximum",
        "exclusive_minimum",
        "exclusive_maximum",
        "multiple_of",
    ] {
        let Some(node) = fields.take(key) else {
            continue;
        };
        if !constraint_applies(
            key,
            kind,
            "integer` or `number",
            ScalarKind::is_numeric,
            &node.span,
            subject,
            cx,
        ) {
            continue;
        }
        let Some(value) = number(node, &format!("`{key}`"), cx) else {
            continue;
        };
        if key == "multiple_of" && !is_positive(&value.value) {
            cx.error(
                DiagnosticCode::ValueOutOfRange,
                &value.span,
                "`multiple_of` must be greater than 0",
            );
            continue;
        }
        match key {
            "minimum" => scalar.minimum = Some(value),
            "maximum" => scalar.maximum = Some(value),
            "exclusive_minimum" => scalar.exclusive_minimum = Some(value),
            "exclusive_maximum" => scalar.exclusive_maximum = Some(value),
            _ => scalar.multiple_of = Some(value),
        }
    }

    scalar.default = default_value(fields, subject, surface, cx)
        .inspect(|value| check_default_kind(value, kind, subject, cx));

    TypeForm::Scalar(scalar)
}

fn constraint_applies(
    key: &str,
    kind: ScalarKind,
    expected: &str,
    predicate: fn(ScalarKind) -> bool,
    span: &Span,
    subject: &str,
    cx: &mut Cx,
) -> bool {
    if predicate(kind) {
        return true;
    }
    cx.error(
        DiagnosticCode::InvalidValue,
        span,
        format!(
            "`{key}` applies to `type: {expected}`, but {subject} declares `type: {}`",
            kind.as_str()
        ),
    );
    false
}

fn is_positive(value: &Number) -> bool {
    match value {
        Number::Int(v) => *v > 0,
        Number::Float(v) => *v > 0.0,
    }
}

fn number(node: &Node, subject: &str, cx: &mut Cx) -> Option<Spanned<Number>> {
    match &node.value {
        Yaml::Int(value) => Some(Spanned::new(Number::Int(*value), node.span.clone())),
        Yaml::Float(value) => expect_finite(*value, subject, &node.span, cx)
            .then(|| Spanned::new(Number::Float(*value), node.span.clone())),
        _ => {
            cx.wrong_type(node, subject, "a number");
            None
        }
    }
}

fn object_form(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> TypeForm {
    let properties = fields
        .require("properties", cx)
        .and_then(|node| {
            field_map_at(
                node,
                &format!("`properties` of {subject}"),
                nested(surface),
                depth + 1,
                cx,
            )
        })
        .unwrap_or_else(|| FieldMap {
            fields: Vec::new(),
            surface: nested(surface),
            span: fields.span.clone(),
        });

    let mut optional = Vec::new();
    if let Some(node) = fields.take("optional")
        && let Some(items) = expect_sequence(node, &format!("`optional` in {subject}"), cx)
    {
        let declared: Vec<&str> = properties
            .fields
            .iter()
            .map(|field| field.name.value.as_str())
            .collect();
        for item in items {
            let Some(text) = expect_string(item, "each entry of `optional`", cx) else {
                continue;
            };
            let Some(name) = lexical::identifier(&text, "property name", cx) else {
                continue;
            };
            if !declared.contains(&name.value.as_str()) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        name.span.clone(),
                        format!(
                            "`optional` names `{}`, which is not a declared property of {subject}",
                            name.value
                        ),
                    )
                    .with_optional_help(
                        suggest(name.value.as_str(), &declared)
                            .map(|property| format!("did you mean `{property}`?")),
                    ),
                );
                continue;
            }
            if optional
                .iter()
                .any(|other: &Spanned<Ident>| other.value == name.value)
            {
                cx.error(
                    DiagnosticCode::InvalidValue,
                    &name.span,
                    format!("`optional` names `{}` twice", name.value),
                );
                continue;
            }
            optional.push(name);
        }
    }

    let default = default_value(fields, subject, surface, cx).inspect(|value| {
        if !matches!(value.value, Literal::Mapping(_)) {
            cx.error(
                DiagnosticCode::InvalidValue,
                &value.span,
                format!(
                    "`default` for an object must be a mapping, found {}",
                    value.value.description()
                ),
            );
        }
    });

    TypeForm::Object(ObjectType {
        properties,
        optional,
        default,
    })
}

fn array_form(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> TypeForm {
    let items = fields
        .require("items", cx)
        .map(|node| {
            type_node(
                node,
                &format!("`items` of {subject}"),
                nested(surface),
                depth + 1,
                cx,
            )
        })
        .unwrap_or_else(|| TypeNode {
            form: TypeForm::Invalid,
            description: None,
            span: fields.span.clone(),
        });

    let max_items = fields
        .integer("max_items", cx)
        .filter(|value| in_range(value, "`max_items`", 1..=10_000, cx));
    if max_items.is_none() && surface.requires_max_items() && !fields.contains("max_items") {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::MissingKey,
                fields.span.clone(),
                format!("missing required key `max_items` in {subject}"),
            )
            .with_help(
                "arrays in a result schema declare a bound, so model-produced cardinality is finite and every edge payload stays serializable (grammar 3.5)",
            ),
        );
    }

    let min_items = fields
        .integer("min_items", cx)
        .filter(|value| at_least(value, "`min_items`", 0, cx));
    if let (Some(min), Some(max)) = (min_items.as_ref(), max_items.as_ref())
        && min.value > max.value
    {
        cx.error(
            DiagnosticCode::ValueOutOfRange,
            &min.span,
            format!(
                "`min_items` ({}) must not exceed `max_items` ({})",
                min.value, max.value
            ),
        );
    }

    let unique_items = fields.boolean("unique_items", cx);
    let default = default_value(fields, subject, surface, cx).inspect(|value| {
        if !matches!(value.value, Literal::Sequence(_)) {
            cx.error(
                DiagnosticCode::InvalidValue,
                &value.span,
                format!(
                    "`default` for an array must be a sequence, found {}",
                    value.value.description()
                ),
            );
        }
    });

    TypeForm::Array(ArrayType {
        items: Box::new(items),
        max_items,
        min_items,
        unique_items,
        default,
    })
}

fn enum_form(fields: &mut Fields<'_>, subject: &str, surface: Surface, cx: &mut Cx) -> TypeForm {
    let mut variants: Vec<Spanned<String>> = Vec::new();
    if let Some(node) = fields.take("enum")
        && let Some(items) = expect_sequence(node, &format!("`enum` in {subject}"), cx)
    {
        if items.is_empty() {
            cx.error(
                DiagnosticCode::InvalidValue,
                &node.span,
                format!("`enum` in {subject} must declare at least one variant"),
            );
        }
        for item in items {
            let Some(variant) = expect_string(item, "each `enum` variant", cx) else {
                continue;
            };
            // A schema is one of the surfaces where nothing is interpolated, so
            // a `${NAME}` token here is an error rather than six literal
            // characters (grammar 4.3, Decision D41).
            lexical::reject_env_refs(&variant, "an `enum` variant", cx);
            if variant.value.is_empty() {
                cx.error(
                    DiagnosticCode::InvalidValue,
                    &variant.span,
                    "an `enum` variant must not be empty",
                );
                continue;
            }
            if let Some(first) = variants.iter().find(|other| other.value == variant.value) {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        variant.span.clone(),
                        format!("duplicate `enum` variant `{}`", variant.value),
                    )
                    .with_label(first.span.clone(), "first declared here"),
                );
                continue;
            }
            variants.push(variant);
        }
    }

    let default = default_value(fields, subject, surface, cx)
        .inspect(|value| check_enum_default(value, &variants, cx));

    TypeForm::Enum(EnumType { variants, default })
}

/// An enum's `default:` names one of its declared variants (grammar 3.3).
fn check_enum_default(value: &Spanned<Literal>, variants: &[Spanned<String>], cx: &mut Cx) {
    let Literal::String(text) = &value.value else {
        cx.error(
            DiagnosticCode::InvalidValue,
            &value.span,
            format!(
                "`default` for an enum must be one of its string variants, found {}",
                value.value.description()
            ),
        );
        return;
    };
    // An empty variant list has already been reported; measuring the default
    // against it would say the same thing twice.
    if variants.is_empty() || variants.iter().any(|variant| &variant.value == text) {
        return;
    }
    cx.push(
        Diagnostic::error(
            DiagnosticCode::InvalidValue,
            value.span.clone(),
            format!("`default` is `{text}`, which is not one of the declared variants"),
        )
        .with_help(format!(
            "the variants are {}",
            list(variants.iter().map(|variant| variant.value.as_str()))
        )),
    );
}

fn union_form(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    depth: usize,
    cx: &mut Cx,
) -> TypeForm {
    let discriminator = fields
        .take("discriminator")
        .and_then(|node| expect_string(node, &format!("`discriminator` in {subject}"), cx))
        .and_then(|text| lexical::identifier(&text, "discriminator field name", cx));

    // A union takes no `default:`, at any surface — including a state channel,
    // where every other form's default is the channel's initial value. The
    // literal would have to name a variant, and manufacturing a discriminator
    // tag is the same silent routing decision the result-surface prohibition
    // refuses (grammar 3.6, Decision D77).
    if let Some(node) = fields.take("default") {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("`default` is not allowed on the discriminated union in {subject}"),
            )
            .with_help(
                "a union default would have to name a variant, which manufactures a discriminator tag: give the channel or the field one of the union's variants as its own type instead (grammar 3.6, Decision D77)",
            ),
        );
    }

    let mut variants = Vec::new();
    if let Some(node) = fields.require("variants", cx)
        && let Some(mapping) = expect_mapping(node, &format!("`variants` in {subject}"), cx)
    {
        // Stated over how many variants the source declares, so it is not asked
        // of a `variants:` block the loader dropped entries from: each drop
        // already has its own diagnostic, and "declares 1" of a mapping that
        // declares two, one of them a duplicate key, is a second diagnostic for
        // one mistake.
        if mapping.dropped() == 0 && mapping.len() < 2 {
            cx.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidValue,
                    node.span.clone(),
                    format!(
                        "a discriminated union declares at least 2 variants, {subject} declares {}",
                        mapping.len()
                    ),
                )
                .with_help("a one-variant union is an object: write it with `type: object`"),
            );
        }
        for entry in mapping.entries() {
            let Some(tag) = lexical::key_identifier(&entry.key, "variant tag", cx) else {
                continue;
            };
            let Some(payload) = field_map_at(
                &entry.value,
                &format!("variant `{}` of {subject}", tag.value),
                nested(surface),
                depth + 1,
                cx,
            ) else {
                continue;
            };
            if let Some(discriminator) = discriminator.as_ref()
                && let Some(field) = payload.field(discriminator.value.as_str())
            {
                cx.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidValue,
                        field.name.span.clone(),
                        format!(
                            "variant `{}` redeclares the discriminator field `{}`",
                            tag.value, discriminator.value
                        ),
                    )
                    .with_label(discriminator.span.clone(), "declared as the discriminator here")
                    .with_help(
                        "the compiler synthesizes the discriminator as a string constant equal to the variant tag",
                    ),
                );
            }
            variants.push(UnionVariant {
                tag,
                fields: payload,
            });
        }
    }

    match discriminator {
        Some(discriminator) => TypeForm::Union(UnionType {
            discriminator,
            variants,
        }),
        None => TypeForm::Invalid,
    }
}

/// Read a `default:`, rejecting it where the surface forbids one (grammar 3.6,
/// Decision D77).
///
/// Every type-node form but a discriminated union accepts one, at every input
/// surface and on a state channel; a union never does, at any surface, which is
/// [`union_form`]'s rule because it is a property of the form.
fn default_value(
    fields: &mut Fields<'_>,
    subject: &str,
    surface: Surface,
    cx: &mut Cx,
) -> Option<Spanned<Literal>> {
    let node = fields.take("default")?;
    if !surface.allows_default() {
        cx.push(
            Diagnostic::error(
                DiagnosticCode::InvalidValue,
                node.span.clone(),
                format!("`default` is not allowed in {subject}: it is a result schema"),
            )
            .with_help(
                "a defaulted model output would silently manufacture routing values (grammar 3.6)",
            ),
        );
        return None;
    }
    let value = literal(node);
    let context = format!("`default` in {subject}");
    // A default is written inside a schema, and a schema interpolates nothing:
    // an unescaped `${NAME}` here would reach the IR as the six characters the
    // author did not intend (grammar 4.3, Decision D41).
    reject_env_refs_in_literal(&value, &context, cx);
    // A default is data, and this data is lowered to JSON (grammar 3.8).
    reject_non_finite_in_literal(&value, &context, cx);
    Some(value)
}

/// Reject every `${NAME}` token a literal carries, at any depth.
///
/// A composite `default:` on a state channel is a whole initial value, so the
/// token can hide in a nested string or a mapping key rather than at the top;
/// the same is true of a model's `settings:`, which is the other surface
/// grammar 4.3 puts in class 3 as a whole subtree rather than a single string.
pub(crate) fn reject_env_refs_in_literal(value: &Spanned<Literal>, subject: &str, cx: &mut Cx) {
    match &value.value {
        Literal::String(text) => {
            lexical::reject_env_refs(&Spanned::new(text.clone(), value.span.clone()), subject, cx)
        }
        Literal::Sequence(items) => {
            for item in items {
                reject_env_refs_in_literal(item, subject, cx);
            }
        }
        Literal::Mapping(entries) => {
            for entry in entries {
                lexical::reject_env_refs(&entry.key, subject, cx);
                reject_env_refs_in_literal(&entry.value, subject, cx);
            }
        }
        Literal::Null | Literal::Bool(_) | Literal::Int(_) | Literal::Float(_) => {}
    }
}

/// Reject every infinity or NaN a literal carries, at any depth.
///
/// Both surfaces that carry a literal are walked to their leaves: a composite
/// `default:` on a state channel is a whole initial value, and a model's
/// `settings:` is open and arbitrarily deep, so the unwritable number can sit
/// inside an array or an object rather than at the top. Every one of them is
/// lowered into the artifact and from there into JSON (grammar 3.8), which has
/// no notation for either — so it is refused where it is written, while there
/// is still a span to point at (see [`expect_finite`]).
pub(crate) fn reject_non_finite_in_literal(value: &Spanned<Literal>, subject: &str, cx: &mut Cx) {
    match &value.value {
        Literal::Float(number) => {
            expect_finite(*number, subject, &value.span, cx);
        }
        Literal::Sequence(items) => {
            for item in items {
                reject_non_finite_in_literal(item, subject, cx);
            }
        }
        Literal::Mapping(entries) => {
            for entry in entries {
                reject_non_finite_in_literal(&entry.value, subject, cx);
            }
        }
        Literal::Null | Literal::Bool(_) | Literal::Int(_) | Literal::String(_) => {}
    }
}

fn check_default_kind(value: &Spanned<Literal>, kind: ScalarKind, subject: &str, cx: &mut Cx) {
    let matches = matches!(
        (&value.value, kind),
        (Literal::String(_), ScalarKind::String)
            | (Literal::Int(_), ScalarKind::Integer | ScalarKind::Number)
            | (Literal::Float(_), ScalarKind::Number)
            | (Literal::Bool(_), ScalarKind::Boolean)
    );
    if !matches {
        cx.error(
            DiagnosticCode::InvalidValue,
            &value.span,
            format!(
                "`default` in {subject} must be {}, found {}",
                match kind {
                    ScalarKind::String => "a string",
                    ScalarKind::Integer => "an integer",
                    ScalarKind::Number => "a number",
                    ScalarKind::Boolean => "a boolean",
                },
                value.value.description()
            ),
        );
    }
}

/// Convert a YAML node into a spanned literal, for the surfaces that carry data
/// rather than schema.
pub(crate) fn literal(node: &Node) -> Spanned<Literal> {
    let value = match &node.value {
        Yaml::Null => Literal::Null,
        Yaml::Bool(value) => Literal::Bool(*value),
        Yaml::Int(value) => Literal::Int(*value),
        Yaml::Float(value) => Literal::Float(*value),
        Yaml::String(value) => Literal::String(value.clone()),
        Yaml::Sequence(items) => Literal::Sequence(items.iter().map(literal).collect()),
        Yaml::Mapping(mapping) => Literal::Mapping(
            mapping
                .entries()
                .iter()
                .map(|entry| LiteralEntry {
                    key: entry.key.clone(),
                    value: literal(&entry.value),
                })
                .collect(),
        ),
    };
    Spanned::new(value, node.span.clone())
}

#[cfg(test)]
mod tests {
    use crate::parse_str;

    /// Every diagnostic one source produces, as `code: message`. The whole list
    /// is compared, so a second diagnostic for one mistake fails the test.
    fn diagnostics(source: &str) -> Vec<String> {
        parse_str(source, "test.yml".to_string())
            .diagnostics
            .iter()
            .map(|diagnostic| format!("{}: {}", diagnostic.code, diagnostic.message))
            .collect()
    }

    /// An agent whose only variable is its `output:` surface — the surface that
    /// carries grammar 5.1's ≥1-property rule, which is what a synthesized empty
    /// field map would trip.
    fn agent(output: &str) -> String {
        format!(
            "agent.a:
  model: model.m
  prompt: Write a draft.
  output:
{output}"
        )
    }

    /// A union at a declaration surface is one mistake about the surface's
    /// *shape*, so it draws one diagnostic. Handing the caller an empty field
    /// map instead would add "must declare at least one property", which is
    /// wrong-headed advice: the surface is not a short field map, it is not a
    /// field map at all (grammar 3.7, 5.1, Decision D11).
    #[test]
    fn a_union_at_a_declaration_surface_is_not_also_an_empty_field_map() {
        assert_eq!(
            diagnostics(&agent(
                "    discriminator: kind\n    variants:\n      a: { x: { type: string } }\n      b: { y: { type: string } }\n"
            )),
            [
                "invalid-value: `output` of agent definition `agent.a` is a discriminated union, but a field map belongs here",
            ]
        );
    }

    /// The same for a field map whose every entry the loader dropped: the author
    /// declared a property, and the key that was not a string already has its
    /// own diagnostic (grammar 1.1).
    #[test]
    fn a_field_map_whose_only_key_was_dropped_is_not_an_empty_field_map() {
        assert_eq!(
            diagnostics(&agent("    1: { type: string }\n")),
            ["non-string-key: mapping keys must be strings, found an integer"]
        );
    }

    /// And for one whose every entry this pass rejected, which is the same
    /// question one level up: `Verdict` is a mapping key the loader keeps and
    /// the field-name rule refuses (grammar 2.1).
    #[test]
    fn a_field_map_whose_only_field_name_was_rejected_is_not_an_empty_field_map() {
        assert_eq!(
            diagnostics(&agent("    Verdict: { type: string }\n")),
            ["invalid-identifier: field name `Verdict` is not a valid identifier"]
        );
    }

    /// An `output: {}` really is the empty field map, and grammar 5.1 refuses
    /// it — the suppressions above must not swallow the rule they guard.
    #[test]
    fn an_empty_output_surface_still_fails_the_one_property_rule() {
        assert_eq!(
            diagnostics("agent.a:\n  model: model.m\n  prompt: Write a draft.\n  output: {}\n"),
            [
                "invalid-value: `output` of agent definition `agent.a` must declare at least one property",
            ]
        );
    }

    /// A `variants:` arity is stated over how many variants the source declares,
    /// so a duplicate tag is one mistake: reporting "declares 1" as well would
    /// count what survived rather than what was written (grammar 1.1, 3.7).
    #[test]
    fn a_duplicate_variant_tag_is_not_also_a_one_variant_union() {
        assert_eq!(
            diagnostics(&agent(
                "    r:\n      discriminator: kind\n      variants:\n        x: { a: { type: string } }\n        x: { b: { type: string } }\n"
            )),
            ["duplicate-key: duplicate key `x`"]
        );
    }
}
