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
