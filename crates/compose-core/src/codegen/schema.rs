//! Grammar 3.8's mapping table, implemented once in both directions.
//!
//! ```text
//! DSL  ──►  JSON Schema 2020-12  ──►  Zod
//! ```
//!
//! Both lowerings live here on purpose. The table is one contract with two
//! columns, and two modules implementing one column each would drift the first
//! time a constraint key was added to one of them —
//! `tests/generated_code_gates.rs` exists to catch exactly that, running one
//! corpus of documents against both columns, and it can only compare the two if
//! both are reachable from one place.
//!
//! # Where the table left latitude, and what was chosen
//!
//! Grammar 3.8 fixes seven rows and the snake_case → camelCase key rule. The
//! rest of the vocabulary of grammar 3.3–3.7 is not in the table, so the choices
//! below are this module's, made to keep the two columns *observably equivalent*
//! (that is what the conformance corpus checks) rather than merely plausible:
//!
//! | DSL | JSON Schema | Zod | why |
//! |---|---|---|---|
//! | `description` | `description` | `.describe(…)` | it reaches the model in a structured-output schema (grammar 3.2) |
//! | `format: email` | `format: "email"` | `z.email()` | Zod 4's format constructors are the non-deprecated spelling; `z.string().email()` is the one it tells you not to write |
//! | `format: date-time` | `format: "date-time"` | `z.iso.datetime({ offset: true })` | JSON Schema's `date-time` is RFC 3339, which admits `+02:00`; Zod's default admits only `Z` |
//! | `format: time` | `format: "time"` | `z.string().regex(rfc3339Time)` | RFC 3339 `full-time` **requires** an offset and `z.iso.time()` **refuses** one, so the two constructors are disjoint rather than merely different |
//! | `exclusive_minimum` | `exclusiveMinimum` | `.gt(…)` | Zod spells the exclusive bounds `gt`/`lt` |
//! | `multiple_of` | `multipleOf` | `.multipleOf(…)` | |
//! | `min_items` | `minItems` | `.min(…)` | |
//! | `unique_items` | `uniqueItems` | `.refine(uniqueItems, …)` | Zod has no built-in; the emitted helper compares JSON encodings |
//! | `optional: [b]` **and** `default:` on `b` | `b` omitted from `required` | `.default(v)` alone | grammar 3.6 makes a defaulted property implicitly optional, and `.default(v).optional()` would answer `undefined` for an omitted property instead of the default — the one composition where the two orderings differ |
//!
//! Two spellings are the table's own and are kept verbatim even where Zod offers
//! a newer one: a closed object is `z.object({…}).strict()` (not
//! `z.strictObject`), and an integer is `z.number().int()` (not `z.int()`). The
//! grammar is normative and both pairs denote the same schema.
//!
//! One divergence is **known and left**: a leap second. `rfc3339Time` admits
//! `:60` wherever the rest of the value parses, while the JSON Schema column
//! admits it only at `23:59:60`. Narrowing the regex to `[0-5]\d` would trade
//! that corner for the opposite one — refusing a value JSON Schema accepts — and
//! neither is reachable from a model that emits a wall-clock time.
//!
//! # Patterns
//!
//! `pattern:` is RE2 (Decision D12), a subset of JavaScript's syntax, so the
//! source text transfers unchanged. It is written as a regex **literal** with
//! any unescaped `/` escaped, and falls back to `new RegExp("…")` for a pattern
//! carrying a line terminator, which a literal cannot hold. No flags are added:
//! `u` mode rejects escapes RE2 accepts, and both `RegExp.test` and JSON
//! Schema's `pattern` are unanchored searches, so the two agree without one.

use serde_json::{Map, Value, json};

use crate::ast::schema::{Number, ScalarKind, StringFormat};
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::NodeKind;
use crate::ir::schema::{ArrayType, EnumType, FieldMap, ObjectType, Scalar, TypeForm, TypeNode};
use crate::ir::{Channel, Ir};

use super::names::{self, Names};

/// One schema the emitted project declares, and where it came from.
#[derive(Clone, Debug)]
pub struct Surface<'ir> {
    /// The canonical path (see [`super::names`]), which is the key every other
    /// module reaches this schema by.
    pub path: String,
    /// What it is, for the doc comment above it.
    pub about: String,
    /// The schema itself.
    pub body: Body<'ir>,
}

/// A schema is either a declaration surface's field map or a single type node —
/// a state channel is the only one of the second kind (grammar 10.1).
#[derive(Clone, Copy, Debug)]
pub enum Body<'ir> {
    /// A field map: a closed object (grammar 3.1).
    Fields(&'ir FieldMap),
    /// One type node, which is what a channel declares.
    Type(&'ir TypeNode),
}

/// Every schema of the composition, in canonical order.
///
/// Definitions come first, in the address order [`Ir::definitions`] already
/// holds them in, each definition's surfaces in the order grammar 3.9 tabulates
/// them, and a flow's node-level schemas after its own in **node declaration
/// order**. State channels come last, in channel-name order.
///
/// This function is the single enumeration of "every schema there is": the name
/// registry, the emitted module, and the conformance corpus all walk it, so a
/// surface added to the grammar reaches all three at once or none.
#[must_use]
pub fn surfaces(ir: &Ir) -> Vec<Surface<'_>> {
    let mut surfaces = Vec::new();
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Agent(agent) => {
                if let Some(input) = &agent.input {
                    surfaces.push(Surface {
                        path: format!("{address}.input"),
                        about: format!("`{address}` — its declared input (grammar 5.3)."),
                        body: Body::Fields(input),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!(
                        "`{address}` — the structured output the model is constrained to, and \
                         what routing reads (PRD 5.2, 5.3)."
                    ),
                    body: Body::Fields(&agent.output),
                });
            }
            DefinitionBody::Tool(tool) => {
                surfaces.push(Surface {
                    path: format!("{address}.input"),
                    about: format!("`{address}` — its parameters (grammar 6)."),
                    body: Body::Fields(&tool.input),
                });
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!("`{address}` — its result (grammar 6)."),
                    body: Body::Fields(&tool.output),
                });
            }
            DefinitionBody::Flow(flow) => {
                if let Some(inputs) = &flow.inputs {
                    surfaces.push(Surface {
                        path: format!("{address}.inputs"),
                        about: format!("`{address}` — the module's parameters (grammar 7.5)."),
                        body: Body::Fields(inputs),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.outputs"),
                    about: format!(
                        "`{address}` — the module's result, materialized from the state channels \
                         of the same names at quiescence (grammar 7.5, 7.6.3)."
                    ),
                    body: Body::Fields(&flow.outputs),
                });
                for node in &flow.nodes {
                    let id = node.id.value.as_str();
                    match &node.kind {
                        NodeKind::Human { human } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.input"),
                                about: format!(
                                    "`{address}` node `{id}` — what the human is shown \
                                     (grammar 8.7)."
                                ),
                                body: Body::Fields(&human.input),
                            });
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the human returns, routable \
                                     like any structured output (grammar 8.7)."
                                ),
                                body: Body::Fields(&human.output),
                            });
                        }
                        NodeKind::Exec { exec } => {
                            if let Some(output) = &exec.output {
                                surfaces.push(Surface {
                                    path: format!("{address}.node.{id}.output"),
                                    about: format!(
                                        "`{address}` node `{id}` — what the subprocess produces \
                                         (grammar 8.2)."
                                    ),
                                    body: Body::Fields(output),
                                });
                            }
                        }
                        NodeKind::Http { http } => {
                            if let Some(output) = &http.output {
                                surfaces.push(Surface {
                                    path: format!("{address}.node.{id}.output"),
                                    about: format!(
                                        "`{address}` node `{id}` — what the response decodes to \
                                         (grammar 8.3)."
                                    ),
                                    body: Body::Fields(output),
                                });
                            }
                        }
                        NodeKind::Agent { .. }
                        | NodeKind::Function { .. }
                        | NodeKind::Flow { .. }
                        | NodeKind::Map { .. }
                        | NodeKind::Store { .. } => {}
                    }
                }
            }
            DefinitionBody::Store(store) => {
                if let Some(value) = &store.value_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.value_schema"),
                        about: format!("`{address}` — the value it stores (grammar 11.1)."),
                        body: Body::Fields(value),
                    });
                }
                if let Some(metadata) = &store.metadata_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.metadata_schema"),
                        about: format!(
                            "`{address}` — the metadata a match carries (grammar 11.1)."
                        ),
                        body: Body::Fields(metadata),
                    });
                }
            }
            DefinitionBody::Provider(_) | DefinitionBody::Model(_) => {}
        }
    }

    if let Some(state) = &ir.state {
        for (name, channel) in &state.entries {
            surfaces.push(Surface {
                path: format!("state.{name}"),
                about: format!("State channel `{name}` — its declared type (grammar 10.1)."),
                body: Body::Type(&channel.ty),
            });
        }
    }

    surfaces
}

/// Every state channel, in channel-name order, paired with its canonical path.
///
/// [`super::state`] reads this rather than `ir.state` directly so the two
/// modules cannot disagree about which channels exist or what they are called.
#[must_use]
pub fn channels(ir: &Ir) -> Vec<(String, &Channel)> {
    ir.state
        .as_ref()
        .map(|state| {
            state
                .entries
                .iter()
                .map(|(name, channel)| (format!("state.{name}"), channel))
                .collect()
        })
        .unwrap_or_default()
}

// ---------------------------------------------------------------------------
// The Zod column
// ---------------------------------------------------------------------------

/// `src/schemas.ts`: every schema of the composition, as Zod.
#[must_use]
pub fn module(ir: &Ir, names: &Names) -> super::GeneratedFile {
    let surfaces = surfaces(ir);
    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents.push_str("\nimport { z } from \"zod\";\n");

    // Two helpers, each emitted only where the composition reaches the form that
    // needs it — an unused `const` in a generated module is a question a reader
    // has to answer for nothing.
    if surfaces.iter().any(|surface| {
        surface.declares(
            &|ty| matches!(&ty.form, TypeForm::Array(array) if array.unique_items == Some(true)),
        )
    }) {
        contents.push_str(UNIQUE_ITEMS_HELPER);
    }
    if surfaces.iter().any(|surface| {
        surface.declares(&|ty| {
            matches!(&ty.form, TypeForm::Scalar(scalar) if scalar.format == Some(StringFormat::Time))
        })
    }) {
        contents.push_str(RFC3339_TIME_HELPER);
    }

    for surface in &surfaces {
        let name = names.value(&surface.path);
        let ty = names.ty(&surface.path);
        contents.push('\n');
        contents.push_str(&names::doc("", std::slice::from_ref(&surface.about)));
        let expression = match surface.body {
            Body::Fields(fields) => field_map(fields, ""),
            Body::Type(node) => type_node(node, ""),
        };
        contents.push_str(&format!("export const {name} = {expression};\n"));
        contents.push_str(&format!("export type {ty} = z.infer<typeof {name}>;\n"));
    }

    super::GeneratedFile {
        path: "src/schemas.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// Every schema the composition declares, lowered per grammar 3.8. These are the
// runtime contract: an agent's output is parsed with the schema named for it
// before any edge is evaluated, a tool's arguments are parsed with its input
// schema, and a state channel's type is the schema its channel is built from.
";

const UNIQUE_ITEMS_HELPER: &str = r#"
/**
 * `unique_items: true` (grammar 3.5). Zod has no built-in, and JSON encodings
 * are what JSON Schema's `uniqueItems` compares, so that is what this compares.
 */
const uniqueItems = (items: readonly unknown[]): boolean =>
  new Set(items.map((item) => JSON.stringify(item))).size === items.length;
"#;

const RFC3339_TIME_HELPER: &str = r#"
/**
 * `format: time` (grammar 3.3), which JSON Schema reads as RFC 3339 `full-time`
 * — an offset is required. Zod's `z.iso.time()` refuses an offset outright, so
 * the check is written here rather than borrowed from a constructor that means
 * something else.
 */
const rfc3339Time =
  /^([01]\d|2[0-3]):[0-5]\d:([0-5]\d|60)(\.\d+)?([Zz]|[+-]([01]\d|2[0-3]):[0-5]\d)$/;
"#;

impl Surface<'_> {
    /// Whether any type node anywhere inside this surface satisfies `predicate`.
    fn declares(&self, predicate: &dyn Fn(&TypeNode) -> bool) -> bool {
        match self.body {
            Body::Fields(fields) => fields
                .fields
                .iter()
                .any(|field| declares(&field.ty, predicate)),
            Body::Type(ty) => declares(ty, predicate),
        }
    }
}

/// Whether this type node or anything nested inside it satisfies `predicate`.
fn declares(ty: &TypeNode, predicate: &dyn Fn(&TypeNode) -> bool) -> bool {
    if predicate(ty) {
        return true;
    }
    match &ty.form {
        TypeForm::Array(array) => declares(&array.items, predicate),
        TypeForm::Object(object) => object
            .properties
            .fields
            .iter()
            .any(|field| declares(&field.ty, predicate)),
        TypeForm::Union(union) => union.variants.iter().any(|variant| {
            variant
                .fields
                .fields
                .iter()
                .any(|field| declares(&field.ty, predicate))
        }),
        TypeForm::Scalar(_) | TypeForm::Enum(_) => false,
    }
}

/// A field map as `z.object({ … }).strict()` — a closed object (grammar 3.1,
/// Decision D8).
///
/// Every declared property is **required**. `optional:` is a key of an *object*
/// type node (grammar 3.4), not of a declaration surface, so the only thing that
/// makes a property of a field map optional is its own `default:` (grammar 3.6)
/// — which `.default(…)` already expresses.
///
/// `indent` is the indentation of the line the expression starts on; the
/// object's properties sit one level deeper and the closing brace comes back to
/// it.
#[must_use]
pub fn field_map(map: &FieldMap, indent: &str) -> String {
    properties(&map.fields, &[], indent)
}

/// The shared body of a field map and an object type node: `z.object({…})`
/// closed with `.strict()`, with `optional:` applied to the properties that name
/// it.
fn properties(
    fields: &[crate::ir::schema::Field],
    optional: &[crate::diag::Spanned<crate::ast::common::Ident>],
    indent: &str,
) -> String {
    if fields.is_empty() {
        return "z.object({}).strict()".to_string();
    }
    let inner = format!("{indent}  ");
    let mut text = String::from("z.object({\n");
    for field in fields {
        let name = field.name.value.as_str();
        let mut rendered = type_node(&field.ty, &inner);
        // A defaulted property is already optional at its surface (grammar 3.6),
        // and `.default(v).optional()` would answer an omitted property with
        // `undefined` instead of the default — see the module docs.
        if declared_default(&field.ty).is_none()
            && optional.iter().any(|entry| entry.value.as_str() == name)
        {
            rendered.push_str(".optional()");
        }
        text.push_str(&format!("{inner}{}: {rendered},\n", property_key(name)));
    }
    text.push_str(&format!("{indent}}}).strict()"));
    text
}

/// A property key, quoted only when it has to be.
///
/// Identifiers are grammar 2.1's `lower , { lower | digit | "_" }`, every one of
/// which is a legal JavaScript property name, so no key here is ever quoted —
/// the function exists so that stays a decision rather than an assumption.
fn property_key(name: &str) -> String {
    let bare = name
        .chars()
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && name.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        });
    if bare {
        name.to_string()
    } else {
        names::string(name)
    }
}

/// A type node as a Zod expression (grammar 3.2).
#[must_use]
pub fn type_node(ty: &TypeNode, indent: &str) -> String {
    let mut text = match &ty.form {
        TypeForm::Scalar(scalar) => scalar_expression(scalar),
        TypeForm::Enum(enumeration) => enum_expression(enumeration),
        TypeForm::Object(object) => object_expression(object, indent),
        TypeForm::Array(array) => array_expression(array, indent),
        TypeForm::Union(union) => union_expression(union, indent),
    };
    if let Some(description) = &ty.description {
        text.push_str(&format!(".describe({})", names::string(&description.value)));
    }
    if let Some(default) = declared_default(ty) {
        text.push_str(&format!(".default({})", names::literal(default)));
    }
    text
}

/// The `default:` a type node declares, whichever form carries it. A union never
/// does (grammar 3.6).
fn declared_default(ty: &TypeNode) -> Option<&crate::ast::common::Literal> {
    match &ty.form {
        TypeForm::Scalar(scalar) => scalar.default.as_ref(),
        TypeForm::Enum(enumeration) => enumeration.default.as_ref(),
        TypeForm::Object(object) => object.default.as_ref(),
        TypeForm::Array(array) => array.default.as_ref(),
        TypeForm::Union(_) => None,
    }
    .map(|default| &default.value)
}

fn scalar_expression(scalar: &Scalar) -> String {
    let mut text = match (scalar.kind, scalar.format) {
        (ScalarKind::String, Some(format)) => zod_format(format).to_string(),
        (ScalarKind::String, None) => "z.string()".to_string(),
        (ScalarKind::Integer, _) => "z.number().int()".to_string(),
        (ScalarKind::Number, _) => "z.number()".to_string(),
        (ScalarKind::Boolean, _) => "z.boolean()".to_string(),
    };
    if let Some(min) = scalar.min_length {
        text.push_str(&format!(".min({min})"));
    }
    if let Some(max) = scalar.max_length {
        text.push_str(&format!(".max({max})"));
    }
    if let Some(pattern) = &scalar.pattern {
        text.push_str(&format!(".regex({})", regex_expression(pattern)));
    }
    for (constraint, value) in [
        ("min", scalar.minimum.as_ref()),
        ("max", scalar.maximum.as_ref()),
        ("gt", scalar.exclusive_minimum.as_ref()),
        ("lt", scalar.exclusive_maximum.as_ref()),
        ("multipleOf", scalar.multiple_of.as_ref()),
    ] {
        if let Some(value) = value {
            text.push_str(&format!(".{constraint}({})", names::number(value)));
        }
    }
    text
}

/// Zod 4's constructor for one `format:` (grammar 3.3).
///
/// Two of the ten need more than the bare constructor, because Zod's default
/// reading of the ISO form is narrower than the RFC 3339 one JSON Schema's
/// `format` keyword means — and the JSON Schema is what a model is handed, so a
/// value it blessed that the emitted Zod then refused would be a run failing on
/// its own contract:
///
/// * `date-time` takes `{ offset: true }`. Zod's default accepts only `Z`;
///   RFC 3339 — and therefore JSON Schema — accepts `+02:00` just as happily.
/// * `time` is not `z.iso.time()` at all. That constructor **refuses** an offset,
///   and RFC 3339's `full-time` **requires** one, so the two are disjoint rather
///   than merely different; there is no option to reconcile them on this pinned
///   Zod. The check is written out as [`RFC3339_TIME`] instead.
const fn zod_format(format: StringFormat) -> &'static str {
    match format {
        StringFormat::DateTime => "z.iso.datetime({ offset: true })",
        StringFormat::Date => "z.iso.date()",
        StringFormat::Time => "z.string().regex(rfc3339Time)",
        StringFormat::Duration => "z.iso.duration()",
        StringFormat::Email => "z.email()",
        StringFormat::Uri => "z.url()",
        StringFormat::Uuid => "z.uuid()",
        StringFormat::Hostname => "z.hostname()",
        StringFormat::Ipv4 => "z.ipv4()",
        StringFormat::Ipv6 => "z.ipv6()",
    }
}

fn enum_expression(enumeration: &EnumType) -> String {
    let variants: Vec<String> = enumeration
        .variants
        .iter()
        .map(|variant| names::string(&variant.value))
        .collect();
    format!("z.enum([{}])", variants.join(", "))
}

fn object_expression(object: &ObjectType, indent: &str) -> String {
    properties(&object.properties.fields, &object.optional, indent)
}

fn array_expression(array: &ArrayType, indent: &str) -> String {
    let mut text = format!("z.array({})", type_node(&array.items, indent));
    if let Some(min) = array.min_items {
        text.push_str(&format!(".min({min})"));
    }
    if let Some(max) = array.max_items {
        text.push_str(&format!(".max({max})"));
    }
    if array.unique_items == Some(true) {
        text.push_str(".refine(uniqueItems, { message: \"expected unique items\" })");
    }
    text
}

fn union_expression(union: &crate::ir::schema::UnionType, indent: &str) -> String {
    let inner = format!("{indent}  ");
    let discriminator = union.discriminator.value.as_str();
    let mut text = format!("z.discriminatedUnion({}, [\n", names::string(discriminator));
    for variant in &union.variants {
        let deeper = format!("{inner}  ");
        let mut object = String::from("z.object({\n");
        object.push_str(&format!(
            "{deeper}{}: z.literal({}),\n",
            property_key(discriminator),
            names::string(variant.tag.value.as_str())
        ));
        for field in &variant.fields.fields {
            object.push_str(&format!(
                "{deeper}{}: {},\n",
                property_key(field.name.value.as_str()),
                type_node(&field.ty, &deeper)
            ));
        }
        object.push_str(&format!("{inner}}}).strict()"));
        text.push_str(&format!("{inner}{object},\n"));
    }
    text.push_str(&format!("{indent}])"));
    text
}

/// A `pattern:` as a JavaScript regular expression (see the module docs).
fn regex_expression(pattern: &str) -> String {
    if pattern.contains(['\n', '\r', '\u{2028}', '\u{2029}']) {
        return format!("new RegExp({})", names::string(pattern));
    }
    let mut literal = String::with_capacity(pattern.len() + 2);
    literal.push('/');
    let mut escaped = false;
    for character in pattern.chars() {
        if escaped {
            literal.push(character);
            escaped = false;
            continue;
        }
        match character {
            '\\' => {
                literal.push('\\');
                escaped = true;
            }
            '/' => literal.push_str("\\/"),
            other => literal.push(other),
        }
    }
    literal.push('/');
    literal
}

// ---------------------------------------------------------------------------
// The JSON Schema column
// ---------------------------------------------------------------------------

/// A field map as JSON Schema draft 2020-12: a closed object.
#[must_use]
pub fn json_field_map(map: &FieldMap) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in &map.fields {
        let name = field.name.value.as_str();
        properties.insert(name.to_string(), json_type_node(&field.ty));
        if declared_default(&field.ty).is_none() {
            required.push(Value::String(name.to_string()));
        }
    }
    object_schema(properties, required)
}

fn object_schema(properties: Map<String, Value>, required: Vec<Value>) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_string(), json!("object"));
    schema.insert("properties".to_string(), Value::Object(properties));
    schema.insert("required".to_string(), Value::Array(required));
    schema.insert("additionalProperties".to_string(), json!(false));
    Value::Object(schema)
}

/// A type node as JSON Schema draft 2020-12.
#[must_use]
pub fn json_type_node(ty: &TypeNode) -> Value {
    let mut schema = match &ty.form {
        TypeForm::Scalar(scalar) => json_scalar(scalar),
        TypeForm::Enum(enumeration) => {
            let variants: Vec<Value> = enumeration
                .variants
                .iter()
                .map(|variant| Value::String(variant.value.clone()))
                .collect();
            json!({ "type": "string", "enum": variants })
        }
        TypeForm::Object(object) => {
            let mut properties = Map::new();
            let mut required = Vec::new();
            for field in &object.properties.fields {
                let name = field.name.value.as_str();
                properties.insert(name.to_string(), json_type_node(&field.ty));
                let optional = declared_default(&field.ty).is_some()
                    || object
                        .optional
                        .iter()
                        .any(|entry| entry.value.as_str() == name);
                if !optional {
                    required.push(Value::String(name.to_string()));
                }
            }
            object_schema(properties, required)
        }
        TypeForm::Array(array) => {
            let mut schema = Map::new();
            schema.insert("type".to_string(), json!("array"));
            schema.insert("items".to_string(), json_type_node(&array.items));
            if let Some(max) = array.max_items {
                schema.insert("maxItems".to_string(), json!(max));
            }
            if let Some(min) = array.min_items {
                schema.insert("minItems".to_string(), json!(min));
            }
            if array.unique_items == Some(true) {
                schema.insert("uniqueItems".to_string(), json!(true));
            }
            Value::Object(schema)
        }
        TypeForm::Union(union) => {
            let discriminator = union.discriminator.value.as_str();
            let variants: Vec<Value> = union
                .variants
                .iter()
                .map(|variant| {
                    let mut properties = Map::new();
                    let mut required = vec![Value::String(discriminator.to_string())];
                    properties.insert(
                        discriminator.to_string(),
                        json!({ "const": variant.tag.value.as_str() }),
                    );
                    for field in &variant.fields.fields {
                        let name = field.name.value.as_str();
                        properties.insert(name.to_string(), json_type_node(&field.ty));
                        if declared_default(&field.ty).is_none() {
                            required.push(Value::String(name.to_string()));
                        }
                    }
                    object_schema(properties, required)
                })
                .collect();
            json!({ "oneOf": variants })
        }
    };

    let object = schema
        .as_object_mut()
        .expect("every lowering above answers an object");
    if let Some(description) = &ty.description {
        object.insert("description".to_string(), json!(description.value));
    }
    if let Some(default) = declared_default(ty) {
        object.insert("default".to_string(), json_literal(default));
    }
    schema
}

fn json_scalar(scalar: &Scalar) -> Value {
    let mut schema = Map::new();
    schema.insert("type".to_string(), json!(scalar.kind.as_str()));
    if let Some(min) = scalar.min_length {
        schema.insert("minLength".to_string(), json!(min));
    }
    if let Some(max) = scalar.max_length {
        schema.insert("maxLength".to_string(), json!(max));
    }
    if let Some(pattern) = &scalar.pattern {
        schema.insert("pattern".to_string(), json!(pattern));
    }
    if let Some(format) = scalar.format {
        schema.insert("format".to_string(), json!(format.as_str()));
    }
    for (key, value) in [
        ("minimum", scalar.minimum.as_ref()),
        ("maximum", scalar.maximum.as_ref()),
        ("exclusiveMinimum", scalar.exclusive_minimum.as_ref()),
        ("exclusiveMaximum", scalar.exclusive_maximum.as_ref()),
        ("multipleOf", scalar.multiple_of.as_ref()),
    ] {
        if let Some(number) = value {
            schema.insert(key.to_string(), json_number(number));
        }
    }
    Value::Object(schema)
}

fn json_number(number: &Number) -> Value {
    match number {
        Number::Int(int) => json!(int),
        Number::Float(float) => json!(float),
    }
}

/// A literal as JSON data — the same shape [`crate::ir::leaf`] writes it in.
#[must_use]
pub fn json_literal(literal: &crate::ast::common::Literal) -> Value {
    use crate::ast::common::Literal;
    match literal {
        Literal::Null => Value::Null,
        Literal::Bool(boolean) => json!(boolean),
        Literal::Int(int) => json!(int),
        Literal::Float(float) => json!(float),
        Literal::String(text) => json!(text),
        Literal::Sequence(items) => {
            Value::Array(items.iter().map(|item| json_literal(&item.value)).collect())
        }
        Literal::Mapping(entries) => {
            let mut map = Map::new();
            for entry in entries {
                map.insert(entry.key.value.clone(), json_literal(&entry.value.value));
            }
            Value::Object(map)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    fn schema_of(source: &str, path: &str) -> String {
        let ir = ir_of(source);
        let surfaces = surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == path)
            .unwrap_or_else(|| {
                panic!(
                    "`{path}` is not a surface; the composition declares {:?}",
                    surfaces.iter().map(|s| s.path.as_str()).collect::<Vec<_>>()
                )
            });
        match surface.body {
            Body::Fields(fields) => field_map(fields, ""),
            Body::Type(ty) => type_node(ty, ""),
        }
    }

    const CHANNEL: &str = "version: \"0.1\"\nstate:\n";

    #[test]
    fn scalars_carry_their_constraint_vocabulary() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, min_length: 1, max_length: 200 }}\n"),
                "state.a"
            ),
            "z.string().min(1).max(200)"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: integer, minimum: 0, multiple_of: 2 }}\n"),
                "state.a"
            ),
            "z.number().int().min(0).multipleOf(2)"
        );
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a: {{ type: number, exclusive_minimum: 0, exclusive_maximum: 1 }}\n"
                ),
                "state.a"
            ),
            "z.number().gt(0).lt(1)"
        );
        assert_eq!(
            schema_of(&format!("{CHANNEL}  a: {{ type: boolean }}\n"), "state.a"),
            "z.boolean()"
        );
    }

    #[test]
    fn a_format_selects_zods_constructor_for_it() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: date-time }}\n"),
                "state.a"
            ),
            "z.iso.datetime({ offset: true })",
            "RFC 3339 admits an offset and so does JSON Schema's `date-time`; \
             Zod's default does not"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: time }}\n"),
                "state.a"
            ),
            "z.string().regex(rfc3339Time)",
            "`z.iso.time()` refuses the offset RFC 3339 requires"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: email, max_length: 320 }}\n"),
                "state.a"
            ),
            "z.email().max(320)"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: uri }}\n"),
                "state.a"
            ),
            "z.url()"
        );
    }

    #[test]
    fn an_enum_is_a_zod_enum_in_declaration_order() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ enum: [revise, approve] }}\n"),
                "state.a"
            ),
            "z.enum([\"revise\", \"approve\"])"
        );
    }

    #[test]
    fn a_default_becomes_a_zod_default() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, default: \"\" }}\n"),
                "state.a"
            ),
            "z.string().default(\"\")"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: integer, default: 1 }}\n"),
                "state.a"
            ),
            "z.number().int().default(1)"
        );
    }

    #[test]
    fn a_description_becomes_a_zod_describe() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, description: The draft. }}\n"),
                "state.a"
            ),
            "z.string().describe(\"The draft.\")"
        );
    }

    #[test]
    fn an_array_carries_its_bounds_and_its_uniqueness_check() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: array\n    max_items: 20\n    min_items: 1\n    items: {{ type: string }}\n"
                ),
                "state.a"
            ),
            "z.array(z.string()).min(1).max(20)"
        );
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: array\n    unique_items: true\n    items: {{ type: string }}\n"
                ),
                "state.a"
            ),
            "z.array(z.string()).refine(uniqueItems, { message: \"expected unique items\" })"
        );
    }

    #[test]
    fn an_object_is_closed_and_lists_its_optional_properties() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: object\n    properties:\n      name: {{ type: string }}\n      email: {{ type: string, format: email }}\n    optional: [email]\n"
                ),
                "state.a"
            ),
            "z.object({\n  name: z.string(),\n  email: z.email().optional(),\n}).strict()"
        );
    }

    /// Grammar 3.6 makes a defaulted property implicitly optional; writing
    /// `.optional()` on top of `.default()` would answer an omitted property
    /// with `undefined` instead of the default.
    #[test]
    fn a_defaulted_property_is_not_also_written_optional() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: object\n    properties:\n      n: {{ type: integer, default: 0 }}\n    optional: [n]\n"
                ),
                "state.a"
            ),
            "z.object({\n  n: z.number().int().default(0),\n}).strict()"
        );
    }

    #[test]
    fn an_empty_field_map_is_a_closed_object_with_no_properties() {
        let source = "version: \"0.1\"\n\
tool.noop:\n  description: Does nothing.\n  input: {}\n  output: {}\n  exec:\n    command: \"true\"\n";
        assert_eq!(
            schema_of(source, "tool.noop.input"),
            "z.object({}).strict()"
        );
    }

    #[test]
    fn a_union_synthesizes_its_discriminator_as_a_literal() {
        let source = "version: \"0.1\"\n\
agent.triage:\n  model: model.m\n  prompt: p\n  output:\n    findings:\n      type: array\n      max_items: 3\n      items:\n        discriminator: kind\n        variants:\n          auto_fixable: { file: { type: string } }\n          needs_human: { summary: { type: string } }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n";
        assert_eq!(
            schema_of(source, "agent.triage.output"),
            "z.object({\n  \
               findings: z.array(z.discriminatedUnion(\"kind\", [\n    \
                 z.object({\n      kind: z.literal(\"auto_fixable\"),\n      file: z.string(),\n    }).strict(),\n    \
                 z.object({\n      kind: z.literal(\"needs_human\"),\n      summary: z.string(),\n    }).strict(),\n  \
               ])).max(3),\n\
             }).strict()"
        );
    }

    #[test]
    fn a_pattern_is_a_regex_literal_with_its_slashes_escaped() {
        assert_eq!(regex_expression("^a.*z$"), "/^a.*z$/");
        assert_eq!(regex_expression("a/b"), "/a\\/b/");
        assert_eq!(regex_expression("a\\/b"), "/a\\/b/");
        assert_eq!(regex_expression("a\\\\"), "/a\\\\/");
        assert_eq!(regex_expression("a\nb"), "new RegExp(\"a\\nb\")");
    }

    #[test]
    fn the_json_column_lowers_the_same_table() {
        let ir = ir_of(&format!(
            "{CHANNEL}  a:\n    type: object\n    properties:\n      name: {{ type: string, min_length: 1 }}\n      email: {{ type: string, format: email }}\n    optional: [email]\n"
        ));
        let surfaces = surfaces(&ir);
        let Body::Type(ty) = surfaces[0].body else {
            panic!("a channel is a type node");
        };
        assert_eq!(
            json_type_node(ty),
            json!({
                "type": "object",
                "properties": {
                    "name": { "type": "string", "minLength": 1 },
                    "email": { "type": "string", "format": "email" },
                },
                "required": ["name"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn a_union_lowers_to_one_of_with_a_const_tag() {
        let source = "version: \"0.1\"\n\
agent.triage:\n  model: model.m\n  prompt: p\n  output:\n    finding:\n      discriminator: kind\n      variants:\n        a: { x: { type: string } }\n        b: { y: { type: integer } }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n";
        let ir = ir_of(source);
        let surfaces = surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == "agent.triage.output")
            .expect("the agent declares an output");
        let Body::Fields(fields) = surface.body else {
            panic!("an output is a field map");
        };
        assert_eq!(
            json_field_map(fields),
            json!({
                "type": "object",
                "properties": {
                    "finding": {
                        "oneOf": [
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "a" }, "x": { "type": "string" } },
                                "required": ["kind", "x"],
                                "additionalProperties": false,
                            },
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "b" }, "y": { "type": "integer" } },
                                "required": ["kind", "y"],
                                "additionalProperties": false,
                            },
                        ]
                    }
                },
                "required": ["finding"],
                "additionalProperties": false,
            })
        );
    }

    #[test]
    fn every_surface_of_a_composition_is_enumerated_once_in_canonical_order() {
        let source = "version: \"0.1\"\n\
state:\n  draft: { type: string, default: \"\" }\n\
provider.p:\n  kind: anthropic\n  api_key: ${K}\n\
model.m:\n  provider: provider.p\n  id: some-model\n\
agent.a:\n  model: model.m\n  prompt: p\n  input: { goal: { type: string } }\n  output: { verdict: { enum: [ok] } }\n\
tool.t:\n  description: A tool.\n  input: {}\n  output: {}\n  exec: { command: \"true\" }\n\
store.s:\n  kind: kv\n  scope: execution\n  value_schema: { v: { type: string } }\n\
flow.f:\n  outputs: { draft: { type: string } }\n  nodes:\n    a: { agent: agent.a, input: { goal: \"'x'\" } }\n    ask:\n      human:\n        input: { q: { type: string } }\n        output: { answer: { type: string } }\n  edges:\n    - { from: start, to: a }\n    - { from: a, to: ask }\n    - { from: ask, to: end }\n";
        let ir = ir_of(source);
        assert_eq!(
            surfaces(&ir)
                .iter()
                .map(|surface| surface.path.as_str())
                .collect::<Vec<_>>(),
            [
                "agent.a.input",
                "agent.a.output",
                "flow.f.outputs",
                "flow.f.node.ask.input",
                "flow.f.node.ask.output",
                "store.s.value_schema",
                "tool.t.input",
                "tool.t.output",
                "state.draft",
            ]
        );
    }
}
