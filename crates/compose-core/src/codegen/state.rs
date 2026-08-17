//! `src/state.ts`: the graph's state model (PRD 5.7 tier 2, grammar 10).
//!
//! # One channel per declaration, plus one that is never declared
//!
//! Every `state:` channel becomes a LangGraph channel whose value type is the
//! Zod schema `src/schemas.ts` declares for it, and `messages` — the implicit
//! conversation history of grammar 10.4, which MUST NOT be declared and is
//! therefore never in the IR — is emitted alongside them. A state model without
//! it would be missing the tier PRD 5.7 counts third.
//!
//! # Reduce policies (grammar 10.2)
//!
//! | `reduce:` | channel | initial value | one write supplies |
//! |---|---|---|---|
//! | *absent* | `Annotation<T>` — LangGraph's `LastValue` | unset, or the declared `default:` | the whole value |
//! | `last_wins` | reducer `(_left, right) => right` | unset, or the declared `default:` | the whole value |
//! | `append` | reducer `(left, right) => left.concat([right])` | `[]`, or the declared `default:` | **one element** (Decision D58) |
//! | `merge` | reducer `(left, right) => ({ …left, …right })` | `{}`, or the declared `default:` | a partial object |
//!
//! Two of those rows carry a decision worth stating outright:
//!
//! * **An unreduced channel with no `default:` is `Annotation<T>`, and one with a
//!   `default:` is not.** LangGraph's `LastValue` refuses two writes in one step,
//!   which is exactly the single-writer rule grammar 10.2 states — but it has no
//!   initial-value form, and a channel's `default:` is its initial value
//!   (grammar 10.1). So a defaulted unreduced channel is a reducer that keeps the
//!   right-hand side. Nothing is lost: writing an unreduced channel from
//!   concurrent contexts is a **compile** error, so the runtime guard is a second
//!   line of defence rather than the rule itself.
//! * **A `merge` channel's value type is `Partial<…>`.** Grammar 10.1 is explicit
//!   that a `merge` channel is the one form whose value need not validate against
//!   its own declared type while the flow runs: it holds `{}` before the first
//!   write and a subset after a partial one, and reading a property it does not
//!   hold fails the execution (Decision D101). `Partial<…>` is that fact in the
//!   type system; a total type here would be a lie the compiler tells about a
//!   value the runtime may not have.
//!
//! # One name the channel table cannot hold
//!
//! The emitted `channels` is a plain object literal keyed by the channel's own
//! name, and LangGraph reads it with a plain property lookup — `StateGraph`'s
//! `_addSchema` asks `this.channels[key] !== undefined` before installing a
//! channel, over a `this.channels` that is itself `{}`. A key every JavaScript
//! object already answers therefore reads as *already installed*, and the next
//! line calls `.equals(…)` on whatever `Object.prototype` handed back:
//!
//! ```text
//! TypeError: this.channels[key].equals is not a function
//!     at StateGraph._addSchema (@langchain/langgraph/dist/graph/state.js:333)
//! ```
//!
//! Those names are [`INHERITED_PROPERTY_NAMES`], and grammar 2.5 reserves none
//! of them — `constructor` is a channel name the validator is right to accept
//! and this target cannot express, so [`super::diagnostics`] refuses the build
//! rather than emitting a project that type-checks, loads, and cannot construct
//! its own graph. Renaming is the only fix available here: grammar 10.3 wires a
//! write by the channel's own name, so the emitter has no second spelling to
//! reach for.
//!
//! # Ordering
//!
//! A reducer sees writes one at a time and can only be as ordered as the caller.
//! The canonical write order of grammar 7.6.4 — writers of a step by node id, a
//! `map`'s instances by source-item index — is imposed by the pass that *applies*
//! the writes, which is a later M1 bullet. What is fixed here is that each policy
//! is **order-faithful**: `append` appends in the order it is called, `merge`
//! lets the last call win a key, `last_wins` keeps the last call. A reducer that
//! sorted or deduplicated on its own would make the canonical order unobservable
//! and unfixable.

use crate::ast::document::Reduce;
use crate::ir::Ir;
use crate::ir::schema::TypeForm;

use super::names::{self, Names};
use super::schema;

/// Every property name a JavaScript object carries without being given one:
/// the own properties of `Object.prototype`, sorted.
///
/// A channel whose name is one of these cannot be a key of the emitted
/// `channels` object [`module`] emits — see the module docs for the mechanism, and
/// [`super::diagnostics`] for the refusal. The whole list is here rather than
/// only the reachable part of it, because the rule is prototype lookup rather
/// than spelling; grammar 2.1's identifier (`lower , { lower | digit | "_" }`)
/// admits exactly one of them, `constructor`, which is why that is the one
/// every test names.
///
/// `tests/generated_code_gates.rs`'
/// `a_channel_named_after_an_inherited_property_cannot_be_built_at_all` is what
/// keeps this list honest: it asks the pinned Node for
/// `Object.getOwnPropertyNames(Object.prototype)` and then makes LangGraph fail
/// on each one, so a list that grew stale — or a LangGraph release that stopped
/// caring — is a red test rather than a silent over-refusal.
pub const INHERITED_PROPERTY_NAMES: &[&str] = &[
    "__defineGetter__",
    "__defineSetter__",
    "__lookupGetter__",
    "__lookupSetter__",
    "__proto__",
    "constructor",
    "hasOwnProperty",
    "isPrototypeOf",
    "propertyIsEnumerable",
    "toLocaleString",
    "toString",
    "valueOf",
];

/// Whether every JavaScript object already answers to this name.
#[must_use]
pub fn inherited_property_name(name: &str) -> bool {
    INHERITED_PROPERTY_NAMES.contains(&name)
}

/// `src/state.ts`.
#[must_use]
pub fn module(ir: &Ir, names: &Names) -> super::GeneratedFile {
    let channels = schema::channels(ir);

    let mut contents = super::header(ir, "// ");
    contents.push_str(MODULE_DOC);
    contents
        .push_str("\nimport { Annotation, MessagesAnnotation } from \"@langchain/langgraph\";\n");
    if !channels.is_empty() {
        contents.push_str("import type { z } from \"zod\";\n\n");
        contents.push_str("import {\n");
        for (path, _) in &channels {
            contents.push_str(&format!("  {},\n", names.value(path)));
        }
        contents.push_str("} from \"./schemas.ts\";\n");
    }

    contents.push('\n');
    contents.push_str(&names::doc(
        "",
        &[
            "Every channel of the graph's state, keyed by the name a write targets".to_string(),
            "(grammar 10.3's name-based wiring).".to_string(),
        ],
    ));
    contents.push_str("export const channels = {\n");
    for (path, channel) in &channels {
        let schema = names.value(path);
        let name = channel.name.value.as_str();
        contents.push_str(&names::doc("  ", &describe(channel)));
        contents.push_str(&format!("  {name}: {},\n", annotation(channel, schema)));
    }
    contents.push_str(&names::doc("  ", &MESSAGES_DOC.map(String::from)));
    contents.push_str("  messages: MessagesAnnotation.spec.messages,\n");
    contents.push_str("};\n");

    contents.push_str(TAIL);

    super::GeneratedFile {
        path: "src/state.ts".to_string(),
        contents,
    }
}

const MODULE_DOC: &str = "\
//
// The graph's state model (PRD 5.7 tier 2, grammar 10). Each declared `state:`
// channel is one LangGraph channel typed by its schema in `./schemas.ts`, with
// the reducer its `reduce:` policy names and the initial value its `default:`
// names. `messages` is the implicit conversation history of grammar 10.4, which
// a composition never declares and every graph has.
";

const MESSAGES_DOC: [&str; 2] = [
    "The implicit conversation history (grammar 10.4, PRD 5.7 tier 3): append-only,",
    "never declared in `state:`, and isolated across flow boundaries by default.",
];

const TAIL: &str = r#"
/** The state schema the graph is built from. */
export const State = Annotation.Root(channels);

/** What a node reads: every channel, at its declared type. */
export type GraphState = typeof State.State;

/** What a node writes: any subset of the channels, at their update types. */
export type GraphStateUpdate = typeof State.Update;
"#;

/// The doc comment above one channel: what it is, how it reduces, and what it
/// starts at.
fn describe(channel: &crate::ir::Channel) -> Vec<String> {
    let mut lines = Vec::new();
    if let Some(description) = &channel.ty.description {
        lines.push(description.value.clone());
        lines.push(String::new());
    }
    lines.push(match channel.reduce {
        None => "Unreduced: single-writer, and a write supplies the whole value.".to_string(),
        Some(Reduce::LastWins) => {
            "`reduce: last_wins`: concurrent overwrite, declared explicitly; a write \
             supplies the whole value."
                .to_string()
        }
        Some(Reduce::Append) => {
            "`reduce: append`: a write supplies **one element**, appended in canonical \
             write order (grammar 7.6.4, Decision D58)."
                .to_string()
        }
        Some(Reduce::Merge) => {
            "`reduce: merge`: a write supplies a subset of the properties, merged \
             key-wise in canonical write order — so the value is `Partial` until \
             every property has been supplied (grammar 10.1, Decision D101)."
                .to_string()
        }
    });
    lines.push(match initial(channel) {
        Some(value) => format!("Starts at `{value}`."),
        None => "Starts **unset**: reading it before its first write fails the execution \
                 (Decision D78)."
            .to_string(),
    });
    lines
}

/// The channel's initial value as TypeScript source, or `None` when it starts
/// unset.
fn initial(channel: &crate::ir::Channel) -> Option<String> {
    let declared = match &channel.ty.form {
        TypeForm::Scalar(scalar) => scalar.default.as_ref(),
        TypeForm::Enum(enumeration) => enumeration.default.as_ref(),
        TypeForm::Object(object) => object.default.as_ref(),
        TypeForm::Array(array) => array.default.as_ref(),
        TypeForm::Union(_) => None,
    };
    if let Some(default) = declared {
        return Some(names::literal(&default.value));
    }
    // The identity element of the reduce, which is what makes a fan-out that
    // produced zero items read as "nothing yet" (grammar 10.1).
    match channel.reduce {
        Some(Reduce::Append) => Some("[]".to_string()),
        Some(Reduce::Merge) => Some("{}".to_string()),
        Some(Reduce::LastWins) | None => None,
    }
}

/// The `Annotation<…>` expression for one channel.
fn annotation(channel: &crate::ir::Channel, schema: &str) -> String {
    let value = format!("z.infer<typeof {schema}>");
    let initial = initial(channel);

    let (types, reducer) = match channel.reduce {
        Some(Reduce::Append) => (
            format!("{value}, {value}[number]"),
            "(left, right) => left.concat([right])",
        ),
        Some(Reduce::Merge) => (
            format!("Partial<{value}>, Partial<{value}>"),
            "(left, right) => ({ ...left, ...right })",
        ),
        Some(Reduce::LastWins) | None => (value.clone(), "(_left, right) => right"),
    };

    // An unreduced channel with no initial value is LangGraph's `LastValue`,
    // which refuses two writes in one step — the single-writer rule of
    // grammar 10.2, enforced at run time as well as at compile time.
    if channel.reduce.is_none() && initial.is_none() {
        return format!("Annotation<{value}>");
    }

    let mut text = format!("Annotation<{types}>({{\n    reducer: {reducer},\n");
    if let Some(initial) = initial {
        text.push_str(&format!("    default: () => {},\n", factory(&initial)));
    }
    text.push_str("  })");
    text
}

/// A default factory's body, parenthesized where an object literal would
/// otherwise be read as a block.
fn factory(value: &str) -> String {
    if value.starts_with('{') {
        format!("({value})")
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    fn state_of(channels: &str) -> String {
        let ir = ir_of(&format!("version: \"0.1\"\nstate:\n{channels}"));
        let names = Names::of(&ir);
        module(&ir, &names).contents
    }

    #[test]
    fn an_unreduced_channel_with_no_default_is_langgraphs_last_value() {
        let emitted = state_of("  draft: { type: string }\n");
        assert!(
            emitted.contains("  draft: Annotation<z.infer<typeof stateDraft>>,\n"),
            "{emitted}"
        );
        assert!(emitted.contains("Starts **unset**"), "{emitted}");
    }

    #[test]
    fn an_unreduced_channel_with_a_default_starts_at_it() {
        let emitted = state_of("  draft: { type: string, default: \"\" }\n");
        assert!(
            emitted.contains(
                "  draft: Annotation<z.infer<typeof stateDraft>>({\n    \
                 reducer: (_left, right) => right,\n    \
                 default: () => \"\",\n  }),\n"
            ),
            "{emitted}"
        );
    }

    /// Grammar 10.2 and Decision D58: one element per write, which is what makes
    /// a `map`'s fan-in work.
    #[test]
    fn an_append_channel_takes_one_element_per_write_and_starts_empty() {
        let emitted = state_of(
            "  notes:\n    type: array\n    items: { type: string }\n    reduce: append\n",
        );
        assert!(
            emitted.contains(
                "  notes: Annotation<z.infer<typeof stateNotes>, z.infer<typeof stateNotes>[number]>({\n    \
                 reducer: (left, right) => left.concat([right]),\n    \
                 default: () => [],\n  }),\n"
            ),
            "{emitted}"
        );
    }

    #[test]
    fn a_merge_channel_is_partial_and_starts_at_the_empty_object() {
        let emitted = state_of(
            "  totals:\n    type: object\n    properties: { fixed: { type: integer } }\n    reduce: merge\n",
        );
        assert!(
            emitted.contains(
                "  totals: Annotation<Partial<z.infer<typeof stateTotals>>, Partial<z.infer<typeof stateTotals>>>({\n    \
                 reducer: (left, right) => ({ ...left, ...right }),\n    \
                 default: () => ({}),\n  }),\n"
            ),
            "{emitted}"
        );
    }

    /// A declared `default:` is the channel's initial value, and it beats the
    /// reduce policy's identity element (grammar 10.1).
    #[test]
    fn a_declared_default_beats_the_policys_identity_element() {
        let emitted = state_of(
            "  totals:\n    type: object\n    properties: { fixed: { type: integer } }\n    reduce: merge\n    default: { fixed: 0 }\n",
        );
        assert!(
            emitted.contains("default: () => ({ \"fixed\": 0 }),"),
            "{emitted}"
        );
    }

    #[test]
    fn a_last_wins_channel_declares_the_overwrite_it_opts_into() {
        let emitted = state_of("  draft: { type: string, reduce: last_wins }\n");
        assert!(
            emitted.contains(
                "  draft: Annotation<z.infer<typeof stateDraft>>({\n    \
                 reducer: (_left, right) => right,\n  }),\n"
            ),
            "{emitted}"
        );
        assert!(emitted.contains("`reduce: last_wins`"), "{emitted}");
    }

    /// Grammar 10.4: the history channel is never declared and every graph has
    /// it — including one whose `state:` section is absent entirely.
    #[test]
    fn the_conversation_history_channel_is_emitted_without_being_declared() {
        let ir = ir_of("version: \"0.1\"\n");
        let names = Names::of(&ir);
        let emitted = module(&ir, &names).contents;
        assert!(
            emitted.contains("  messages: MessagesAnnotation.spec.messages,\n"),
            "{emitted}"
        );
        assert!(
            !emitted.contains("from \"./schemas.ts\""),
            "a composition with no channels imports no channel schema: {emitted}"
        );
    }
}
