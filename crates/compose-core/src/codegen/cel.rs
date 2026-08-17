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
//! | `logical-operators-short-circuit-rather-than-absorb` | `&&`, `\|\|` | both short-circuit; neither absorbs an error from the other side | the specification's commutative reading answers `false` for `error && false`; both implementations answer with the error. Identical on every expression whose operands evaluate, which is every expression the corpus states and every guard shape grammar 7.3.1 recognizes — a guard reads a declared field, and the one case where a read legitimately fails is the one `has()` is for (grammar 4.1). Closing it means evaluating both sides of every conjunction, which is a change to *when* effects happen for a language whose expressions have none |
//!
//! Neither row is reachable from a shape the validator has to refuse, so nothing
//! here is a validate-time rejection. A future divergence that *is* reachable
//! belongs in [`super::diagnostics`] beside the two the target already refuses.
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

use crate::ir::Ir;
use crate::ir::schema::{FieldMap, TypeForm, TypeNode};

use super::names;

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

/// The `Shape` a field map's values are read through, as TypeScript source.
#[must_use]
pub fn shape_of_field_map(map: &FieldMap, indent: &str) -> String {
    let inner = format!("{indent}  ");
    let mut text = String::from("{\n");
    text.push_str(&format!("{inner}properties: {{\n"));
    for field in &map.fields {
        text.push_str(&format!(
            "{inner}  {}: {},\n",
            names::string(field.name.value.as_str()),
            shape_of_type(&field.ty, &format!("{inner}  "))
        ));
    }
    text.push_str(&format!("{inner}}},\n"));
    text.push_str(&format!("{indent}}}"));
    text
}

/// The `Shape` one type node's values are read through, as TypeScript source.
///
/// A **union** is the one form with no total answer: the payload a value carries
/// depends on the tag it carries, so the shape names the discriminator — a
/// string on every variant — and leaves the rest to the fallback rule (an
/// integral number is an `int`). Every rule that reads a union's payload
/// narrows it by tag first (grammar 8.6 rule 4), and narrowing is the `map`
/// construct's, which is a later M1 bullet.
#[must_use]
pub fn shape_of_type(ty: &TypeNode, indent: &str) -> String {
    match &ty.form {
        TypeForm::Scalar(scalar) => match scalar.kind {
            crate::ast::schema::ScalarKind::String => "\"string\"".to_string(),
            crate::ast::schema::ScalarKind::Integer => "\"int\"".to_string(),
            crate::ast::schema::ScalarKind::Number => "\"double\"".to_string(),
            crate::ast::schema::ScalarKind::Boolean => "\"bool\"".to_string(),
        },
        TypeForm::Enum(_) => "\"string\"".to_string(),
        TypeForm::Object(object) => shape_of_field_map(&object.properties, indent),
        TypeForm::Array(array) => format!(
            "{{ items: {} }}",
            shape_of_type(&array.items, &format!("{indent}  "))
        ),
        TypeForm::Union(union) => format!(
            "{{ properties: {{ {}: \"string\" }}, rest: \"any\" }}",
            names::string(union.discriminator.value.as_str())
        ),
    }
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
        let shape = shape_of_type(&channels[0].1.ty, "");
        assert!(shape.contains("items:"), "{shape}");
        assert!(shape.contains("\"n\": \"int\""), "{shape}");
    }

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
