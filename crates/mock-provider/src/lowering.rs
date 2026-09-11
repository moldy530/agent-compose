//! The **subset of JSON Schema** each structured-output decoder compiles, and
//! the 400 a request outside it draws (PRD §9 resolved q55).
//!
//! The oracle half of the ruling. A compiled graph projects the schema it sends
//! through a per-(wire, mechanism) lowering table before the request — the
//! constraint keywords that decoder cannot compile are stripped and folded into
//! descriptions — and a table is only worth having if something checks it. So
//! this server enforces the same subsets from the other side: a schema arriving
//! with a keyword the decoder does not take is refused, in the surface's own
//! words, exactly as the live wire refuses it.
//!
//! That is what makes CI prove the projection rather than merely exercise it.
//! An **under-lowered** schema — a table row this compiler forgot — fails the
//! acceptance suite here instead of on somebody's first live call, which is the
//! failure q55 exists to close: grammar D10 makes `max_items` REQUIRED on every
//! result-schema array, §3.5 sends it to the wire as `maxItems`, and the
//! Messages wire's native decoder does not compile it, so nearly every real
//! composition drew a 400 on the rung resolved q53 prefers.
//!
//! # Why the table is stated twice
//!
//! [`ENFORCED`] here and `LOWERED_AWAY` in
//! `crates/compose-core/src/codegen/js/runtime.ts` are the same four rows in two
//! languages, and `crates/agent-compose/tests/wire_lowering_agreement.rs`
//! asserts they are equal. Two statements rather than one shared constant is
//! deliberate: this crate stands in for a *vendor*, and a mock that read the
//! compiler's own table could only ever agree with it. What the equality test
//! buys is that an edit to one side alone fails loudly — the compiler's
//! projection and the wire it is a projection of cannot drift apart quietly.
//!
//! # What is *not* enforced
//!
//! The Messages wire's **forced-tool** rung, whose row is deliberately empty: a
//! schema rides there as an ordinary tool's `input_schema`, which the API takes
//! whole. An empty row is a claim like any other, and this module makes it: a
//! `maxItems` inside a pinned tool's `input_schema` is *accepted* here.

use serde_json::{Map, Value};

use crate::control::{OutputMechanism, Surface};
use crate::strict::{Checker, Dialect, at};

/// The keywords the **Messages** wire's `output_config` format does not compile.
///
/// Array-length, numeric and string-length constraints — the three families PRD
/// §9 resolved q55 names, in the spelling grammar §3.8 lowers them to.
/// Recursion, the fourth thing the vendor's subset excludes, is unreachable from
/// this grammar at all (§3 makes `$ref` a compile error and §3.4 caps nesting at
/// 8), so there is nothing here for it.
const ANTHROPIC_NATIVE: &[&str] = &[
    "maxItems",
    "minItems",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
];

/// …and the keywords the **OpenAI** wires' structured-output decoder does not
/// compile, wherever the schema was declared.
///
/// `response_format`'s `json_schema`, Responses' `text.format` and a forced
/// function's `parameters` are three spellings of one decoder, so one list
/// serves all of them. `uniqueItems` is here and is not in the Anthropic list
/// above: these are two vendors' subsets rather than one shared list, and
/// OpenAI's guide names it among the array keywords its decoder refuses.
const OPENAI_STRICT: &[&str] = &[
    "maxItems",
    "minItems",
    "uniqueItems",
    "minimum",
    "maximum",
    "exclusiveMinimum",
    "exclusiveMaximum",
    "multipleOf",
    "minLength",
    "maxLength",
];

/// Nothing: the row for a mechanism whose decoder compiles whatever it is given.
const ACCEPTS_EVERYTHING: &[&str] = &[];

/// Where a subschema hides, by the shape of the position it hides in — the
/// positions holding **one** subschema.
///
/// The same three lists the generated runtime's projection walks
/// (`SUBSCHEMA_KEYS`, `SUBSCHEMA_LIST_KEYS`, `SUBSCHEMA_MAP_KEYS` in
/// `crates/compose-core/src/codegen/js/runtime.ts`), and
/// `crates/agent-compose/tests/wire_lowering_agreement.rs` asserts they are
/// equal — for the reason it asserts the keyword tables are: a position the
/// runtime lowers and this check does not look at is a keyword that reaches the
/// wire with nothing to report it, and a position this looks at and the runtime
/// does not lower is a refusal no projection can repair. Only `properties`,
/// `items` and `oneOf` are reachable from grammar §3 today; the rest are here
/// because the cost of listing one is a line and the cost of missing one is a
/// silent disagreement between a projection and its oracle.
///
/// `additionalProperties` is in none of the three: it is a boolean on every
/// object this compiler emits (grammar 3.4 closes them) and a subschema in JSON
/// Schema at large, so both walks reach it only where it is one — a guard rather
/// than a position, written out on both sides.
pub const SUBSCHEMA_POSITIONS: &[&str] = &[
    "items",
    "contains",
    "not",
    "if",
    "then",
    "else",
    "propertyNames",
    "additionalItems",
    "unevaluatedItems",
];

/// …the positions holding a **list** of subschemas.
pub const SUBSCHEMA_LIST_POSITIONS: &[&str] = &["oneOf", "anyOf", "allOf", "prefixItems"];

/// …and the positions holding a **map** of names to subschemas.
pub const SUBSCHEMA_MAP_POSITIONS: &[&str] = &[
    "properties",
    "patternProperties",
    "dependentSchemas",
    "$defs",
    "definitions",
];

/// What each (surface, mechanism) **refuses**, one row at a time.
///
/// Read by [`enforced`], and by the acceptance suite's equality test against the
/// generated runtime's own table. A row is one line, which is the point: a
/// vendor that starts compiling a keyword tomorrow is an edit here and an edit
/// there, and nothing else moves (resolved q30's treadmill terms).
const ENFORCED: &[(Surface, OutputMechanism, &[&str])] = &[
    (
        Surface::Anthropic,
        OutputMechanism::Native,
        ANTHROPIC_NATIVE,
    ),
    (
        Surface::Anthropic,
        OutputMechanism::ForcedTool,
        ACCEPTS_EVERYTHING,
    ),
    (Surface::OpenAi, OutputMechanism::Native, OPENAI_STRICT),
    (Surface::OpenAi, OutputMechanism::ForcedTool, OPENAI_STRICT),
    (Surface::AzureOpenAi, OutputMechanism::Native, OPENAI_STRICT),
    (
        Surface::AzureOpenAi,
        OutputMechanism::ForcedTool,
        OPENAI_STRICT,
    ),
    (Surface::Responses, OutputMechanism::Native, OPENAI_STRICT),
    (
        Surface::Responses,
        OutputMechanism::ForcedTool,
        OPENAI_STRICT,
    ),
];

/// The keywords one endpoint refuses inside a structured-output schema asked for
/// this way (PRD §9 resolved q55).
///
/// Public because the agreement test reads it: the generated runtime's lowering
/// table for the same (wire, mechanism) must be exactly this list.
#[must_use]
pub fn enforced(surface: Surface, mechanism: OutputMechanism) -> &'static [&'static str] {
    ENFORCED
        .iter()
        .find(|(row, asked, _)| *row == surface && *asked == mechanism)
        .map_or(ACCEPTS_EVERYTHING, |(_, _, keywords)| *keywords)
}

/// Every (surface, mechanism) row, for a test that has to walk them all.
#[must_use]
pub fn rows() -> &'static [(Surface, OutputMechanism, &'static [&'static str])] {
    ENFORCED
}

/// Refuse a structured-output schema carrying a keyword this decoder does not
/// compile — one complaint per keyword, in the surface's own dialect.
///
/// `pointer` is the harness's address of the schema in the request
/// (`output_config.format.schema`, `response_format.json_schema.schema`,
/// `tools.1.function.parameters`); `subject` is what the surface calls the thing
/// being compiled, which is the half its message quotes.
///
/// Every offending node is reported rather than the first, for the reason
/// [`Checker`] reports every mistake: a lowering table missing a row usually
/// shows up as several keywords at once, and one round trip naming all of them
/// is worth more than five that each name one.
pub(crate) fn check_schema(
    checker: &mut Checker,
    dialect: Dialect,
    surface: Surface,
    mechanism: OutputMechanism,
    pointer: &str,
    subject: &str,
    schema: &Value,
) {
    let keywords = enforced(surface, mechanism);
    if keywords.is_empty() {
        return;
    }
    walk(
        checker,
        dialect,
        keywords,
        pointer,
        subject,
        &mut Vec::new(),
        schema,
    );
}

/// …and the same subset over a **client tool** that promises `strict: true`.
///
/// `strict` on a function hands its `parameters` to the very compiler
/// `response_format` and `text.format` hand their schema to — one decoder, three
/// spellings — so a keyword it will not take is refused in an ordinary tool's
/// schema exactly as it is in a pinned one, and the row asked for is the
/// forced-function one, whose schema *is* a function's `parameters`.
///
/// It is enforced separately because the schema arrives separately: grammar §3.5
/// lets a `tool.…` `input:` declare `min_length`, `minimum` or `max_items`, and a
/// compiled graph puts every declared tool on every Responses request it sends,
/// pinned or not. Without this, the one strict surface a real composition
/// reaches on every call would be the one the oracle never checked.
///
/// A schema promised at `strict: false` is not checked, here or on the live
/// wire: the decoder compiles nothing it was not asked to be strict about, so
/// nothing in it can be a keyword the decoder refuses.
pub(crate) fn check_strict_tool(
    checker: &mut Checker,
    dialect: Dialect,
    surface: Surface,
    pointer: &str,
    subject: &str,
    schema: &Value,
) {
    check_schema(
        checker,
        dialect,
        surface,
        OutputMechanism::ForcedTool,
        pointer,
        subject,
        schema,
    );
}

fn walk(
    checker: &mut Checker,
    dialect: Dialect,
    keywords: &[&str],
    pointer: &str,
    subject: &str,
    context: &mut Vec<String>,
    schema: &Value,
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    for keyword in keywords {
        if !object.contains_key(*keyword) {
            continue;
        }
        let address = address(pointer, context, keyword);
        let message = match dialect {
            // The Messages API validates with pydantic and addresses its
            // complaints at the path it stopped on, which is the dialect
            // `crate::strict` writes for every other refusal on this surface.
            // The sentence names the keyword and the parameter, because those
            // are the two things a client has to know to repair the request.
            Dialect::Anthropic => format!(
                "{address}: '{keyword}' is not supported. The `output_config` format compiles a \
                 subset of JSON Schema and this keyword is not in it."
            ),
            // …and OpenAI's own shape for a schema its decoder will not take,
            // `context=` tuple and all — the same wording
            // `crate::openai::check_strict_schema` writes for the other half of
            // the subset.
            Dialect::OpenAi => format!(
                "Invalid schema for {subject}: In context={}, '{keyword}' is not permitted.",
                printed(context)
            ),
        };
        checker.fail(&address, message);
    }

    // Every position a subschema hides in — read off the three lists rather
    // than spelled out here, so that what this check looks at and what the
    // generated runtime's projection lowers are one statement compared by
    // `wire_lowering_agreement.rs` rather than two sets of match arms nothing
    // holds together.
    for (key, member) in object {
        let key = key.as_str();
        if SUBSCHEMA_POSITIONS.contains(&key)
            || (key == "additionalProperties" && member.is_object())
        {
            context.push(key.to_string());
            walk(
                checker, dialect, keywords, pointer, subject, context, member,
            );
            context.pop();
        } else if SUBSCHEMA_LIST_POSITIONS.contains(&key) {
            for (index, branch) in member.as_array().into_iter().flatten().enumerate() {
                context.push(key.to_string());
                context.push(index.to_string());
                walk(
                    checker, dialect, keywords, pointer, subject, context, branch,
                );
                context.truncate(context.len() - 2);
            }
        } else if SUBSCHEMA_MAP_POSITIONS.contains(&key) {
            for (name, member) in member.as_object().into_iter().flatten() {
                context.push(key.to_string());
                context.push(name.clone());
                walk(
                    checker, dialect, keywords, pointer, subject, context, member,
                );
                context.truncate(context.len() - 2);
            }
        }
    }
}

/// The harness's dotted address of one offending keyword.
fn address(pointer: &str, context: &[String], keyword: &str) -> String {
    let mut address = pointer.to_string();
    for member in context {
        address = at(&address, member);
    }
    at(&address, keyword)
}

/// A schema path as OpenAI prints it: a Python tuple, `()` at the root.
///
/// The same rendering `crate::openai::check_strict_schema` uses, written here
/// rather than shared because the two are one surface's dialect and this module
/// is the only other place that speaks it.
fn printed(context: &[String]) -> String {
    let members = context
        .iter()
        .map(|member| format!("'{member}'"))
        .collect::<Vec<_>>()
        .join(", ");
    match context.len() {
        0 => "()".to_string(),
        1 => format!("({members},)"),
        _ => format!("({members})"),
    }
}

/// Whether this object declares any keyword this endpoint refuses — the
/// question a test asks of a *recorded* request, without the checker.
#[must_use]
pub fn carries_refused_keyword(
    surface: Surface,
    mechanism: OutputMechanism,
    schema: &Value,
) -> Option<String> {
    let keywords = enforced(surface, mechanism);
    found(keywords, schema)
}

fn found(keywords: &[&str], schema: &Value) -> Option<String> {
    match schema {
        Value::Object(object) => {
            for keyword in keywords {
                if object.contains_key(*keyword) {
                    return Some((*keyword).to_string());
                }
            }
            object.values().find_map(|member| found(keywords, member))
        }
        Value::Array(members) => members.iter().find_map(|member| found(keywords, member)),
        _ => None,
    }
}

/// Where in a request's `tools` array the pinned tool sits, so a complaint about
/// its schema is addressed at the entry that carried it.
///
/// `named` reads the name off one entry, which is the one thing the three wires
/// spell differently: flat on the Messages and Responses surfaces, nested under
/// `function` on Chat Completions.
pub(crate) fn tool_index(
    body: &Map<String, Value>,
    name: &str,
    named: fn(&Value) -> Option<&str>,
) -> Option<usize> {
    body.get("tools")?
        .as_array()?
        .iter()
        .position(|tool| named(tool) == Some(name))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The four (wire, mechanism) rows the ruling names, and the one that is
    /// deliberately empty.
    #[test]
    fn each_mechanism_refuses_its_own_subset() {
        assert!(enforced(Surface::Anthropic, OutputMechanism::Native).contains(&"maxItems"));
        assert!(
            enforced(Surface::Anthropic, OutputMechanism::ForcedTool).is_empty(),
            "a schema rides the Messages forced-tool rung as a tool's `input_schema`, whole"
        );
        assert!(enforced(Surface::OpenAi, OutputMechanism::Native).contains(&"uniqueItems"));
        assert!(enforced(Surface::Responses, OutputMechanism::ForcedTool).contains(&"maxItems"));
        assert!(
            !enforced(Surface::Anthropic, OutputMechanism::Native).contains(&"uniqueItems"),
            "the two subsets are two vendors', not one shared list"
        );
        assert!(
            !enforced(Surface::OpenAi, OutputMechanism::Native).contains(&"pattern"),
            "`pattern` is left on the wire on purpose (PRD resolved q55: under-stripping is the \
             direction the tables err in)"
        );
    }

    /// A keyword nested two levels down is found, and named at its own address.
    #[test]
    fn a_refused_keyword_is_reported_wherever_it_nests() {
        let schema = json!({
            "type": "object",
            "properties": {
                "notes": {
                    "type": "array",
                    "maxItems": 8,
                    "items": { "type": "string", "minLength": 2 },
                },
            },
        });
        let mut checker = Checker::new(Dialect::Anthropic);
        check_schema(
            &mut checker,
            Dialect::Anthropic,
            Surface::Anthropic,
            OutputMechanism::Native,
            "output_config.format.schema",
            "output_config",
            &schema,
        );
        let failures = checker.into_failures();
        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            [
                "output_config.format.schema.properties.notes.maxItems",
                "output_config.format.schema.properties.notes.items.minLength",
            ]
        );
        assert_eq!(
            failures[0].message,
            "output_config.format.schema.properties.notes.maxItems: 'maxItems' is not supported. \
             The `output_config` format compiles a subset of JSON Schema and this keyword is not \
             in it."
        );
    }

    /// …and the OpenAI dialect words the same finding its own way.
    #[test]
    fn the_openai_dialect_names_the_context_it_stopped_in() {
        let schema = json!({
            "type": "object",
            "properties": {
                "kinds": { "anyOf": [{ "type": "array", "maxItems": 2 }] },
            },
        });
        let mut checker = Checker::new(Dialect::OpenAi);
        check_schema(
            &mut checker,
            Dialect::OpenAi,
            Surface::OpenAi,
            OutputMechanism::Native,
            "response_format.json_schema.schema",
            "response_format 'reviewer_output'",
            &schema,
        );
        let failures = checker.into_failures();
        assert_eq!(failures.len(), 1);
        assert_eq!(
            failures[0].message,
            "Invalid schema for response_format 'reviewer_output': In context=('properties', \
             'kinds', 'anyOf', '0'), 'maxItems' is not permitted."
        );
    }

    /// A schema the projection already lowered is accepted, on every row.
    #[test]
    fn a_lowered_schema_is_accepted_everywhere() {
        let lowered = json!({
            "type": "object",
            "properties": {
                "notes": {
                    "type": "array",
                    "description": "Side notes. At most 8 items.",
                    "items": { "type": "string" },
                },
            },
            "required": ["notes"],
            "additionalProperties": false,
        });
        for (surface, mechanism, _) in rows() {
            let dialect = if *surface == Surface::Anthropic {
                Dialect::Anthropic
            } else {
                Dialect::OpenAi
            };
            let mut checker = Checker::new(dialect);
            check_schema(
                &mut checker,
                dialect,
                *surface,
                *mechanism,
                "schema",
                "the schema",
                &lowered,
            );
            assert!(
                checker.into_failures().is_empty(),
                "a lowered schema was refused on {surface:?}/{mechanism:?}"
            );
        }
    }

    /// The Messages forced-tool rung takes the schema whole — the empty row,
    /// asserted as the claim it is.
    #[test]
    fn the_messages_forced_tool_rung_takes_the_whole_schema() {
        let schema = json!({ "type": "array", "maxItems": 8, "items": { "type": "string" } });
        let mut checker = Checker::new(Dialect::Anthropic);
        check_schema(
            &mut checker,
            Dialect::Anthropic,
            Surface::Anthropic,
            OutputMechanism::ForcedTool,
            "tools.0.input_schema",
            "the tool",
            &schema,
        );
        assert!(checker.into_failures().is_empty());
        assert_eq!(
            carries_refused_keyword(Surface::Anthropic, OutputMechanism::Native, &schema)
                .as_deref(),
            Some("maxItems"),
            "…and the same schema on the native rung is refused"
        );
    }
}
