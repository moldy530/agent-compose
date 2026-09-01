//! `src/otlp.ts`: the hand-written OTLP/JSON span exporter (PRD resolved q51).
//!
//! One settled trace envelope in, one `ExportTraceServiceRequest` out. What
//! differs between two deployments is what the sink is *pointed at*, which is
//! [`super::deployment`]'s; the mapping over the trace format is a **constant**,
//! for the reasons [`super::runtime`] is one.
//!
//! # Why there is no SDK behind it
//!
//! Resolved q51 settles the SDK question against the dependency by name:
//! "OTLP/JSON over HTTP is a stable spec the hub can emit directly, so the
//! exporter is hand-written and held by a **conformance fixture corpus** (the
//! CEL-corpus discipline applied to span output) rather than by a vendored SDK".
//! So the module imports one `node:` builtin and nothing else, and what holds it
//! honest is `tests/fixtures/otlp-conformance/` — fixtures pairing an envelope
//! and an export context with the exact bytes this module must produce, run
//! through the emitted module itself by `tests/toolchain/otlp-conformance.mjs`
//! and driven from `tests/otlp_conformance.rs`.
//!
//! # The one-way door the amendment holds open
//!
//! Resolved q51's second amendment requires W3C-shaped ids and **stable,
//! documented** resource attributes, so that a later `@opentelemetry/sdk-metrics`
//! adoption correlates with these spans on any collector without touching this
//! exporter. `docs/trace.md` §12.6 is the table, and
//! [`the_resource_attributes_are_the_documented_ones`] reads it back out of the
//! emitted source — a resource attribute renamed here and not there is the
//! quietest way that promise could stop being true.
//!
//! [`the_resource_attributes_are_the_documented_ones`]: tests::the_resource_attributes_are_the_documented_ones

use crate::ir::Ir;

/// The exporter's source, carried in the compiler and emitted verbatim.
const SOURCE: &str = include_str!("js/otlp.ts");

/// The resource attributes every export carries (`docs/trace.md` §12.6).
///
/// A compiler constant so the document and the module cannot drift: the test
/// below reads each of these out of both.
pub const RESOURCE_ATTRIBUTES: &[&str] = &[
    "service.name",
    "service.namespace",
    "service.version",
    "deployment.environment.name",
    "telemetry.sdk.name",
    "telemetry.sdk.language",
    "telemetry.sdk.version",
];

/// Every envelope status the root span's mapping has an arm for
/// (`docs/trace.md` §12.4's first row).
///
/// A compiler constant for [`RESOURCE_ATTRIBUTES`]'s reason, aimed at the
/// silence a *ternary* leaves. The mapping's three arms — OK, ERROR, and the
/// UNSET one a run holding a question gets — are reachable from the trace
/// **format** rather than from today's callers: `shipTrace` settles executions,
/// and `agent-compose run` parks one instead, writing a `status: "interrupted"`
/// document that a reader may hand this exporter later. So the arms are named
/// here, the corpus must reach every one of them
/// (`tests/otlp_conformance.rs`'s `every_root_status_arm_has_a_fixture`), and
/// [`the_root_status_arms_are_the_formats`] holds the list to the union
/// `runtime.ts` publishes — a fourth status added to the format without a
/// mapping would otherwise fall silently into the `else`.
///
/// [`the_root_status_arms_are_the_formats`]: tests::the_root_status_arms_are_the_formats
pub const ENVELOPE_STATUSES: &[&str] = &["completed", "failed", "interrupted"];

/// Every reason `parseTraceparent` refuses an inbound header
/// (`docs/trace.md` §12.3).
///
/// A compiler constant for [`RESOURCE_ATTRIBUTES`]'s reason, aimed at a different
/// silence. An ignored header and a refused one produce the *same* well-formed
/// export — the run gets a trace id of its own either way — so a guard that
/// stopped guarding would be invisible from the export side, and a header a
/// collector would have dropped would start being adopted. So each arm is named
/// here, and `tests/fixtures/traceparent-conformance.json` must carry a case for
/// every one of them: this is the discipline `parse::deploy`'s
/// `every_refusal_arm_answers_some_url` holds the URL rules to, applied to the
/// one wire contract this module reads rather than writes.
pub const TRACEPARENT_REFUSALS: &[&str] = &[
    "absent",
    "too-few-fields",
    "version-not-hex",
    "version-reserved",
    "version-00-extra-fields",
    "trace-id-not-hex",
    "trace-id-wrong-width",
    "trace-id-all-zero",
    "span-id-not-hex",
    "span-id-wrong-width",
    "span-id-all-zero",
    "flags-not-hex",
];

/// How many `return undefined;` statements `parseTraceparent` has.
///
/// The other direction of [`TRACEPARENT_REFUSALS`]: that list says every named
/// arm is exercised, and this says no *unnamed* one was added. A guard is one
/// statement, several of them refuse for two reasons at once (a width and an
/// all-zero id share a line), and the corpus names the reasons — so a new
/// statement here without a new case there is a refusal nothing has ever
/// reached.
pub const TRACEPARENT_GUARDS: usize = 7;

/// `src/otlp.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    super::GeneratedFile {
        path: "src/otlp.ts".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    /// The exporter is a constant: two compositions emit the same bytes after
    /// the provenance line.
    #[test]
    fn the_exporter_is_the_same_module_in_every_project() {
        let one = module(&ir_of("version: \"0.1\"\n")).contents;
        let two = module(&ir_of(
            "version: \"0.1\"\n\
provider.p:\n  kind: openai\n  api_key: ${SOME_KEY}\n\
model.m:\n  provider: provider.p\n  id: some-model\n",
        ))
        .contents;
        let body = |text: &str| {
            text.split_once('\n')
                .expect("every emitted file carries a provenance line")
                .1
                .to_string()
        };
        assert_eq!(body(&one), body(&two));
    }

    /// **The promise resolved q51's second amendment makes is a list of names.**
    ///
    /// A metrics exporter adopted later correlates with these spans by
    /// *resourcing itself the same way*, which is a promise about seven strings
    /// and nothing else. So the seven are read out of the emitted module and out
    /// of the document that publishes them, and a rename that touched one is a
    /// failure here rather than a silent re-identification of every deployment
    /// on somebody's collector.
    #[test]
    fn the_resource_attributes_are_the_documented_ones() {
        let document = include_str!("../../../../docs/trace.md");
        for attribute in RESOURCE_ATTRIBUTES {
            assert!(
                SOURCE.contains(&format!("text(\"{attribute}\"")),
                "`src/otlp.ts` sets no resource attribute `{attribute}`, which \
                 `docs/trace.md` §12.6 promises a later metrics exporter can correlate on"
            );
            assert!(
                document.contains(&format!("`{attribute}`")),
                "`docs/trace.md` does not document the resource attribute `{attribute}`"
            );
        }
        // …and no eighth, which is the half that keeps the list a list: an
        // attribute the module sets and the document does not name is one a
        // reader cannot rely on and a later exporter cannot match.
        let emitted = SOURCE
            .split_once("function resourceAttributes(")
            .expect("`src/otlp.ts` derives its resource attributes in one place")
            .1;
        let body = emitted
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        let set = body.matches("text(\"").count();
        assert_eq!(
            set,
            RESOURCE_ATTRIBUTES.len(),
            "`resourceAttributes` sets {set} attributes and `docs/trace.md` §12.6 documents {}",
            RESOURCE_ATTRIBUTES.len()
        );
    }

    /// **The root's status arms are the trace format's statuses, all of them.**
    ///
    /// [`ENVELOPE_STATUSES`] is what `tests/otlp_conformance.rs` makes the corpus
    /// reach, and this is what stops that list from being a list of its own: it
    /// is read back out of `src/runtime.ts`'s `TraceDocument.status`, which is
    /// the format `docs/trace.md` §2 publishes and the exporter's whole input. A
    /// fourth status added there lands in the mapping's `else` — exported as
    /// UNSET with no arm of its own and no fixture — and the failure here is
    /// what sends its author to §12.4's table instead.
    #[test]
    fn the_root_status_arms_are_the_formats() {
        let runtime = include_str!("js/runtime.ts");
        let union = ENVELOPE_STATUSES
            .iter()
            .map(|status| format!("\"{status}\""))
            .collect::<Vec<_>>()
            .join(" | ");
        assert!(
            runtime.contains(&format!("readonly status: {union};")),
            "`src/runtime.ts`'s `TraceDocument.status` is no longer `{union}`, so \
             `ENVELOPE_STATUSES` names a different set of root-span arms than the format \
             admits (`docs/trace.md` §2, §12.4)"
        );
        // …and the two the mapping names outright are named in it, so a renamed
        // arm is a failure rather than a run silently exported as UNSET.
        for status in ["completed", "failed"] {
            assert!(
                SOURCE.contains(&format!("document.status === \"{status}\"")),
                "`src/otlp.ts` no longer maps the envelope status `{status}` \
                 (`docs/trace.md` §12.4)"
            );
        }
    }

    /// **Every guard in the header parser is one the corpus has a case for.**
    ///
    /// [`TRACEPARENT_GUARDS`] is the count this reads back out of the emitted
    /// source. A guard added here is a refusal `docs/trace.md` §12.3 has to
    /// describe and `tests/fixtures/traceparent-conformance.json` has to reach,
    /// and the two are what this failing sends a reader to.
    #[test]
    fn the_header_parser_has_the_guards_the_corpus_answers() {
        let body = SOURCE
            .split_once("export function parseTraceparent(")
            .expect("`src/otlp.ts` reads an inbound `traceparent` in one place")
            .1
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        let guards = body.matches("return undefined;").count();
        assert_eq!(
            guards, TRACEPARENT_GUARDS,
            "`parseTraceparent` refuses a header in {guards} places and \
             `TRACEPARENT_GUARDS` says {TRACEPARENT_GUARDS}; name the new arm in \
             `TRACEPARENT_REFUSALS`, give it a case in \
             `tests/fixtures/traceparent-conformance.json`, and say what it refuses in \
             `docs/trace.md` §12.3"
        );
    }

    /// **No OpenTelemetry package, and no package at all.**
    ///
    /// Resolved q51's whole trade is that the exporter is hand-emitted; a
    /// dependency arriving here later would be that decision reversed by an
    /// import rather than by a question.
    #[test]
    fn the_exporter_imports_one_builtin_and_nothing_else() {
        // Read as **statements** rather than as lines: an import list long
        // enough to wrap is the shape a line-wise reader silently misses, and
        // this module has one.
        let specifiers: Vec<String> = SOURCE
            .split("\nimport ")
            .skip(1)
            .map(|statement| {
                let quoted = statement
                    .split_once("from \"")
                    .expect("every import names a module")
                    .1;
                quoted.split_once('"').expect("…in quotes").0.to_string()
            })
            .collect();
        assert_eq!(
            specifiers,
            ["node:crypto", "./runtime.ts"],
            "the exporter's imports are the hash it derives ids with and the trace format it \
             reads; anything else is the SDK resolved q51 declined"
        );
    }
}
