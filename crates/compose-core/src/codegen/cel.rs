//! `src/cel.ts`: the CEL evaluator generated routers embed (PRD 5.5).
//!
//! # The choice, and what it was between
//!
//! PRD 5.5 fixes the shape — "generated routers embed a JS CEL evaluator rather
//! than transpiling CEL→TS" — and leaves *which* evaluator open. The two
//! candidates were a maintained JS CEL library, pinned exactly beside the rest
//! of [`super::project::PINS`], and a small evaluator written for the subset
//! grammar 4.1 admits. This module is the second, and the corpus is what decided
//! it:
//!
//! * **`int` is an `int64`.** `tests/fixtures/cel-conformance/operators.json`
//!   pins a literal, an addition and an inequality either side of 2^53, plus the
//!   overflow at `int64`'s own edge, because a JS evaluator backed by `number`
//!   folds those silently. An evaluator that answers them is one built on
//!   `BigInt` from the ground up; retrofitting that onto a library is not a
//!   configuration.
//! * **The surface is closed.** Grammar 4.1 admits the standard operators,
//!   `size()`, `has()`, four string predicates and five comprehension macros,
//!   and the validator refuses everything else before a router is emitted. An
//!   evaluator that accepts more is drift pointing the other way — a
//!   composition that runs and does not validate.
//! * **Pinning is per compiler release** (PRD 5.12). A dependency whose reading
//!   of `matches` or of numeric equality changed in a patch release would move
//!   the runtime's semantics without moving the compiler's, which is the exact
//!   failure the pinning discipline exists to prevent.
//!
//! The cost is this module's ~900 lines of TypeScript, carried in the compiler
//! and emitted byte-identically into every project. What buys it back is that
//! **the corpus runs against it**:
//! `crates/agent-compose/tests/compiled_graph_acceptance/cel-conformance.mjs`
//! evaluates every case in `tests/fixtures/cel-conformance/` against the emitted
//! module under the pinned Node, and
//! `the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus`
//! fails the build when one case answers differently from the Rust column.
//!
//! # The divergence ledger
//!
//! `codegen::schema` keeps one of these for grammar 3.8's two columns, and the
//! discipline is the same here: a difference between the two CEL
//! implementations is something a reader signed off on, or it is a bug.
//!
//! | id | where | which way | why it is left |
//! |---|---|---|---|
//! | `size-of-a-string-counts-code-points` | `size(<string>)` | the crate counts **bytes**, this counts **code points** | the specification says code points, and `[...value].length` is that. The crate's answer is a defect (`size('héllo')` is 6 there and 5 by the specification) and cannot be corrected from outside it: `Context::add_function("size", …)` does not override a built-in, the standard set being consulted first. What makes the gap harmless is *where* each evaluator runs — the compiler never evaluates an expression (`crate::cel` type-checks and stops), so the crate is the corpus's reference column rather than a runtime, and the only evaluator a compiled graph runs is this one. The corpus states only ASCII `size()` cases, where all readings agree, and `the_size_of_a_non_ascii_string_still_diverges_from_the_specification` pins the crate's answer so a fix or a pin bump goes red |
//! | `a-matches-dot-is-a-code-unit-and-excludes-more` | `matches(…, '<pattern with `.`>')` | the crate's `.` is one **code point** and excludes **LF alone**; `RegExp`'s is one UTF-16 **code unit** and excludes every **line terminator** | the pattern is a second language inside the expression, and `matches()` reaches the same two regex engines `pattern:` does — so this is `codegen::schema`'s `dot-matches-a-code-unit` and `dot-excludes-a-line-terminator`, reached through a guard instead of a schema. `'👍'.matches('^.$')` is `true` in the crate and `false` here, and `'\r'` likewise. Closing it means a second regex engine in every generated module, or the `u` flag — which [`super::pattern`] shows refuses escapes RE2 accepts — or rewriting the pattern's text, which this compiler does not do to a `pattern:` either |
//! | `a-matches-word-boundary-is-unicode-aware` | `matches(…, '<pattern with `\b`>')` | the crate's `\b` sits between **Unicode** word characters, `RegExp`'s between **ASCII** ones | `codegen::schema`'s `word-boundary-is-unicode-aware`, same surface change. `'caté'.matches('\\bcat\\b')` is `false` in the crate and `true` here. The translation that makes `\w`, `\d` and `\s` agree rewrites the *classes* and leaves the boundary alone, which is why this row survives the one above being about `.` |
//!
//! `size` is not reachable from a shape the validator has to refuse. The two
//! `matches` rows are what is **left over** from a refusal: every pattern only
//! one engine can *parse* — inline flags, `(?P<…>)`, look-around, a
//! backreference — is refused by [`super::diagnostics`], and these two are the
//! constructs both engines parse and read differently, which no refusal can tell
//! apart from the patterns that transfer. Neither is a validate-time rejection:
//! `validate` answers a target-independent question, and both of these are about
//! what *this* target's engine does. `the_dot_and_the_word_boundary_in_a_matches_pattern_still_read_differently`
//! pins the crate's answers, and the corpus states only patterns with neither
//! construct in them.
//!
//! **One row was closed rather than declared**, and how it was found is the
//! point: CEL's logical operators are commutative in the presence of errors —
//! `<error> && false` is `false`, not an error — and the first draft of this
//! evaluator short-circuited instead, which agreed with the crate on every case
//! the hand-written corpus states. `tests/property_conformance.rs` generated
//! `(state.c2.n > 0) && false` over a `merge`-shaped channel whose optional `n`
//! was absent, on its eighth seed, and the two columns split. The corpus now
//! states all six absorption cases, and the evaluator implements them.
//!
//! # Shapes
//!
//! JSON cannot tell `1` from `1.0` and CEL must (`1 + 2.5` is an error in the
//! specification and in both implementations), so a value entering an expression
//! is read through a **shape** derived from the declaration it came from — a
//! state channel's type, an agent's output schema, a flow's input schema.
//! [`shape_of_field_map`] and [`shape_of_type`] are that lowering, and they are
//! what keeps `type: number` a double at run time even when the value that
//! arrived happens to be integral.

use crate::ast::common::{Cel, EdgeSource, EdgeTarget};
use crate::diag::Spanned;
use crate::ir::Ir;
use crate::ir::binding::{Bindings, Http, NodeInput};
use crate::ir::flow::{MapDispatch, NodeKind, StoreValue};
use crate::ir::schema::{FieldMap, TypeForm, TypeNode};

/// The evaluator's source, carried in the compiler and emitted verbatim.
const SOURCE: &str = include_str!("js/cel.ts");

/// `src/cel.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    super::GeneratedFile {
        path: "src/cel.ts".to_string(),
        contents,
    }
}

/// The `Shape` a field map's values are read through.
///
/// A `Shape` is **data** — the emitted type is a union of string tags and two
/// object forms, every one of them JSON-representable — so it is built as JSON
/// once and rendered from there. That is what lets a test hand the very same
/// shape to the emitted evaluator (`tests/property_conformance.rs`) instead of
/// re-deriving one that would then be the thing under test.
#[must_use]
pub fn shape_json_of_field_map(map: &FieldMap) -> serde_json::Value {
    let mut properties = serde_json::Map::new();
    for field in &map.fields {
        properties.insert(field.name.value.as_str().to_string(), shape_json(&field.ty));
    }
    serde_json::json!({ "properties": serde_json::Value::Object(properties) })
}

/// The `Shape` one type node's values are read through.
///
/// A **union** is the one form with no total answer: the payload a value carries
/// depends on the tag it carries, so the shape names the discriminator — a
/// string on every variant — and leaves the rest to the fallback rule (an
/// integral number is an `int`). Every rule that reads a union's payload
/// narrows it by tag first (grammar 8.6 rule 4), and narrowing is the `map`
/// construct's, which is a later M1 bullet.
#[must_use]
pub fn shape_json(ty: &TypeNode) -> serde_json::Value {
    match &ty.form {
        TypeForm::Scalar(scalar) => serde_json::Value::String(
            match scalar.kind {
                crate::ast::schema::ScalarKind::String => "string",
                crate::ast::schema::ScalarKind::Integer => "int",
                crate::ast::schema::ScalarKind::Number => "double",
                crate::ast::schema::ScalarKind::Boolean => "bool",
            }
            .to_string(),
        ),
        TypeForm::Enum(_) => serde_json::Value::String("string".to_string()),
        TypeForm::Object(object) => shape_json_of_field_map(&object.properties),
        TypeForm::Array(array) => serde_json::json!({ "items": shape_json(&array.items) }),
        TypeForm::Union(union) => serde_json::json!({
            "properties": { union.discriminator.value.as_str(): "string" },
            "rest": "any",
        }),
    }
}

/// The same, as the TypeScript source `src/graph.ts` declares.
#[must_use]
pub fn shape_of_field_map(map: &FieldMap, indent: &str) -> String {
    super::graph::json_literal(&shape_json_of_field_map(map), indent)
}

/// The same, for one type node.
#[must_use]
pub fn shape_of_type(ty: &TypeNode, indent: &str) -> String {
    super::graph::json_literal(&shape_json(ty), indent)
}

/// Whether an expression reads `<node>.output` — the one thing a `skip` changes
/// about routing (grammar 9.2, Decision D97).
///
/// Decided by the same front-end the validator uses rather than by a scan of the
/// text: an empty scope reports the roots as unknown, which this ignores, and
/// still records every root-anchored path the expression reads.
#[must_use]
pub fn reads_output_of(source: &str, node: &str) -> bool {
    crate::cel::analyze(source, &crate::cel::Scope::default())
        .reads
        .iter()
        .any(|read| read.root == node && read.path.first().is_some_and(|first| first == "output"))
}

/// Every CEL expression the composition writes, with the surface that wrote it.
///
/// One walk, exhaustive over the IR's node kinds, trigger kinds and tool
/// bindings — the same shape [`super::env::References::of`] takes over `${ENV}`
/// references, and for the same reason: a target check that reads *some* of the
/// expressions is a check whose coverage is an accident of which surfaces
/// somebody remembered. The `match`es are exhaustive on purpose, so a construct
/// a later milestone adds is a compile error here rather than a surface this
/// silently stops reading.
///
/// The `site` is diagnostic voice — "`flow.review_loop`'s edge `review` → `write`
/// guard" — because a span names *where* and a reader also needs *what*.
#[must_use]
pub fn expressions(ir: &Ir) -> Vec<Expression<'_>> {
    let mut found = Vec::new();
    for (address, definition) in &ir.definitions {
        match &definition.body {
            crate::ir::definition::DefinitionBody::Tool(tool) => match &tool.implementation {
                crate::ir::flow::ToolImplementation::Http { http } => {
                    http_block(http, &format!("`{address}`'s `http:` binding"), &mut found);
                }
                crate::ir::flow::ToolImplementation::Exec { .. }
                | crate::ir::flow::ToolImplementation::Function { .. } => {}
            },
            crate::ir::definition::DefinitionBody::Flow(flow) => {
                for node in &flow.nodes {
                    let id = node.id.value.as_str();
                    let site = format!("`{address}` node `{id}`");
                    node_input(
                        node.input.as_ref(),
                        &format!("{site}'s `input:`"),
                        &mut found,
                    );
                    node_kind(&node.kind, &site, &mut found);
                }
                for edge in &flow.edges {
                    if let Some(when) = &edge.when {
                        let from = match &edge.from.value {
                            EdgeSource::Start => "start".to_string(),
                            EdgeSource::Node(node) => node.to_string(),
                        };
                        let to = match &edge.to.value {
                            EdgeTarget::End => "end".to_string(),
                            EdgeTarget::Node(node) => node.to_string(),
                        };
                        found.push(Expression {
                            source: when,
                            site: format!("`{address}`'s edge `{from}` → `{to}` guard"),
                        });
                    }
                }
            }
            crate::ir::definition::DefinitionBody::Agent(_)
            | crate::ir::definition::DefinitionBody::Store(_)
            | crate::ir::definition::DefinitionBody::Provider(_)
            | crate::ir::definition::DefinitionBody::Model(_) => {}
        }
    }
    let triggers = ir
        .triggers
        .iter()
        .flat_map(|section| section.entries.values());
    for trigger in triggers {
        let name = trigger.name.value.as_str();
        let site = format!("trigger `{name}`");
        if let Some(key) = &trigger.session_key {
            found.push(Expression {
                source: key,
                site: format!("{site}'s `session_key:`"),
            });
        }
        match &trigger.kind {
            crate::ir::trigger::TriggerKind::Manual => {}
            crate::ir::trigger::TriggerKind::Http(http) => {
                bindings(
                    http.input.as_ref(),
                    &format!("{site}'s `input:`"),
                    &mut found,
                );
                if let Some(callback) = &http.callback {
                    found.push(Expression {
                        source: callback,
                        site: format!("{site}'s `callback:`"),
                    });
                }
            }
            crate::ir::trigger::TriggerKind::Schedule(schedule) => {
                bindings(
                    schedule.input.as_ref(),
                    &format!("{site}'s `input:`"),
                    &mut found,
                );
            }
            crate::ir::trigger::TriggerKind::Event(event) => {
                bindings(
                    event.input.as_ref(),
                    &format!("{site}'s `input:`"),
                    &mut found,
                );
                if let Some(key) = &event.dedupe_key {
                    found.push(Expression {
                        source: key,
                        site: format!("{site}'s `dedupe_key:`"),
                    });
                }
            }
        }
    }
    found
}

/// One CEL expression the composition writes.
#[derive(Clone, Debug)]
pub struct Expression<'ir> {
    /// The expression itself, with the span of the scalar that carried it.
    pub source: &'ir Spanned<Cel>,
    /// Where it was written, in diagnostic voice.
    pub site: String,
}

fn bindings<'ir>(map: Option<&'ir Bindings>, site: &str, found: &mut Vec<Expression<'ir>>) {
    for binding in map.iter().flat_map(|map| &map.entries) {
        found.push(Expression {
            source: &binding.value,
            site: format!("{site} `{}`", binding.name.value),
        });
    }
}

fn node_input<'ir>(input: Option<&'ir NodeInput>, site: &str, found: &mut Vec<Expression<'ir>>) {
    match input {
        None => {}
        Some(NodeInput::Scalar { value }) => found.push(Expression {
            source: value,
            site: site.to_string(),
        }),
        Some(NodeInput::Fields { bindings: map }) => bindings(Some(map), site, found),
    }
}

fn http_block<'ir>(http: &'ir Http, site: &str, found: &mut Vec<Expression<'ir>>) {
    bindings(http.query.as_ref(), &format!("{site}'s `query:`"), found);
    bindings(http.body.as_ref(), &format!("{site}'s `body:`"), found);
}

fn node_kind<'ir>(kind: &'ir NodeKind, site: &str, found: &mut Vec<Expression<'ir>>) {
    match kind {
        NodeKind::Agent { .. }
        | NodeKind::Exec { .. }
        | NodeKind::Function { .. }
        | NodeKind::Flow { .. }
        | NodeKind::Human { .. } => {}
        NodeKind::Http { http } => http_block(http, site, found),
        NodeKind::Map { map } => match &map.dispatch {
            MapDispatch::Homogeneous { input, .. } => {
                node_input(input.as_ref(), &format!("{site}'s `map.input:`"), found);
            }
            MapDispatch::Routed {
                routes, default, ..
            } => {
                for route in routes.iter().chain(default.iter().map(AsRef::as_ref)) {
                    let tag = route
                        .tag
                        .as_ref()
                        .map_or_else(|| "default".to_string(), |tag| tag.value.to_string());
                    node_input(
                        route.input.as_ref(),
                        &format!("{site}'s `map` route `{tag}` `input:`"),
                        found,
                    );
                }
            }
        },
        NodeKind::Store { params, .. } => {
            for (expression, key) in [
                (params.key.as_ref(), "key"),
                (params.query.as_ref(), "query"),
                (params.prefix.as_ref(), "prefix"),
            ] {
                if let Some(source) = expression {
                    found.push(Expression {
                        source,
                        site: format!("{site}'s `{key}:`"),
                    });
                }
            }
            match &params.value {
                None => {}
                Some(StoreValue::Expression { value }) => found.push(Expression {
                    source: value,
                    site: format!("{site}'s `value:`"),
                }),
                Some(StoreValue::Fields { bindings: map }) => {
                    bindings(Some(map), &format!("{site}'s `value:`"), found);
                }
            }
            bindings(
                params.filter.as_ref(),
                &format!("{site}'s `filter:`"),
                found,
            );
            bindings(
                params.metadata.as_ref(),
                &format!("{site}'s `metadata:`"),
                found,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    #[test]
    fn the_module_is_emitted_verbatim_under_the_header() {
        let emitted = module(&ir_of("version: \"0.1\"\n")).contents;
        assert!(emitted.starts_with("// This file was generated by agent-compose"));
        assert!(emitted.contains("export function evaluateGuard("));
        assert!(
            emitted.ends_with(SOURCE),
            "the evaluator is emitted verbatim"
        );
    }

    /// The numeric forms are what the shape exists for: an `integer` is an
    /// `int` however the JSON looked, and a `number` a `double`.
    #[test]
    fn a_shape_carries_the_declared_numeric_type() {
        let ir = ir_of(
            "version: \"0.1\"\nstate:\n  count: { type: integer }\n  score: { type: number }\n  name: { type: string }\n  flag: { type: boolean }\n  mood: { enum: [calm, tense] }\n",
        );
        let channels = crate::codegen::schema::channels(&ir);
        let shapes: Vec<String> = channels
            .iter()
            .map(|(_, channel)| shape_of_type(&channel.ty, ""))
            .collect();
        assert_eq!(
            shapes,
            [
                "\"int\"",
                "\"bool\"",
                "\"string\"",
                "\"string\"",
                "\"double\""
            ]
        );
    }

    #[test]
    fn a_nested_shape_reaches_arrays_and_objects() {
        let ir = ir_of(
            "version: \"0.1\"\nstate:\n  rows:\n    type: array\n    max_items: 4\n    items:\n      type: object\n      properties:\n        n: { type: integer }\n",
        );
        let channels = crate::codegen::schema::channels(&ir);
        assert_eq!(
            shape_json(&channels[0].1.ty),
            serde_json::json!({ "items": { "properties": { "n": "int" } } })
        );
        // …and the TypeScript the emitter writes is that JSON, rendered.
        let shape = shape_of_type(&channels[0].1.ty, "");
        assert!(shape.contains("\"items\":"), "{shape}");
        assert!(shape.contains("\"n\": \"int\""), "{shape}");
    }

    /// The shape a test hands the emitted evaluator is the emitter's own, so a
    /// harness comparing the two columns is not comparing a copy of one of them
    /// (`tests/property_conformance.rs`).
    #[test]
    fn the_shape_a_test_reads_is_the_shape_the_project_carries() {
        let ir = ir_of("version: \"0.1\"\nstate:\n  count: { type: integer }\n");
        let channels = crate::codegen::schema::channels(&ir);
        let ty = &channels[0].1.ty;
        assert_eq!(
            shape_of_type(ty, ""),
            crate::codegen::graph::json_literal(&shape_json(ty), "")
        );
    }

    /// The walk reaches every surface a composition can write CEL at.
    ///
    /// Not the ones somebody remembered: the composition below writes an
    /// expression at each position the IR carries one — both `input:` forms, an
    /// inline `http:` body, a `tool.*`'s own `http:` query, the six CEL store
    /// parameters across four ops, both `map` dispatch forms including a
    /// `default:` route, an edge guard, and all three trigger kinds that carry
    /// expressions — and the assertion is the whole set. A surface the walk
    /// stopped reading is a missing row here rather than a target check quietly
    /// covering less.
    #[test]
    fn the_walk_reaches_every_surface_a_composition_writes_cel_at() {
        let ir = ir_of(SURFACES);
        let mut found: Vec<String> = expressions(&ir)
            .into_iter()
            .map(|expression| format!("{} = {}", expression.site, expression.source.value.as_str()))
            .collect();
        found.sort();
        assert_eq!(found, SITES, "the surfaces the walk reads have changed");
    }

    /// Every expression the composition below writes, sorted by site.
    const SITES: &[&str] = &[
        r"`flow.surfaces` node `call`'s `body:` `goal` = input.goal",
        r"`flow.surfaces` node `fan`'s `map.input:` `text` = item",
        r"`flow.surfaces` node `find`'s `filter:` `tag` = 'a'",
        r"`flow.surfaces` node `find`'s `query:` = state.note",
        r"`flow.surfaces` node `keep`'s `key:` = state.note",
        r"`flow.surfaces` node `keep`'s `metadata:` `tag` = 'b'",
        r"`flow.surfaces` node `keep`'s `value:` = state.note",
        r"`flow.surfaces` node `listing`'s `prefix:` = state.note",
        r"`flow.surfaces` node `named`'s `input:` `q` = state.note",
        r"`flow.surfaces` node `plan`'s `input:` `text` = input.goal",
        r"`flow.surfaces` node `routed`'s `map` route `default` `input:` `text` = finding.kind",
        r"`flow.surfaces` node `routed`'s `map` route `quick` `input:` `text` = finding.file",
        r"`flow.surfaces` node `save`'s `key:` = execution.session_key",
        r"`flow.surfaces` node `save`'s `value:` `body` = state.note",
        r"`flow.surfaces` node `shell`'s `input:` = state.note",
        r"`flow.surfaces` node `sub`'s `input:` `text` = state.note",
        r"`flow.surfaces`'s edge `listing` → `sub` guard = state.note != ''",
        r"`tool.remote`'s `http:` binding's `query:` `term` = input.q",
        r"trigger `nightly`'s `input:` `goal` = payload.trigger",
        r"trigger `nightly`'s `session_key:` = payload.trigger",
        r"trigger `stream`'s `dedupe_key:` = payload.id",
        r"trigger `stream`'s `input:` `goal` = payload.body.goal",
        r"trigger `stream`'s `session_key:` = payload.id",
        r"trigger `web`'s `callback:` = payload.body.callback_url",
        r"trigger `web`'s `input:` `goal` = payload.body.goal",
        r"trigger `web`'s `session_key:` = payload.headers['x-session-id']",
    ];

    /// One expression per CEL-carrying position in the IR.
    const SURFACES: &str = r##"version: "0.1"

state:
  note: { type: string, default: "" }
  tally:
    type: array
    max_items: 4
    items: { type: string }
    reduce: append

store.notes:
  kind: kv
  scope: session
  value_schema:
    body: { type: string }

store.vectors:
  kind: vector
  scope: global
  embed: { model: text-embedding-3-small, provider: provider.o }
  metadata_schema:
    tag: { type: string }

tool.remote:
  description: A tool whose binding carries CEL over its own input.
  input:
    q: { type: string }
  output:
    hit: { type: string }
  http:
    method: GET
    url: "https://${API_HOST}/search"
    query:
      term: "input.q"

agent.tagger:
  description: Tags one item.
  model: model.smart
  prompt: Tag it.
  input:
    text: { type: string }
  output:
    tag: { enum: [a, b] }
    items:
      type: array
      max_items: 4
      items: { type: string }
    findings:
      type: array
      max_items: 4
      items:
        discriminator: kind
        variants:
          quick: { file: { type: string } }
          slow: { summary: { type: string } }

provider.p:
  kind: anthropic
  api_key: ${KEY}

provider.o:
  kind: openai
  api_key: ${OKEY}

model.smart:
  provider: provider.p
  id: claude-sonnet-4-6

flow.leaf:
  inputs:
    text: { type: string }
  outputs:
    note: { type: string }
  nodes:
    echo:
      exec:
        command: printf
        args: ["%s", "x"]
        output:
          stdout: { type: string }
      writes:
        stdout: note
  edges:
    - { from: start, to: echo }
    - { from: echo, to: end }

flow.surfaces:
  inputs:
    goal: { type: string }
  outputs:
    note: { type: string }
  nodes:
    call:
      http:
        method: POST
        url: "https://${API_HOST}/go"
        body:
          goal: "input.goal"
        output:
          status: { type: integer }
          body: { type: string }

    shell:
      exec:
        command: printf
        args: ["%s", "y"]
        output:
          stdout: { type: string }
      input: "state.note"
      writes:
        stdout: note

    named:
      function: tool.remote
      input:
        q: "state.note"

    save:
      store: store.notes
      op: set
      key: "execution.session_key"
      value:
        body: "state.note"

    find:
      store: store.vectors
      op: search
      query: "state.note"
      top_k: 3
      filter:
        tag: "'a'"

    keep:
      store: store.vectors
      op: upsert
      key: "state.note"
      value: "state.note"
      metadata:
        tag: "'b'"

    listing:
      store: store.notes
      op: list
      limit: 10
      prefix: "state.note"

    fan:
      map:
        over: "plan.output.items"
        as: item
        max_concurrency: 2
        node: agent.tagger
        input:
          text: "item"
        writes:
          tag: tally

    routed:
      map:
        over: "plan.output.findings"
        as: finding
        route_by: kind
        max_concurrency: 2
        routes:
          quick:
            node: agent.tagger
            input:
              text: "finding.file"
            writes:
              tag: tally
        default:
          node: agent.tagger
          input:
            text: "finding.kind"
          writes:
            tag: tally

    plan:
      agent: agent.tagger
      input:
        text: "input.goal"

    sub:
      flow: flow.leaf
      input:
        text: "state.note"

  edges:
    - { from: start, to: plan }
    - { from: plan, to: fan }
    - { from: fan, to: routed }
    - { from: routed, to: call }
    - { from: call, to: shell }
    - { from: shell, to: named }
    - { from: named, to: save }
    - { from: save, to: find }
    - { from: find, to: keep }
    - { from: keep, to: listing }
    - { from: listing, to: sub, when: "state.note != ''" }
    - { from: listing, to: end, else: true }
    - { from: sub, to: end }

triggers:
  web:
    type: http
    flow: flow.surfaces
    path: /go
    method: POST
    session_key: "payload.headers['x-session-id']"
    input:
      goal: "payload.body.goal"
    respond: async
    callback: "payload.body.callback_url"

  nightly:
    type: schedule
    flow: flow.surfaces
    cron: "0 3 * * *"
    session_key: "payload.trigger"
    input:
      goal: "payload.trigger"

  stream:
    type: event
    flow: flow.surfaces
    source: bus
    session_key: "payload.id"
    dedupe_key: "payload.id"
    input:
      goal: "payload.body.goal"
"##;

    /// Decision D97's test: which guards a `skip` turns false.
    #[test]
    fn an_output_reading_guard_is_told_apart_from_a_state_only_one() {
        assert!(reads_output_of(
            "review.output.verdict == 'revise'",
            "review"
        ));
        assert!(reads_output_of(
            "size(state.feedback) > 0 || review.output.verdict != 'approve'",
            "review"
        ));
        assert!(!reads_output_of("size(state.feedback) > 0", "review"));
        // Another node's output is not this node's.
        assert!(!reads_output_of("write.output.draft != ''", "review"));
        // A channel that happens to be named after the node is not its output.
        assert!(!reads_output_of("state.review == 'done'", "review"));
    }
}
