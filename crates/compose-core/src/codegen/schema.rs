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
//! Grammar 3.8 fixes eight rows and the snake_case → camelCase key rule. The
//! rest of the vocabulary of grammar 3.3–3.7 is not in the table, so the choices
//! below are this module's, made to keep the two columns *observably equivalent*
//! (that is what the conformance corpus checks) rather than merely plausible:
//!
//! | DSL | JSON Schema | Zod | why |
//! |---|---|---|---|
//! | `description` | `description` | `.describe(…)` | it reaches the model in a structured-output schema (grammar 3.2) |
//! | `min_length`/`max_length` | `minLength`/`maxLength` | `.refine(…)` over a code-point count | both keywords count Unicode code points and `.min()`/`.max()` count UTF-16 code units — see below |
//! | `exclusive_minimum` | `exclusiveMinimum` | `.gt(…)` | Zod spells the exclusive bounds `gt`/`lt` |
//! | `multiple_of` | `multipleOf` | `.multipleOf(…)` | |
//! | `min_items` | `minItems` | `.min(…)` | |
//! | `unique_items` | `uniqueItems` | `.refine(uniqueItems, …)` | Zod has no built-in; the emitted helper compares JSON encodings of the *parsed* elements, which decides the same question the keyword asks everywhere a `default:` does not fill one in — see below |
//! | `optional: [b]` **and** `default:` on `b` | `b` omitted from `required` | `.default(v)` alone | grammar 3.6 makes a defaulted property implicitly optional, and `.default(v).optional()` would answer `undefined` for an omitted property instead of the default — the one composition where the two orderings differ |
//!
//! Two spellings are the table's own and are kept verbatim even where Zod offers
//! a newer one: a closed object is `z.object({…}).strict()` (not
//! `z.strictObject`), and an integer is `z.number().int()` (not `z.int()`). The
//! grammar is normative and both pairs denote the same schema.
//!
//! # `min_length`/`max_length`, and what one character is
//!
//! JSON Schema counts a string's length in Unicode **code points**; `.min()` and
//! `.max()` count `String.prototype.length`, which is UTF-16 **code units**.
//! Every character outside the BMP is one of the first and two of the second, so
//! the two columns disagree in *both* directions on any document carrying one:
//! `"😀"` is within `max_length: 1` and below `min_length: 2` to JSON Schema, and
//! the other way about to `.min()`/`.max()`. That is reachable straight from a
//! model-facing surface — `agent.output: { title: { type: string, max_length:
//! 200 } }` constrains the provider by the code-point rule, and an emitted parse
//! counting code units would refuse the answer it asked for.
//!
//! So the bound is written out: `.refine((value) => codePoints(value) <= 200, …)`
//! over [`HELPERS`]' `codePoints`, which spreads the string and so iterates it by
//! code point. Like six of the ten formats below, that puts the check where
//! `z.toJSONSchema` cannot see it (see *What a provider is handed*); a check that
//! is visible and measures the wrong thing is the worse of the two. `state.mark`
//! in the corpus is the pair of documents that tells the two spellings apart.
//!
//! # `unique_items`, and the one row that leans on its surroundings
//!
//! JSON Schema's `uniqueItems` compares **instances**: two objects are one item
//! when they carry the same keys with equal values, whatever order those keys
//! were written in. The emitted `uniqueItems` compares `JSON.stringify`
//! encodings, and an object's encoding follows its key order — a different rule,
//! which on raw input answers differently (`[{a: 1, b: 2}, {b: 2, a: 1}]` is
//! unique to the encoding rule and a duplicate to JSON Schema's).
//!
//! It is nevertheless the same *decision* in the position it is emitted into,
//! and for a reason worth stating because nothing about the helper says it:
//! `.refine` runs over the array's already-**parsed** elements, and
//! `z.object({…}).strict()` rebuilds each object in the order its own shape
//! declares. Two equal instances therefore reach the helper with identical
//! encodings. `state.authors` in the corpus pins that decision from raw text
//! (`as_written`, so the key order a document was written in survives the
//! corpus's own round trip), which is what would fail if a pinned Zod release
//! ever stopped normalizing — the alternative, a canonicalizing helper in every
//! generated project, would be a rule no gate could tell apart from this one.
//!
//! Key order is only half of what the argument needs, though: the rule also
//! wants two *unequal* instances to reach the helper with **different**
//! encodings, and there the position it is emitted into has one exception.
//! Nothing this module emits coerces or strips a value — every object is
//! `.strict()`, no `format:` normalizes, no field transforms — except
//! `.default(…)`, which fills a property in. So `[{a: "1"}, {a: "1", b: "x"}]`,
//! where `b` defaults to `"x"`, is two instances to JSON Schema and to anyone
//! reading the document, and two equal objects by the time the helper sees them:
//! the emitted parse refuses an array the JSON column accepts. That is
//! `omitted-default-is-the-same-item` in the ledger below, and `state.visits`
//! in the corpus is the document that decides it.
//!
//! It is declared rather than closed because closing it means checking the
//! array's **input** —
//! `z.array(z.unknown()).refine(uniqueItems, …).pipe(z.array(item)…)` — and
//! `@langchain/core` 1.2.8 converts a Zod pipe from its *input* side
//! (`interopZodTransformInputSchema`, `dist/utils/json_schema.js`), so
//! `withStructuredOutput` would then hand the model `{"type": "array", "items":
//! {}}` for every unique-items array. Trading the whole item schema of the
//! surface PRD 5.2 is about for one corner of one keyword is the worse bargain,
//! and the corner is the direction where the emitted parse asks for *more* than
//! the model was told rather than less.
//!
//! # `format:`, and why seven of the ten are written out
//!
//! Grammar 3.3 fixes a closed vocabulary of ten formats and grammar 3.8's table
//! says nothing about any of them, so the reading is this module's to choose —
//! and choosing a Zod constructor by *name* is how the two columns drift. Zod's
//! constructors are not implementations of JSON Schema's `format` keyword and do
//! not claim to be: `z.url()` is `new URL()`, which repairs its input;
//! `z.iso.datetime()` is an upper-case-only ISO 8601 profile, not RFC 3339;
//! `z.email()` is a deliberately narrow subset; `z.hostname()` admits the root
//! dot; `z.uuid()` reads the version and variant nibbles that RFC 9562's *layout*
//! defines and its string production does not. Every one of those was a document
//! the published JSON Schema accepted and the emitted Zod refused — a model
//! answering its own contract and failing the parse.
//!
//! So the reading is named by RFC, and where Zod's constructor reads a different
//! one the check is written out in [`HELPERS`] (`rfc3339Date`, `rfc3339Time`,
//! `rfc3339DateTime`, `rfc3986Uri`, `rfc4122Uuid`, `rfc1123Hostname`,
//! `rfc5321Email`). Three formats keep their constructor. On `ipv4` and `ipv6`
//! the two readings coincide. On `duration` they do not quite, and the
//! constructor is still the right one: `z.iso.duration()` reads the
//! specification grammar 3.3 *names*, and JSON Schema's keyword reads RFC
//! 3339's narrower subset of it, so writing the check out would move the
//! emitted parse away from the grammar rather than towards the schema. That gap
//! is the ledger's `duration-is-iso-8601-not-rfc-3339` row instead of a helper,
//! and it is the one direction this section is not about: the emitted parse
//! accepts what the published schema refuses, never the reverse.
//! [`zod_format`] is the whole table.
//!
//! # The divergence ledger
//!
//! Agreement is a **test**, not a claim: `tests/generated_code_gates.rs` runs
//! `tests/fixtures/schema-lowering/cases.json` against both columns and pins
//! each column's verdict on each document separately, so a document the two
//! answer differently cannot pass as agreement. A corpus case that records two
//! different verdicts must name one of the divergences below, and every
//! divergence below must be exercised by a case — the two lists are asserted
//! against each other.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `integer-beyond-the-safe-range` | `type: integer` | JSON accepts, Zod refuses | grammar 3.8 fixes both spellings, and `z.number().int()` is JavaScript's safe-integer range. Past 2^53 the language cannot count: `JSON.parse` has already rounded `9007199254740993` to an even neighbour by the time any check sees it, so the emitted parse refuses what it cannot represent rather than accepting a number that is no longer the one that was sent |
//! | `multiple-of-under-a-scaled-tolerance` | `multiple_of` | Zod accepts, JSON refuses | the JSON column divides **exactly** — `jsonschema` reads `0.3` as the fraction 3/10 — and `.multipleOf(0.3)` divides in doubles, where `0.9 / 0.3` is `2.9999999999999996` and some tolerance is the only way to call that a multiple. Zod 4.4.3's is relative: `Number.EPSILON * max(\|value / step\|, 1)`, which is what passes `0.9`, and which grows with the quotient until it reaches 0.5 at `0.5 / Number.EPSILON` = 2^51 — past there nothing can fail, so `1e15` is a multiple of `0.3` to the emitted parse and is not one to the JSON column. Note this is a *tolerance* rather than a range: the `type: integer` row below is about a value the language cannot represent, and this is about one it represents exactly. Closing it means a decimal implementation inside every generated module, for quotients no model producing a bounded quantity reaches |
//! | `display-name-is-not-an-addr-spec` | `format: email` | JSON accepts, Zod refuses | `Name <a@example.test>`. JSON Schema defines `email` as RFC 5321's `Mailbox` rule, which has no display-name form; the Rust column's parser offers one and accepts it. This is the one row where the emitted Zod is the **stricter and more correct** column, so it is recorded rather than widened |
//! | `leap-second-away-from-midnight` | `format: time`, `format: date-time` | Zod accepts, JSON refuses | `rfc3339Time` admits `:60` wherever the rest parses; the JSON column admits it only where the value normalizes to `23:59:60` UTC. Narrowing the regex to `[0-5]\d` would trade this corner for the opposite one, and neither is reachable from a model emitting a wall-clock time |
//! | `duration-is-iso-8601-not-rfc-3339` | `format: duration` | Zod accepts, JSON refuses | grammar 3.3's `duration` is the ISO 8601 one, which is what `z.iso.duration()` reads, and the JSON column reads RFC 3339's appendix-A ABNF, which is a proper subset of it. This row is that whole gap rather than one shape of it: the ABNF's date productions nest (`dur-year = Y [dur-month]`), so a skipped designator has no spelling — `P1Y1D`, `PT1H1S` — and its `dur-second = 1*DIGIT "S"` admits no decimal fraction, where ISO 8601 puts one on the smallest component with either separator — `PT1.5S`, `PT1,5S`. Every document it covers runs the *safe* way round: the emitted parse accepts what the published schema does not, rather than refusing what a model was told to send |
//! | `punycode-payload-undecoded` | `format: hostname`, `format: email` | Zod accepts, JSON refuses | an `xn--` label whose payload is not decodable punycode. `rfc1123Hostname` checks the label's *shape*; decoding it would be a punycode implementation inside a generated module, for a case a model does not produce |
//! | `omitted-default-is-the-same-item` | `unique_items` over items carrying a `default:` | JSON accepts, Zod refuses | `[{page: "/"}, {page: "/", via: "direct"}]` where `via` defaults to `"direct"`. The check runs over parsed elements, where the default has been filled in — see the section above for what closing it would cost |
//! | `dot-matches-a-code-unit` | `pattern:` | JSON accepts, Zod refuses | `^.$` against `"😀"`. The emitted literal carries no `u` flag ([`super::pattern`] says why: `u` mode refuses escapes RE2 accepts), so `.` matches one UTF-16 code unit while the Rust column's engine matches one code point. JSON Schema *defines* `pattern` as ECMA-262, which makes the emitted regex the literal reading and the validating column the loose one; agreeing would take a second regex engine in the compiler, or refusing `.` outright. One of two things `.` costs, and the only one a flag would reach — the row below is the other |
//! | `dot-excludes-a-line-terminator` | `pattern:` | JSON accepts, Zod refuses | `^.$` against `"\r"`, and the same for U+2028 and U+2029. ECMA-262's `.` matches any code point **except a LineTerminator** — LF, CR, LS, PS — while the engine reading the published schema excludes LF alone, which is also what RE2's `.` excludes and therefore what the author wrote. Not the row above's mechanism, and no flag closes it: `s` would make `.` match the LF *both* columns refuse, trading three documents for one pointing the other way, and the alternative is rewriting the pattern's text, which [`super::pattern`] does not do — it copies a pattern verbatim and decides only whether every construct transfers. Same reading as the row above, and the same closing cost |
//! | `word-boundary-is-unicode-aware` | `pattern:` | Zod accepts, JSON refuses | `\bcat\b` against `"caté"`. Both engines have `\b` and both call it a word boundary; they disagree about what a word character is. ECMAScript's is ASCII, and the validating column keeps Rust's Unicode one — the translation that makes `\w`, `\d` and `\s` agree (`^\w$` refuses `é` in both) rewrites the *classes* and leaves the boundary alone. Same shape as the two `.` rows above, and the same reading: ECMA-262 is what JSON Schema names — though this one points the other way, because here it is the emitted regex that is the looser of the two |
//!
//! # Patterns
//!
//! `pattern:` is RE2 (Decision D12), which is **not** a subset of JavaScript's
//! syntax — [`super::pattern`] is the module that decides what transfers, and
//! [`super::diagnostics`] is what refuses a `build` whose patterns do not.
//!
//! # What a provider is handed
//!
//! `withStructuredOutput` is the mechanism PRD 5.2 named before 9.16 amended it,
//! and what that mechanism *does* with the Zod this module emits is the
//! measurement the amendment rests on: `@langchain/core` 1.2.8 converts a
//! Zod v4 schema by calling `toJSONSchema` from `zod/v4/core`
//! (`dist/utils/json_schema.js`), and that conversion keeps what Zod models as a
//! *check* and drops what it models as a *refinement*, silently. Against the
//! committed `every-schema-form` golden and the pinned versions, that is:
//!
//! * `state.at` → `{"type": "string"}`; the `format: date-time` check is gone,
//!   as it is for the five other written-out formats;
//! * `state.tags` → `minItems`/`maxItems` kept, `uniqueItems` gone;
//! * `state.mark` → `{"type": "string"}`; both length bounds gone.
//!
//! The `.regex`-spelled formats (`uri`, `time`, `uuid`) survive as `pattern`, so
//! the loss tracks the spelling rather than the keyword.
//!
//! **Which schema a provider is handed is decided, and it is not that
//! conversion** — PRD 9.16 is where the decision is logged, and PRD 5.2 is
//! amended to it. The question that log entry closes — a conversion of this Zod,
//! or [`json_field_map`]'s lowering — was answered by the one property that makes
//! structured output load-bearing for routing: the schema a model is
//! *constrained by* has to be the schema its answer is then *parsed with*, or an
//! agent can answer its own contract and fail the parse. Those two are equal by
//! construction only for the JSON column, which
//! `the_emitted_zod_agrees_with_the_json_schema_lowering` proves document by
//! document — so [`super::runtime`]'s agent call sends [`json_field_map`]'s
//! lowering, over `fetch`, with no `withStructuredOutput` in the path.
//!
//! The evidence stays, and so does the gate that keeps it true:
//! `what_the_structured_output_mechanism_would_be_handed` in
//! `tests/generated_code_gates.rs` runs the real conversion over the real
//! goldens, and it now measures **the road not taken** — the day that conversion
//! stops dropping refinements is the day the choice could be reconsidered, which
//! is the revisit condition PRD 9.16 names. This comment is the evidence behind
//! that entry: the measurements above are what the decision was made on.
//!
//! One surface cannot keep that equality all the way down, and the gap is
//! recorded rather than left to be found. OpenAI's structured-output decoder
//! closes a schema only when every object in it lists every property in
//! `required`, so an agent whose output nests an `optional:` property (grammar
//! 3.4) is sent `strict: false` and is constrained by nothing — while the parse
//! still runs over this module's lowering, unchanged. That row lives in
//! [`super::runtime`]'s ledger rather than here, because it is a property of the
//! *request* and not of the lowering: what this module publishes is what goes on
//! the wire and what the answer is checked against, on both surfaces and at
//! either `strict`.

use std::borrow::Cow;

use serde_json::{Map, Value, json};

use crate::ast::common::Ident;
use crate::ast::definition::AgentAccess;
use crate::ast::schema::{Number, ScalarKind, StringFormat};
use crate::check::model;
use crate::diag::Spanned;
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::NodeKind;
use crate::ir::schema::{
    ArrayType, EnumType, Field, FieldMap, ObjectType, Scalar, TypeForm, TypeNode,
};
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
///
/// The field map is a [`Cow`] because some of them are written nowhere: an
/// inline `exec:`/`http:` node that declares no `output:` has a **kind default**
/// as its result schema (grammar 8.2, 8.3), and a `store:` op node has a
/// **derived** one (grammar 11.4). Both are synthesized rather than parsed. See
/// [`surfaces`].
#[derive(Clone, Debug)]
pub enum Body<'ir> {
    /// A field map: a closed object (grammar 3.1).
    Fields(Cow<'ir, FieldMap>),
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
///
/// "Every schema there is" includes the ones nobody writes. Three node kinds
/// have a result schema that appears nowhere in the source text:
///
/// * an inline `exec:` node with no `output:` — grammar 8.2's
///   `{exit_code, stdout}`;
/// * an inline `http:` node with no `output:` — grammar 8.3's `{status, body}`;
/// * a `store:` op node, whose result is *derived* from the op and the store it
///   names — grammar 11.4's catalogue (Decision D34).
///
/// The validator already resolves all three (`check::Context::node_output`),
/// which is why an edge guard over such a node is type-checked at all. Omitting
/// them here would leave a node with a checked result surface and no emitted
/// schema, no export name and no conformance case, and the node-fn PR would find
/// nothing to parse the node's answer with. Each shape is taken from
/// [`crate::check::model`] rather than restated, because two spellings of one
/// grammar rule is exactly the drift this function exists to prevent.
#[must_use]
pub fn surfaces(ir: &Ir) -> Vec<Surface<'_>> {
    let attached = attached_stores(ir);
    let mut surfaces = Vec::new();
    for (address, definition) in &ir.definitions {
        match &definition.body {
            DefinitionBody::Agent(agent) => {
                if let Some(input) = &agent.input {
                    surfaces.push(Surface {
                        path: format!("{address}.input"),
                        about: format!("`{address}` — its declared input (grammar 5.3)."),
                        body: borrowed(input),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!(
                        "`{address}` — the structured output the model is constrained to, and \
                         what routing reads (PRD 5.2, 5.3)."
                    ),
                    body: borrowed(&agent.output),
                });
            }
            DefinitionBody::Tool(tool) => {
                surfaces.push(Surface {
                    path: format!("{address}.input"),
                    about: format!("`{address}` — its parameters (grammar 6)."),
                    body: borrowed(&tool.input),
                });
                surfaces.push(Surface {
                    path: format!("{address}.output"),
                    about: format!("`{address}` — its result (grammar 6)."),
                    body: borrowed(&tool.output),
                });
            }
            DefinitionBody::Flow(flow) => {
                if let Some(inputs) = &flow.inputs {
                    surfaces.push(Surface {
                        path: format!("{address}.inputs"),
                        about: format!("`{address}` — the module's parameters (grammar 7.5)."),
                        body: borrowed(inputs),
                    });
                }
                surfaces.push(Surface {
                    path: format!("{address}.outputs"),
                    about: format!(
                        "`{address}` — the module's result, materialized from the state channels \
                         of the same names at quiescence (grammar 7.5, 7.6.3)."
                    ),
                    body: borrowed(&flow.outputs),
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
                                body: borrowed(&human.input),
                            });
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the human returns, routable \
                                     like any structured output (grammar 8.7)."
                                ),
                                body: borrowed(&human.output),
                            });
                        }
                        NodeKind::Exec { exec } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the subprocess produces \
                                     (grammar 8.2){}.",
                                    default_note(exec.output.is_none())
                                ),
                                body: exec.output.as_ref().map_or_else(
                                    || owned(model::exec_default_output(&exec.span)),
                                    borrowed,
                                ),
                            });
                        }
                        NodeKind::Http { http } => {
                            surfaces.push(Surface {
                                path: format!("{address}.node.{id}.output"),
                                about: format!(
                                    "`{address}` node `{id}` — what the response decodes to \
                                     (grammar 8.3){}.",
                                    default_note(http.output.is_none())
                                ),
                                body: http.output.as_ref().map_or_else(
                                    || owned(model::http_default_output(&http.span)),
                                    borrowed,
                                ),
                            });
                        }
                        NodeKind::Store { store, op, params } => {
                            // Grammar 11.4's row, derived from the op and the
                            // store it names — a shape that exists nowhere in
                            // the source text, like the two kind defaults
                            // above. A `store:` whose address did not resolve
                            // has no row to derive, and the resolver has
                            // already said so, which is the one case with no
                            // surface at all.
                            if let Some(definition) = ir.definitions.get(&store.value.to_string())
                                && let DefinitionBody::Store(definition) = &definition.body
                            {
                                surfaces.push(Surface {
                                    path: format!("{address}.node.{id}.output"),
                                    about: format!(
                                        "`{address}` node `{id}` — what the `{}` of \
                                         `{}` answers, derived from the op and the store \
                                         (grammar 11.4, Decision D34).",
                                        op.as_str(),
                                        store.value
                                    ),
                                    body: owned(model::store_output(
                                        definition.kind,
                                        *op,
                                        definition.value_schema.as_ref(),
                                        definition.metadata_schema.as_ref(),
                                        params.top_k.or(params.limit),
                                        &node.span,
                                    )),
                                });
                            }
                        }
                        NodeKind::Agent { .. }
                        | NodeKind::Function { .. }
                        | NodeKind::Flow { .. }
                        | NodeKind::Map { .. } => {}
                    }
                }
            }
            DefinitionBody::Store(store) => {
                if let Some(value) = &store.value_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.value_schema"),
                        about: format!("`{address}` — the value it stores (grammar 11.1)."),
                        body: borrowed(value),
                    });
                }
                if let Some(metadata) = &store.metadata_schema {
                    surfaces.push(Surface {
                        path: format!("{address}.metadata_schema"),
                        about: format!(
                            "`{address}` — the metadata a match carries (grammar 11.1)."
                        ),
                        body: borrowed(metadata),
                    });
                }
                // The synthesized tool surface of grammar 11.5, which exists
                // exactly when some agent attaches this store: an unattached
                // store synthesizes nothing, so a schema for it would be a
                // schema nothing is parsed against. Which ops are here is
                // `agent_access:`'s (Decision D37).
                if attached.contains(address) {
                    let local = address
                        .split_once('.')
                        .map_or(address.as_str(), |(_, rest)| rest);
                    let access = store.agent_access.unwrap_or(AgentAccess::ReadWrite);
                    for op in model::store_tools(store.kind, access) {
                        surfaces.push(Surface {
                            path: format!("{address}.tool.{}.input", op.as_str()),
                            about: format!(
                                "`{address}` — the arguments of its synthesized `{}` tool, which \
                                 is its `{}` row of grammar 11.4 with the expressions replaced by \
                                 what the model supplies (grammar 11.5).",
                                model::store_tool_name(local, *op),
                                op.as_str()
                            ),
                            body: owned(model::store_tool_input(
                                store.kind,
                                *op,
                                store.value_schema.as_ref(),
                                store.metadata_schema.as_ref(),
                                &definition.span,
                            )),
                        });
                    }
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

/// Every `store.*` some agent attaches (grammar 5.4, 11.5).
///
/// The tool surface a store synthesizes exists because an *agent* listed it, not
/// because the store was defined, so this is what decides whether a store has
/// one at all.
fn attached_stores(ir: &Ir) -> std::collections::BTreeSet<String> {
    let mut found = std::collections::BTreeSet::new();
    for definition in ir.definitions.values() {
        let DefinitionBody::Agent(agent) = &definition.body else {
            continue;
        };
        for store in &agent.stores {
            found.insert(store.value.to_string());
        }
    }
    found
}

/// A field map the composition wrote, as a [`Body`].
///
/// Every surface the source text spells out is one of these; the helper exists
/// so the borrow is written once instead of at each of a dozen push sites.
fn borrowed(fields: &FieldMap) -> Body<'_> {
    Body::Fields(Cow::Borrowed(fields))
}

/// A field map the *grammar* supplies where the composition wrote none — an
/// inline node's kind default (grammar 8.2, 8.3) or a store op's derived row
/// (grammar 11.4).
fn owned(fields: FieldMap) -> Body<'static> {
    Body::Fields(Cow::Owned(fields))
}

/// The clause the doc comment of an inline node's result schema carries when the
/// schema is the kind default rather than one the node declared.
fn default_note(defaulted: bool) -> &'static str {
    if defaulted {
        ", which it does not declare, so this is the kind default"
    } else {
        ""
    }
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

    for helper in HELPERS {
        if surfaces
            .iter()
            .any(|surface| surface.declares(&|ty| helper.wanted_by(ty)))
        {
            contents.push_str(helper.source);
        }
    }

    for surface in &surfaces {
        let name = names.value(&surface.path);
        let ty = names.ty(&surface.path);
        contents.push('\n');
        contents.push_str(&names::doc("", std::slice::from_ref(&surface.about)));
        let expression = match &surface.body {
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

/// One module-level helper the emitted schemas can lean on.
///
/// A helper is written only where the composition reaches the form that wants it
/// — an unused `const` in a generated module is a question a reader has to
/// answer for nothing — and the table is walked in order, so a helper that calls
/// another (`rfc5321Email` calls `rfc1123Hostname`, `rfc3339DateTime` calls both
/// of its halves) is declared after it. `wants` lists those callees, so asking
/// for one asks for its dependencies too and the order of the table is the only
/// thing keeping them in scope.
struct Helper {
    /// The `format:` values whose lowering calls it directly.
    formats: &'static [StringFormat],
    /// Whether an array's `unique_items` calls it.
    unique_items: bool,
    /// Whether a string's `min_length`/`max_length` calls it.
    length_bounds: bool,
    /// Other helpers it calls, which therefore must be emitted with it.
    wants: &'static [&'static str],
    /// Its own name, for [`Helper::wants`] to name it by.
    name: &'static str,
    /// The declaration, opening with a blank line.
    source: &'static str,
}

impl Helper {
    /// Whether this type node reaches this helper, directly or through one that
    /// does.
    fn wanted_by(&self, ty: &TypeNode) -> bool {
        HELPERS.iter().any(|helper| {
            (helper.name == self.name || helper.wants.contains(&self.name)) && helper.called_by(ty)
        })
    }

    /// Whether this type node's own lowering calls this helper by name.
    fn called_by(&self, ty: &TypeNode) -> bool {
        match &ty.form {
            TypeForm::Scalar(scalar) => {
                scalar
                    .format
                    .is_some_and(|format| self.formats.contains(&format))
                    || (self.length_bounds
                        && (scalar.min_length.is_some() || scalar.max_length.is_some()))
            }
            TypeForm::Array(array) => self.unique_items && array.unique_items == Some(true),
            TypeForm::Enum(_) | TypeForm::Object(_) | TypeForm::Union(_) => false,
        }
    }
}

/// Every helper, in declaration order — callees before their callers.
const HELPERS: &[Helper] = &[
    Helper {
        formats: &[],
        unique_items: false,
        length_bounds: true,
        wants: &[],
        name: "codePoints",
        source: r#"
/**
 * The length `min_length` and `max_length` bound (grammar 3.4).
 *
 * JSON Schema counts a string's length in Unicode **code points**, and
 * `String.prototype.length` — which `.min()` and `.max()` count — is UTF-16
 * **code units**. Every character outside the BMP is one of the first and two of
 * the second, so the two disagree in both directions on any string carrying one:
 * `"😀"` is one code point and two units. Spreading a string iterates it by code
 * point, which is the rule the schema this composition published states.
 */
const codePoints = (value: string): number => [...value].length;
"#,
    },
    Helper {
        formats: &[],
        unique_items: true,
        length_bounds: false,
        wants: &[],
        name: "uniqueItems",
        source: r#"
/**
 * `unique_items: true` (grammar 3.5). Zod has no built-in.
 *
 * JSON Schema's `uniqueItems` compares *instances*: two objects are one item
 * when they carry the same keys with equal values, in whatever order those keys
 * were written. `JSON.stringify` compares encodings, and an object's encoding
 * follows its key order — so the two are not the same rule in general.
 *
 * They are the same rule **here**. `.refine` runs over the array's already
 * *parsed* elements, and `z.object({…}).strict()` rebuilds every object in the
 * order its own shape declares, so two equal instances have identical encodings
 * by the time this is called. That is a fact about where this is used, which is
 * why it is not exported: applied to raw input, it would call
 * `[{a: 1, b: 2}, {b: 2, a: 1}]` unique and JSON Schema would not.
 *
 * One instance of that fact runs the other way. A property with a `default:` is
 * filled in before this is called, so two items that differ only in omitting it
 * — two instances to JSON Schema — arrive here as one. The parse is stricter
 * than the published schema on exactly those arrays, deliberately: see
 * `omitted-default-is-the-same-item` in the compiler's divergence ledger.
 */
const uniqueItems = (items: readonly unknown[]): boolean =>
  new Set(items.map((item) => JSON.stringify(item))).size === items.length;
"#,
    },
    Helper {
        formats: &[StringFormat::Hostname],
        unique_items: false,
        length_bounds: false,
        wants: &[],
        name: "rfc1123Hostname",
        source: r#"
/**
 * `format: hostname` (grammar 3.3): an RFC 1123 host name, which is what JSON
 * Schema's `hostname` means. `z.hostname()` is a looser reading — it accepts the
 * root-relative `example.test.`, which JSON Schema refuses — and a value one
 * column accepts and the other refuses is the drift grammar 3.8's table exists
 * to prevent, so the rule is written out.
 */
const rfc1123Hostname = (value: string): boolean => {
  if (value.length === 0 || value.length > 253 || value.endsWith(".")) {
    return false;
  }
  return value.split(".").every((label) => {
    if (label.length === 0 || label.length > 63) {
      return false;
    }
    if (!/^[A-Za-z0-9](?:[A-Za-z0-9-]*[A-Za-z0-9])?$/.test(label)) {
      return false;
    }
    // RFC 5891: hyphens in the third and fourth positions are reserved for the
    // `xn--` prefix of an internationalized label.
    return !(label.length >= 4 && label[2] === "-" && label[3] === "-") || label.startsWith("xn--");
  });
};
"#,
    },
    Helper {
        formats: &[StringFormat::Date, StringFormat::DateTime],
        unique_items: false,
        length_bounds: false,
        wants: &[],
        name: "rfc3339Date",
        source: r#"
/**
 * `format: date` (grammar 3.3): an RFC 3339 `full-date`, calendar-checked — the
 * month bounds the day, and February bounds it by the proleptic Gregorian leap
 * rule, so `2026-02-30` is not a date.
 */
const rfc3339Date = (value: string): boolean => {
  const parts = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (parts === null) {
    return false;
  }
  const [, year, month, day] = parts.map(Number);
  if (month < 1 || month > 12 || day < 1) {
    return false;
  }
  const leap = (year % 4 === 0 && year % 100 !== 0) || year % 400 === 0;
  return day <= [31, leap ? 29 : 28, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31][month - 1];
};
"#,
    },
    Helper {
        formats: &[StringFormat::Time, StringFormat::DateTime],
        unique_items: false,
        length_bounds: false,
        wants: &[],
        name: "rfc3339Time",
        source: r#"
/**
 * `format: time` (grammar 3.3), which JSON Schema reads as RFC 3339 `full-time`
 * — an offset is required. Zod's `z.iso.time()` refuses an offset outright, so
 * the check is written here rather than borrowed from a constructor that means
 * something else. The offset may be `Z`, `z`, or `±HH:MM`, and a leap second is
 * spelled `:60`.
 */
const rfc3339Time =
  /^([01]\d|2[0-3]):[0-5]\d:([0-5]\d|60)(\.\d+)?([Zz]|[+-]([01]\d|2[0-3]):[0-5]\d)$/;
"#,
    },
    Helper {
        formats: &[StringFormat::DateTime],
        unique_items: false,
        length_bounds: false,
        wants: &["rfc3339Date", "rfc3339Time"],
        name: "rfc3339DateTime",
        source: r#"
/**
 * `format: date-time` (grammar 3.3): an RFC 3339 `date-time`, which is a
 * `full-date`, the separator, and a `full-time`. The separator is `T` or `t` and
 * the offset may be `Z` or `z`: RFC 3339 says so in as many words, and the JSON
 * Schema column agrees, while `z.iso.datetime()` accepts only the upper-case
 * spellings.
 */
const rfc3339DateTime = (value: string): boolean => {
  const separator = value.search(/[Tt]/);
  return (
    separator > 0 &&
    rfc3339Date(value.slice(0, separator)) &&
    rfc3339Time.test(value.slice(separator + 1))
  );
};
"#,
    },
    Helper {
        formats: &[StringFormat::Uri],
        unique_items: false,
        length_bounds: false,
        wants: &[],
        name: "rfc3986Uri",
        source: r#"
/**
 * `format: uri` (grammar 3.3): an absolute RFC 3986 URI — a scheme, a
 * hierarchical part, and the optional query and fragment — spelled as the
 * grammar's own production. `z.url()` is not this check: it is `new URL()`,
 * which is the WHATWG parser, and that one *repairs* what it is given. It
 * accepts a space and a non-ASCII character in a path (percent-encoding them)
 * and rejects `https://`, which RFC 3986 admits as a URI with an empty
 * authority — divergences in both directions from the column beside it.
 */
const rfc3986Uri =
  /^[A-Za-z][A-Za-z0-9+\-.]*:(?:\/\/(?:(?:[A-Za-z0-9\-._~!$&'()*+,;=:]|%[0-9A-Fa-f]{2})*@)?(?:\[[A-Za-z0-9:.]+\]|(?:[A-Za-z0-9\-._~!$&'()*+,;=]|%[0-9A-Fa-f]{2})*)(?::\d*)?(?:\/(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})*)*|\/?(?:(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})+(?:\/(?:[A-Za-z0-9\-._~!$&'()*+,;=:@]|%[0-9A-Fa-f]{2})*)*)?)(?:\?(?:[A-Za-z0-9\-._~!$&'()*+,;=:@/?]|%[0-9A-Fa-f]{2})*)?(?:#(?:[A-Za-z0-9\-._~!$&'()*+,;=:@/?]|%[0-9A-Fa-f]{2})*)?$/;
"#,
    },
    Helper {
        formats: &[StringFormat::Uuid],
        unique_items: false,
        length_bounds: false,
        wants: &[],
        name: "rfc4122Uuid",
        source: r#"
/**
 * `format: uuid` (grammar 3.3): the RFC 4122 (now RFC 9562) string
 * representation — five hyphen-separated groups of hex digits, in either case,
 * which is exactly what JSON Schema's `uuid` reads. `z.uuid()` is a narrower
 * check: it also enforces the version and variant nibbles of the *layout*, so it
 * refuses a Microsoft GUID (`…-c456-…`), a version-0 or version-9 value, and
 * anything else a system upstream of this one minted without following RFC
 * 9562's field rules. The published schema blesses those, so the parse beside it
 * has to as well.
 */
const rfc4122Uuid = /^[0-9A-Fa-f]{8}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{4}-[0-9A-Fa-f]{12}$/;
"#,
    },
    Helper {
        formats: &[StringFormat::Email],
        unique_items: false,
        length_bounds: false,
        wants: &["rfc1123Hostname"],
        name: "rfc5321Email",
        source: r#"
/**
 * `format: email` (grammar 3.3): an RFC 5322 `addr-spec` whose domain is an RFC
 * 1123 host name or an address literal — the reading JSON Schema's `email`
 * takes. `z.email()` is a deliberately narrow subset of it and refuses three
 * things the column beside it accepts: a quoted local part, a single-label
 * domain (`a@b`), and `a@[192.0.2.1]`.
 */
const rfc5321Email = (value: string): boolean => {
  // The *last* `@` splits: an unquoted local part cannot hold one, and a quoted
  // one can hold as many as it likes.
  const at = value.lastIndexOf("@");
  if (at <= 0 || at === value.length - 1 || at > 64) {
    return false;
  }
  const local = value.slice(0, at);
  const domain = value.slice(at + 1);
  const atext = /^[A-Za-z0-9!#$%&'*+\-/=?^_`{|}~\u0080-\uffff]+$/;
  const quoted = /^(?:[\t \x21\x23-\x5b\x5d-\x7e\u0080-\uffff]|\\[\x21-\x7e])*$/;
  const localOk =
    local.length > 2 && local.startsWith('"') && local.endsWith('"')
      ? quoted.test(local.slice(1, -1))
      : local.split(".").every((atom) => atext.test(atom));
  if (!localOk) {
    return false;
  }
  if (domain.length > 2 && domain.startsWith("[") && domain.endsWith("]")) {
    const literal = domain.slice(1, -1);
    return literal.startsWith("IPv6:")
      ? z.ipv6().safeParse(literal.slice(5)).success
      : z.ipv4().safeParse(literal).success;
  }
  return rfc1123Hostname(domain);
};
"#,
    },
];

/// Every name [`module`] can declare besides the schemas themselves.
///
/// [`super::names`]'s reserved list is checked against this, so a helper added
/// to [`HELPERS`] cannot be left out of the namespace the registry
/// disambiguates against.
#[cfg(test)]
pub(super) fn helper_names() -> impl Iterator<Item = &'static str> {
    HELPERS.iter().map(|helper| helper.name)
}

impl Surface<'_> {
    /// Visit every type node inside this surface, outermost first.
    ///
    /// [`super::diagnostics`] reads it: a rule stated over a type node has to
    /// reach the nested ones too, and the only enumeration of "every type node
    /// there is" should be the one [`surfaces`] already fixes.
    pub fn walk(&self, visit: &mut dyn FnMut(&TypeNode)) {
        match &self.body {
            Body::Fields(fields) => {
                for field in &fields.fields {
                    walk(&field.ty, visit);
                }
            }
            Body::Type(ty) => walk(ty, visit),
        }
    }

    /// Visit every name the emitted Zod uses as a **key**, outermost first.
    ///
    /// Two things become keys of an object shape: a field map's property names
    /// (grammar 3.1, 3.4) and a union's `discriminator` (grammar 3.7), which is
    /// both the key `z.discriminatedUnion` indexes on and a `z.literal` property
    /// of every variant. A variant *tag* is not one — it is the value that key
    /// holds — and neither is a channel name, which [`channels`] enumerates for
    /// the state model.
    ///
    /// [`super::diagnostics`] reads it for the same reason it reads [`walk`]: a
    /// rule about what a key may be spelled has to reach the nested ones too.
    pub fn walk_keys(&self, visit: &mut dyn FnMut(&Spanned<Ident>, KeyKind)) {
        match &self.body {
            Body::Fields(fields) => walk_map_keys(fields, visit),
            Body::Type(ty) => walk_type_keys(ty, visit),
        }
    }

    /// Whether any type node anywhere inside this surface satisfies `predicate`.
    fn declares(&self, predicate: &dyn Fn(&TypeNode) -> bool) -> bool {
        match &self.body {
            Body::Fields(fields) => fields
                .fields
                .iter()
                .any(|field| declares(&field.ty, predicate)),
            Body::Type(ty) => declares(ty, predicate),
        }
    }
}

/// What makes one name a key of an emitted object shape.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// A property of a field map or of an object type node (grammar 3.1, 3.4).
    Property,
    /// A union's tag field (grammar 3.7).
    Discriminator,
}

impl KeyKind {
    /// How a diagnostic names it.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Property => "schema property",
            Self::Discriminator => "discriminator",
        }
    }
}

/// Visit every key of one field map, and of everything nested inside it.
fn walk_map_keys(fields: &FieldMap, visit: &mut dyn FnMut(&Spanned<Ident>, KeyKind)) {
    for field in &fields.fields {
        visit(&field.name, KeyKind::Property);
        walk_type_keys(&field.ty, visit);
    }
}

/// Visit every key one type node introduces, and of everything nested inside it.
fn walk_type_keys(ty: &TypeNode, visit: &mut dyn FnMut(&Spanned<Ident>, KeyKind)) {
    match &ty.form {
        TypeForm::Array(array) => walk_type_keys(&array.items, visit),
        TypeForm::Object(object) => walk_map_keys(&object.properties, visit),
        TypeForm::Union(union) => {
            visit(&union.discriminator, KeyKind::Discriminator);
            for variant in &union.variants {
                walk_map_keys(&variant.fields, visit);
            }
        }
        TypeForm::Scalar(_) | TypeForm::Enum(_) => {}
    }
}

/// Visit this type node and everything nested inside it, outermost first.
fn walk(ty: &TypeNode, visit: &mut dyn FnMut(&TypeNode)) {
    visit(ty);
    match &ty.form {
        TypeForm::Array(array) => walk(&array.items, visit),
        TypeForm::Object(object) => {
            for field in &object.properties.fields {
                walk(&field.ty, visit);
            }
        }
        TypeForm::Union(union) => {
            for variant in &union.variants {
                for field in &variant.fields.fields {
                    walk(&field.ty, visit);
                }
            }
        }
        TypeForm::Scalar(_) | TypeForm::Enum(_) => {}
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
    if let Some(default) = effective_default(ty) {
        text.push_str(&format!(".default({})", names::literal(&default)));
    }
    text
}

/// The `default:` a type node declares, whichever form carries it. A union never
/// does (grammar 3.6).
fn declared_default(ty: &TypeNode) -> Option<&crate::ast::common::Literal> {
    declared_default_entry(ty).map(|default| &default.value)
}

/// The same, with the span the literal was written at — which is the span a
/// filled-in property borrows when [`effective_default`] copies it.
fn declared_default_entry(
    ty: &TypeNode,
) -> Option<&crate::diag::Spanned<crate::ast::common::Literal>> {
    match &ty.form {
        TypeForm::Scalar(scalar) => scalar.default.as_ref(),
        TypeForm::Enum(enumeration) => enumeration.default.as_ref(),
        TypeForm::Object(object) => object.default.as_ref(),
        TypeForm::Array(array) => array.default.as_ref(),
        TypeForm::Union(_) => None,
    }
}

/// The value this type node's `default:` **denotes**: the literal as written,
/// with every nested `default:` it leaves out filled in.
///
/// Grammar 3.6 makes a property carrying a `default:` implicitly optional at its
/// surface, so `default: { other: "x" }` on an object whose `count` defaults to
/// `0` is a literal that validates against the type node it sits on, and the
/// validator accepts it. It is not the whole value, though: grammar 10.1 says a
/// channel `default:` is what "makes the whole channel total from step 0", and
/// the nested `default:` is the only thing that can supply `count`. So the
/// denoted value is the literal *parsed against its own schema*, which is this —
/// a compile-time parse, because both columns want the answer and neither runs
/// the other's.
///
/// Emitting the literal as written instead was not merely incomplete, it did not
/// compile: Zod 4's `.default(v)` takes `core.output<T>`, in which a defaulted
/// property is **required**, so `z.object({count: z.number().int().default(0), …
/// }).strict().default({other: "x"})` is a `tsc` error — `build` exiting `0` over
/// a project that cannot pass the type gate CLAUDE.md requires of every one.
///
/// Only the two forms that *hold* other type nodes can gain anything, and a
/// literal of the wrong shape is left alone: the validator has already refused
/// it, and guessing at a repair would emit something nobody wrote.
pub(super) fn effective_default(ty: &TypeNode) -> Option<Cow<'_, crate::ast::common::Literal>> {
    declared_default_entry(ty).map(|default| complete(ty, &default.value))
}

/// One literal, completed against the type node it is the `default:` of.
fn complete<'a>(
    ty: &'a TypeNode,
    value: &'a crate::ast::common::Literal,
) -> Cow<'a, crate::ast::common::Literal> {
    use crate::ast::common::Literal;

    match (&ty.form, value) {
        (TypeForm::Object(object), Literal::Mapping(_)) => {
            complete_mapping(&object.properties, value)
        }
        (TypeForm::Array(array), Literal::Sequence(items)) => {
            let mut completed = Vec::with_capacity(items.len());
            let mut changed = false;
            for item in items {
                let value = complete(&array.items, &item.value);
                changed |= matches!(value, Cow::Owned(_));
                completed.push(crate::diag::Spanned {
                    value: value.into_owned(),
                    span: item.span.clone(),
                });
            }
            if changed {
                Cow::Owned(Literal::Sequence(completed))
            } else {
                Cow::Borrowed(value)
            }
        }
        // A union carries no `default:` of its own (grammar 3.6) and still
        // reaches here inside one: an object's default supplies a union-typed
        // property as a mapping, and the variant that mapping's discriminator
        // names is the field map to complete it against.
        (TypeForm::Union(union), Literal::Mapping(entries)) => {
            let tag = entries
                .iter()
                .find(|entry| entry.key.value == union.discriminator.value.as_str())
                .and_then(|entry| match &entry.value.value {
                    Literal::String(text) => Some(text.as_str()),
                    _ => None,
                });
            union
                .variants
                .iter()
                .find(|variant| Some(variant.tag.value.as_str()) == tag)
                .map_or(Cow::Borrowed(value), |variant| {
                    complete_mapping(&variant.fields, value)
                })
        }
        _ => Cow::Borrowed(value),
    }
}

/// A mapping literal completed against the field map it denotes a value of:
/// every entry completed in place, then every declared property the mapping
/// leaves out and that carries its own `default:` appended, in declaration
/// order.
///
/// Written entries keep the order they were written in, so a literal that needs
/// nothing back is the same bytes it always was.
fn complete_mapping<'a>(
    fields: &'a FieldMap,
    value: &'a crate::ast::common::Literal,
) -> Cow<'a, crate::ast::common::Literal> {
    use crate::ast::common::{Literal, LiteralEntry};

    let Literal::Mapping(entries) = value else {
        return Cow::Borrowed(value);
    };
    let mut completed: Vec<LiteralEntry> = Vec::with_capacity(entries.len());
    let mut changed = false;
    for entry in entries {
        let value = match fields.field(&entry.key.value) {
            Some(field) => complete(&field.ty, &entry.value.value),
            None => Cow::Borrowed(&entry.value.value),
        };
        changed |= matches!(value, Cow::Owned(_));
        completed.push(LiteralEntry {
            key: entry.key.clone(),
            value: crate::diag::Spanned {
                value: value.into_owned(),
                span: entry.value.span.clone(),
            },
        });
    }
    for field in &fields.fields {
        let name = field.name.value.as_str();
        if entries.iter().any(|entry| entry.key.value == name) {
            continue;
        }
        let Some(default) = declared_default_entry(&field.ty) else {
            continue;
        };
        changed = true;
        completed.push(LiteralEntry {
            key: crate::diag::Spanned {
                value: name.to_string(),
                span: field.name.span.clone(),
            },
            value: crate::diag::Spanned {
                value: complete(&field.ty, &default.value).into_owned(),
                span: default.span.clone(),
            },
        });
    }
    if changed {
        Cow::Owned(Literal::Mapping(completed))
    } else {
        Cow::Borrowed(value)
    }
}

fn scalar_expression(scalar: &Scalar) -> String {
    let mut text = match (scalar.kind, scalar.format) {
        (ScalarKind::String, Some(format)) => zod_format(format).to_string(),
        (ScalarKind::String, None) => "z.string()".to_string(),
        (ScalarKind::Integer, _) => "z.number().int()".to_string(),
        (ScalarKind::Number, _) => "z.number()".to_string(),
        (ScalarKind::Boolean, _) => "z.boolean()".to_string(),
    };
    // Not `.min()`/`.max()`: those count UTF-16 code units and the keyword
    // beside them counts code points — see the module docs.
    if let Some(min) = scalar.min_length {
        text.push_str(&format!(
            ".refine((value) => codePoints(value) >= {min}, {{ message: \"expected at least {}\" }})",
            characters(min)
        ));
    }
    if let Some(max) = scalar.max_length {
        text.push_str(&format!(
            ".refine((value) => codePoints(value) <= {max}, {{ message: \"expected at most {}\" }})",
            characters(max)
        ));
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

/// A length bound as the message it fails with says it: `1 character`, and
/// `n characters` for every other bound.
///
/// The message is what a reader of a failed parse is handed, and `min_length: 1`
/// — the "not empty" spelling, and the commonest bound there is — would
/// otherwise report `expected at least 1 characters`.
fn characters(count: i64) -> String {
    if count == 1 {
        "1 character".to_string()
    } else {
        format!("{count} characters")
    }
}

/// The check one `format:` lowers to (grammar 3.3).
///
/// Three of the ten are Zod's own constructor. On `ipv4` and `ipv6`, Zod's
/// reading and JSON Schema's `format` keyword agree on every document the
/// conformance corpus can find. On `duration` they do not: `z.iso.duration()`
/// reads ISO 8601, which is what grammar 3.3 means by the word, and the keyword
/// reads RFC 3339's appendix-A subset of it — so the constructor stays because
/// writing the check out would narrow the emitted parse below the grammar, and
/// the difference is a declared divergence
/// (`duration-is-iso-8601-not-rfc-3339`) rather than a helper. It runs the
/// direction the seven below are about avoiding: the parse accepts what the
/// published schema refuses.
///
/// The other seven are written out in [`HELPERS`] instead, because their
/// constructors read a *different specification* from the one JSON Schema's
/// keyword names — and JSON Schema is what a model is handed, so a value it
/// blessed that the emitted Zod then refused is a run failing on its own
/// contract. Each is one sentence:
///
/// * `date-time`/`date` — `z.iso.datetime()` and `z.iso.date()` take the
///   upper-case-only ISO 8601 profile; RFC 3339 admits `t` and `z`, and a leap
///   second.
/// * `time` — `z.iso.time()` **refuses** an offset and RFC 3339's `full-time`
///   **requires** one, so the two are disjoint rather than merely different.
/// * `email` — `z.email()` is a narrow subset that refuses a quoted local part,
///   a single-label domain, and an address literal.
/// * `uri` — `z.url()` is the WHATWG parser, which repairs its input rather than
///   validating it.
/// * `uuid` — `z.uuid()` enforces RFC 9562's version and variant nibbles, which
///   the string representation JSON Schema's `uuid` reads does not constrain: a
///   legacy GUID and a version-0 value are UUIDs to the published schema and not
///   to the constructor.
/// * `hostname` — `z.hostname()` accepts a trailing root dot.
const fn zod_format(format: StringFormat) -> &'static str {
    match format {
        StringFormat::DateTime => {
            "z.string().refine(rfc3339DateTime, { message: \"expected an RFC 3339 date-time\" })"
        }
        StringFormat::Date => {
            "z.string().refine(rfc3339Date, { message: \"expected an RFC 3339 date\" })"
        }
        StringFormat::Time => "z.string().regex(rfc3339Time)",
        StringFormat::Duration => "z.iso.duration()",
        StringFormat::Email => {
            "z.string().refine(rfc5321Email, { message: \"expected an email address\" })"
        }
        StringFormat::Uri => "z.string().regex(rfc3986Uri)",
        StringFormat::Uuid => "z.string().regex(rfc4122Uuid)",
        StringFormat::Hostname => {
            "z.string().refine(rfc1123Hostname, { message: \"expected a hostname\" })"
        }
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

/// A `pattern:` as a JavaScript regular expression.
///
/// The decision — whether RE2's source text *has* a JavaScript spelling — is
/// [`super::pattern`]'s, and [`super::diagnostics`] is what stops `build` before
/// a pattern with no spelling reaches here. Emission stays total for the
/// library caller who skipped that step: the refused text is written out as it
/// would have been, which is a module `tsc` and the first `import` both report.
fn regex_expression(pattern: &str) -> String {
    super::pattern::javascript(pattern).unwrap_or_else(|_| super::pattern::verbatim(pattern))
}

// ---------------------------------------------------------------------------
// The JSON Schema column
// ---------------------------------------------------------------------------

/// A field map as JSON Schema draft 2020-12: a closed object.
#[must_use]
pub fn json_field_map(map: &FieldMap) -> Value {
    json_fields(&map.fields)
}

/// The same object, for a surface held as fields rather than as a written map.
///
/// A flow's parameter surface is one: a flow with no `inputs:` has no field map
/// anywhere — the empty parameter list is written nowhere — so the sites that
/// resolve it answer with fields, and a flow attached as an agent tool
/// (grammar 5.4) needs those fields as the tool's JSON Schema.
#[must_use]
pub fn json_fields(fields: &[Field]) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    for field in fields {
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
    // The value the `default:` denotes rather than the text it was written as —
    // the same one the Zod column installs, so the published annotation and the
    // parse beside it cannot say two different things (see [`effective_default`]).
    if let Some(default) = effective_default(ty) {
        object.insert("default".to_string(), json_literal(&default));
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
        match &surface.body {
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
            "z.string()\
             .refine((value) => codePoints(value) >= 1, { message: \"expected at least 1 character\" })\
             .refine((value) => codePoints(value) <= 200, { message: \"expected at most 200 characters\" })",
            "`.min()`/`.max()` count UTF-16 code units and `minLength`/`maxLength` count code points"
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

    /// Six of the ten formats are written out rather than borrowed from a Zod
    /// constructor that reads a different specification (see the module docs).
    #[test]
    fn a_format_lowers_to_the_check_the_rfc_names() {
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: date-time }}\n"),
                "state.a"
            ),
            "z.string().refine(rfc3339DateTime, { message: \"expected an RFC 3339 date-time\" })",
            "`z.iso.datetime()` is an upper-case-only ISO 8601 profile, not RFC 3339"
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
            "z.string().refine(rfc5321Email, { message: \"expected an email address\" })\
             .refine((value) => codePoints(value) <= 320, { message: \"expected at most 320 characters\" })",
            "a constraint still chains onto the refined string"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: uri }}\n"),
                "state.a"
            ),
            "z.string().regex(rfc3986Uri)"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: uuid }}\n"),
                "state.a"
            ),
            "z.string().regex(rfc4122Uuid)",
            "`z.uuid()` enforces the version and variant nibbles the string form does not"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a: {{ type: string, format: duration }}\n"),
                "state.a"
            ),
            "z.iso.duration()",
            "the three that agree with JSON Schema keep their constructor"
        );
    }

    /// A helper is written only where the composition reaches it, and a helper
    /// that calls another brings it along.
    #[test]
    fn the_helper_prelude_holds_what_the_composition_reaches_and_no_more() {
        let module = |source: &str| {
            let ir = ir_of(source);
            crate::codegen::schema::module(&ir, &Names::of(&ir)).contents
        };

        let bare = module(&format!("{CHANNEL}  a: {{ type: string }}\n"));
        for helper in HELPERS {
            assert!(
                !bare.contains(&format!("const {} ", helper.name)),
                "`{}` is declared in a module that never calls it",
                helper.name
            );
        }

        // `date-time` calls both halves of itself, and neither is declared after
        // the caller that needs it.
        let stamped = module(&format!(
            "{CHANNEL}  a: {{ type: string, format: date-time }}\n"
        ));
        let at = |name: &str| {
            stamped
                .find(&format!("const {name} "))
                .map(|at| at as isize)
        };
        assert!(at("rfc3339Date").is_some() && at("rfc3339Time").is_some());
        assert!(at("rfc3339DateTime") > at("rfc3339Date"));
        assert!(at("rfc3339DateTime") > at("rfc3339Time"));
        assert!(at("rfc5321Email").is_none(), "and nothing it does not call");

        // A length bound is the other constraint-level helper: a string that
        // declares one brings `codePoints` and nothing else.
        let bounded = module(&format!(
            "{CHANNEL}  a: {{ type: string, max_length: 8 }}\n"
        ));
        assert!(bounded.contains("const codePoints "));
        assert!(!bounded.contains("const uniqueItems "));

        // `email` calls `rfc1123Hostname`, which `hostname` also selects.
        let addressed = module(&format!(
            "{CHANNEL}  a: {{ type: string, format: email }}\n"
        ));
        assert!(addressed.contains("const rfc1123Hostname "));
        assert!(
            addressed.find("const rfc1123Hostname ") < addressed.find("const rfc5321Email "),
            "a callee is declared before its caller"
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

    /// A `default:` denotes the value its literal **parses to**, so a property
    /// the literal leaves out and that carries its own `default:` is filled in.
    ///
    /// Grammar 3.6 makes a defaulted property optional at its surface, so the
    /// literal below validates and the validator accepts it; grammar 10.1 says
    /// the channel is nevertheless total from step 0, which only the nested
    /// default can make true. Zod agrees in the type system — `.default()` takes
    /// the *output* type, where `count` is required — so emitting the literal as
    /// written was a project `tsc` refused.
    #[test]
    fn a_default_is_completed_with_the_defaults_nested_inside_it() {
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: object\n    properties:\n      count: {{ type: integer, default: 0 }}\n      other: {{ type: string }}\n    default: {{ other: \"x\" }}\n"
                ),
                "state.a"
            ),
            "z.object({\n  count: z.number().int().default(0),\n  other: z.string(),\n})\
             .strict().default({ \"other\": \"x\", \"count\": 0 })",
            "the written entries keep their order and the filled-in one is appended"
        );

        // One level further in: an array default whose items carry a default.
        assert_eq!(
            schema_of(
                &format!(
                    "{CHANNEL}  a:\n    type: array\n    max_items: 4\n    items:\n      type: object\n      properties:\n        page: {{ type: string }}\n        via: {{ type: string, default: direct }}\n    default: [{{ page: \"/\" }}]\n"
                ),
                "state.a"
            ),
            "z.array(z.object({\n  page: z.string(),\n  via: z.string().default(\"direct\"),\n})\
             .strict()).max(4).default([{ \"page\": \"/\", \"via\": \"direct\" }])"
        );

        // And a union-typed property, which carries no `default:` of its own and
        // still reaches the completion inside one: the variant its
        // discriminator names is the field map to complete against.
        let union = schema_of(
            &format!(
                "{CHANNEL}  a:\n    type: object\n    properties:\n      source:\n        discriminator: kind\n        variants:\n          upload: {{ name: {{ type: string }}, retries: {{ type: integer, default: 3 }} }}\n          crawl: {{ url: {{ type: string }} }}\n    default: {{ source: {{ kind: upload, name: \"f\" }} }}\n"
            ),
            "state.a",
        );
        assert!(
            union.ends_with(
                ".default({ \"source\": { \"kind\": \"upload\", \"name\": \"f\", \"retries\": 3 } })"
            ),
            "{union}"
        );
    }

    /// The completion is the *same* value in both columns: the published
    /// annotation and the value the parse installs cannot say different things.
    #[test]
    fn the_json_column_publishes_the_default_the_parse_installs() {
        let ir = ir_of(&format!(
            "{CHANNEL}  a:\n    type: object\n    properties:\n      count: {{ type: integer, default: 0 }}\n      other: {{ type: string }}\n    default: {{ other: \"x\" }}\n"
        ));
        let surfaces = surfaces(&ir);
        let Body::Type(ty) = &surfaces[0].body else {
            panic!("a channel is a type node");
        };
        assert_eq!(
            json_type_node(ty)["default"],
            json!({ "other": "x", "count": 0 })
        );
    }

    /// A literal that needs nothing back is the bytes it always was — the
    /// completion is not a rewrite of every default.
    #[test]
    fn a_default_that_supplies_everything_is_left_alone() {
        let supplied = schema_of(
            &format!(
                "{CHANNEL}  a:\n    type: object\n    properties:\n      count: {{ type: integer, default: 0 }}\n      other: {{ type: string }}\n    default: {{ other: \"x\", count: 7 }}\n"
            ),
            "state.a",
        );
        assert!(
            supplied.ends_with(".default({ \"other\": \"x\", \"count\": 7 })"),
            "{supplied}"
        );
        assert_eq!(
            schema_of(
                &format!("{CHANNEL}  a:\n    type: string\n    default: \"x\"\n"),
                "state.a"
            ),
            "z.string().default(\"x\")"
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
            "z.object({\n  name: z.string(),\n  email: z.string().refine(rfc5321Email, \
             { message: \"expected an email address\" }).optional(),\n}).strict()"
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
        let Body::Type(ty) = &surfaces[0].body else {
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
        let Body::Fields(fields) = &surface.body else {
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
flow.f:\n  outputs: { draft: { type: string } }\n  nodes:\n    a: { agent: agent.a, input: { goal: \"'x'\" } }\n    ask:\n      human:\n        input: { q: { type: string } }\n        output: { answer: { type: string } }\n    probe: { exec: { command: \"true\" } }\n    ping: { http: { method: GET, url: \"https://example.test\" } }\n    keep: { store: store.s, op: set, key: \"'k'\", value: { v: \"'x'\" } }\n  edges:\n    - { from: start, to: a }\n    - { from: a, to: ask }\n    - { from: ask, to: probe }\n    - { from: probe, to: ping }\n    - { from: ping, to: keep }\n    - { from: keep, to: end }\n";
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
                // None of the three writes an `output:`, and all three are
                // here anyway: a kind default and a derived store row are the
                // node's result schema, not the absence of one.
                "flow.f.node.probe.output",
                "flow.f.node.ping.output",
                "flow.f.node.keep.output",
                "store.s.value_schema",
                "tool.t.input",
                "tool.t.output",
                "state.draft",
            ]
        );
    }

    /// An inline node with no `output:` has the **kind default** as its result
    /// schema (grammar 8.2, 8.3), and the emitter writes it.
    ///
    /// The validator already resolves the same default (`check::model`), so a
    /// composition can guard an edge on `probe.output.exit_code` while the
    /// emitted module holds no schema and no export name for it. The two
    /// defaults are asserted field by field here rather than only through the
    /// goldens, because a change to either one is a change to what a node's
    /// answer is parsed with.
    #[test]
    fn an_inline_node_with_no_output_gets_its_kind_default_schema() {
        let source = "version: \"0.1\"\n\
flow.f:\n  outputs: {}\n  nodes:\n    probe: { exec: { command: \"true\" } }\n    ping: { http: { method: GET, url: \"https://example.test\" } }\n  edges:\n    - { from: start, to: probe }\n    - { from: probe, to: ping }\n    - { from: ping, to: end }\n";
        assert_eq!(
            schema_of(source, "flow.f.node.probe.output"),
            "z.object({\n  exit_code: z.number().int(),\n  stdout: z.string(),\n}).strict()"
        );
        assert_eq!(
            schema_of(source, "flow.f.node.ping.output"),
            "z.object({\n  status: z.number().int(),\n  body: z.string(),\n}).strict()"
        );

        // And the doc comment says the schema is a default rather than
        // something the node was read as declaring.
        let ir = ir_of(source);
        let module = crate::codegen::schema::module(&ir, &Names::of(&ir)).contents;
        assert!(
            module.contains("which it does not declare, so this is the kind default"),
            "{module}"
        );
    }

    /// A node that writes the default out by hand is the same schema, and is
    /// **not** described as a default — the control for the test above.
    #[test]
    fn a_declared_output_is_emitted_as_declared() {
        let source = "version: \"0.1\"\n\
flow.f:\n  outputs: {}\n  nodes:\n    probe:\n      exec:\n        command: \"true\"\n        output:\n          exit_code: { type: integer }\n          stdout: { type: string }\n  edges:\n    - { from: start, to: probe }\n    - { from: probe, to: end }\n";
        assert_eq!(
            schema_of(source, "flow.f.node.probe.output"),
            "z.object({\n  exit_code: z.number().int(),\n  stdout: z.string(),\n}).strict()"
        );
        let ir = ir_of(source);
        let module = crate::codegen::schema::module(&ir, &Names::of(&ir)).contents;
        assert!(!module.contains("kind default"), "{module}");
    }
}
