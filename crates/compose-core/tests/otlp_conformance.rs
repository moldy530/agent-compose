//! The OTLP conformance corpus: the exporter's answer, pinned byte for byte, and
//! held to the shape `docs/trace.md` §12 publishes.
//!
//! PRD resolved q51 buys a hand-written exporter by paying for it here. The trade
//! it states is explicit — "OTLP/JSON over HTTP is a stable spec the hub can emit
//! directly, so the exporter is hand-written and held by a **conformance fixture
//! corpus** (the CEL-corpus discipline applied to span output) rather than by a
//! vendored SDK" — so what stands between this project and a collector that drops
//! its exports is this file and the fixtures beside it.
//!
//! # The two halves, and why neither is enough alone
//!
//! **The bytes.** Each fixture under `tests/fixtures/otlp-conformance/` pairs a
//! trace envelope and an export context with the exact
//! `ExportTraceServiceRequest` the mapping must produce.
//! `tests/toolchain/otlp-conformance.mjs` runs every one of them through the
//! **emitted** `src/otlp.ts` — the module a deployment really ships, not a port
//! of it — and reports divergences. That is what makes a change to the mapping
//! visible in a diff.
//!
//! Expectations that are *bytes* are regenerated rather than typed
//! (`UPDATE_GOLDENS=1`), which on its own would be a test that agrees with
//! itself. So:
//!
//! **The shape.** This file reads the corpus in Rust and asserts the invariants
//! §12 states as properties rather than as literals: ids are lowercase hex of the
//! right width and never all zero, every span but the root names a parent that is
//! a span of the same export, no two spans share an id, every timestamp is an
//! integer written as a string, every link resolves, and the resource attributes
//! are exactly the documented seven. A regenerated expectation that broke any of
//! those fails here, whatever the runner said.
//!
//! Together they are the CEL corpus's two directions: one implementation answering
//! a fixed corpus, and the corpus itself held to a specification.

#[path = "support/goldens.rs"]
mod goldens;
#[path = "support/toolchain.rs"]
mod toolchain;

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use goldens::{files_under, goldens_root};
use toolchain::{installed, runner};

/// Where the corpus lives.
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/otlp-conformance")
}

/// Every fixture, by file name, oldest-sorted so a failure names a stable one.
fn fixtures() -> Vec<(String, Value)> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus())
        .expect("the otlp-conformance corpus directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "json"))
        .collect();
    files.sort();
    assert!(!files.is_empty(), "the OTLP conformance corpus is empty");
    files
        .into_iter()
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|held| held.to_str())
                .unwrap_or_default()
                .to_string();
            let source = fs::read_to_string(&path).expect("a readable corpus file");
            let value: Value = serde_json::from_str(&source)
                .unwrap_or_else(|error| panic!("{name} is not a JSON object: {error}"));
            (name, value)
        })
        .collect()
}

/// One golden, copied into the toolchain fixture so the runner can import it.
///
/// The exporter is a compiler constant — `codegen::otlp`'s
/// `the_exporter_is_the_same_module_in_every_project` is what says so — so one
/// golden answers the corpus for all of them, exactly as one golden answers the
/// CEL corpus.
fn staged(root: &Path) -> PathBuf {
    let golden = goldens::golden("review-loop");
    let destination = root
        .join("projects/otlp-conformance")
        .join(golden.directory);
    let _ = fs::remove_dir_all(&destination);
    let source = goldens_root().join(golden.directory);
    for relative in files_under(&source) {
        let target = destination.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("a staged path has a parent"))
            .expect("the scratch area is writable");
        fs::copy(source.join(&relative), &target).expect("a golden file is copyable");
    }
    destination
}

/// **The corpus, answered by the module a deployment ships.**
///
/// `UPDATE_GOLDENS=1` rewrites the expectations from what the exporter produced,
/// the way every other byte-for-byte expectation in this repository is
/// maintained; the run after it is what has to pass.
#[test]
fn the_emitted_exporter_answers_the_conformance_corpus() {
    let Some(root) = installed() else {
        return;
    };
    let project = staged(root);
    let mut command = runner("otlp-conformance.mjs");
    command.arg(&project).arg(corpus());
    if std::env::var_os("UPDATE_GOLDENS").is_some() {
        command.arg("--write");
    }
    let output = command.output().expect("bun runs");
    assert!(
        output.status.success(),
        "the OTLP corpus did not run:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answered = String::from_utf8_lossy(&output.stdout);
    assert_eq!(
        answered.trim(),
        "[]",
        "the emitted OTLP exporter answers the shared corpus differently from what it holds; \
         `docs/trace.md` §12 is the mapping and PRD resolved q51 is why this corpus stands in \
         for an SDK. Regenerate with `UPDATE_GOLDENS=1` only after reading the diff."
    );
}

/// Every fixture carries an input **and** the bytes it expects.
///
/// A fixture with no expectation is one the runner would answer with a
/// `no-expectation` divergence — this says the same thing where a reader adding
/// a fixture will meet it, and it is the check the shape assertions below all
/// depend on.
#[test]
fn every_fixture_declares_its_input_and_its_expectation() {
    let mut names: BTreeSet<String> = BTreeSet::new();
    for (file, fixture) in fixtures() {
        for key in ["name", "about", "context", "document", "expected"] {
            assert!(
                fixture.get(key).is_some(),
                "{file} declares no `{key}`: a corpus entry is an input, the bytes it expects, \
                 and a sentence saying what it is for"
            );
        }
        let name = fixture["name"].as_str().expect("a string name").to_string();
        assert!(
            names.insert(name.clone()),
            "two fixtures are named `{name}`; a divergence has to name one file"
        );
    }
}

/// **The shape `docs/trace.md` §12 publishes, over every expectation.**
///
/// The structural half of this corpus. It reads the committed bytes rather than
/// running anything, so it holds equally over an expectation somebody
/// regenerated: an exporter that started minting eight-byte trace ids, reusing a
/// span id, or parenting a span at an id nothing in the export declares would
/// still round-trip through the runner and would fail here.
#[test]
fn every_expectation_is_a_well_formed_export() {
    for (file, fixture) in fixtures() {
        let export = &fixture["expected"];
        let resources = export["resourceSpans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: an export carries `resourceSpans`"));
        assert_eq!(
            resources.len(),
            1,
            "{file}: one execution is one resource (`docs/trace.md` §12)"
        );
        let scopes = resources[0]["scopeSpans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a resource carries `scopeSpans`"));
        assert_eq!(scopes.len(), 1, "{file}: one export is one scope");
        assert_eq!(
            scopes[0]["scope"]["name"], "agent-compose",
            "{file}: the instrumentation scope is this project's"
        );

        // §12.6's promise, read as a list rather than as a set: a later metrics
        // exporter correlates by resourcing itself the same way.
        let resource: Vec<&str> = resources[0]["resource"]["attributes"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a resource carries attributes"))
            .iter()
            .map(|attribute| {
                attribute["key"]
                    .as_str()
                    .unwrap_or_else(|| panic!("{file}: an attribute has a string key"))
            })
            .collect();
        assert_eq!(
            resource,
            compose_core::codegen::otlp::RESOURCE_ATTRIBUTES,
            "{file}: the resource attributes are not `docs/trace.md` §12.6's"
        );

        let spans = scopes[0]["spans"]
            .as_array()
            .unwrap_or_else(|| panic!("{file}: a scope carries spans"));
        assert!(!spans.is_empty(), "{file}: an execution is at least a root");

        let trace = hex(&file, &spans[0]["traceId"], 32);
        let mut ids: BTreeSet<String> = BTreeSet::new();
        let mut roots = 0;
        for span in spans {
            let id = hex(&file, &span["spanId"], 16);
            assert!(
                ids.insert(id.clone()),
                "{file}: two spans share the id `{id}`, which is one span to a collector \
                 (`docs/trace.md` §12.2)"
            );
            assert_eq!(
                hex(&file, &span["traceId"], 32),
                trace,
                "{file}: one execution is one trace"
            );
            for key in ["startTimeUnixNano", "endTimeUnixNano"] {
                let stamp = span[key]
                    .as_str()
                    .unwrap_or_else(|| panic!("{file}: `{key}` is a string (proto3 JSON int64)"));
                assert!(
                    stamp.chars().all(|held| held.is_ascii_digit()) && !stamp.is_empty(),
                    "{file}: `{key}` is `{stamp}`, which is not an integer"
                );
            }
            let kind = span["kind"].as_u64().unwrap_or_else(|| {
                panic!("{file}: a span's `kind` is the enum's number, not its name")
            });
            assert!(
                kind == 1 || kind == 3,
                "{file}: `kind` is {kind}; §12.1 uses internal (1) and client (3)"
            );
            let code = span["status"]["code"].as_u64().unwrap_or_else(|| {
                panic!("{file}: a span's status code is the enum's number, not its name")
            });
            assert!(
                code <= 2,
                "{file}: status code {code} is not one of §12.4's"
            );
            if span.get("parentSpanId").is_none() {
                roots += 1;
            } else {
                hex(&file, &span["parentSpanId"], 16);
            }
            for attribute in span["attributes"]
                .as_array()
                .unwrap_or_else(|| panic!("{file}: a span carries an attribute array"))
            {
                let key = attribute["key"].as_str().expect("a string key");
                assert!(
                    key.starts_with("agentcompose."),
                    "{file}: the span attribute `{key}` is outside this project's namespace \
                     (`docs/trace.md` §12.5)"
                );
            }
        }
        assert!(
            roots <= 1,
            "{file}: {roots} spans name no parent; an export is one tree"
        );

        // Every parent, and every link, resolves to a span of this export — with
        // the one exception §12.3 states: a root parented at the **caller's**
        // span, which is not one of ours.
        let caller = fixture["context"]["parent"]["spanId"].as_str();
        for span in spans {
            if let Some(parent) = span["parentSpanId"].as_str() {
                assert!(
                    ids.contains(parent) || Some(parent) == caller,
                    "{file}: the span `{}` is parented at `{parent}`, which this export does \
                     not declare and no `traceparent` names",
                    span["name"]
                );
            }
            for link in span["links"].as_array().into_iter().flatten() {
                let target = hex(&file, &link["spanId"], 16);
                assert!(
                    ids.contains(&target),
                    "{file}: a link points at `{target}`, which this export does not declare; \
                     a flow-as-tool join links a model call to the dispatch it filed \
                     (`docs/trace.md` §12.2)"
                );
                assert_eq!(
                    hex(&file, &link["traceId"], 32),
                    trace,
                    "{file}: a link leaves this trace"
                );
            }
        }
    }
}

/// **The caller's trace is adopted, not merely noted** (`docs/trace.md` §12.3).
///
/// The fixture that carries a `traceparent` and the one that does not are the
/// same document under the same context but for the header, so what differs
/// between their expectations is exactly what the amendment promises: the trace
/// id becomes the caller's, the root gains the caller's span as its parent, and
/// **every span id is unchanged** — an embedded graph joins its caller's trace
/// rather than becoming a different set of spans inside it.
#[test]
fn an_inbound_traceparent_moves_the_trace_id_and_the_roots_parent_and_nothing_else() {
    let held: BTreeMap<String, Value> = fixtures().into_iter().collect();
    let plain = &held["minimal-run.json"]["expected"];
    let joined = &held["traceparent-parented.json"]["expected"];
    let parent = held["traceparent-parented.json"]["context"]["parent"].clone();

    let spans = |export: &Value| -> Vec<Value> {
        export["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .expect("spans")
            .clone()
    };
    let one = spans(plain);
    let two = spans(joined);
    assert_eq!(one.len(), two.len(), "the two fixtures describe one run");

    assert_eq!(
        two[0]["traceId"], parent["traceId"],
        "the export adopts the caller's trace id outright"
    );
    assert_ne!(
        one[0]["traceId"], two[0]["traceId"],
        "…and the unparented run derives one of its own"
    );
    assert_eq!(
        two[0]["parentSpanId"], parent["spanId"],
        "the root span hangs off the caller's span"
    );
    assert!(
        one[0].get("parentSpanId").is_none(),
        "a run nobody traced into has no parent to name"
    );
    for (unparented, parented) in one.iter().zip(two.iter()) {
        assert_eq!(
            unparented["spanId"], parented["spanId"],
            "a span id is derived from the execution and the record, never from the trace it \
             ends up in (`docs/trace.md` §12.2)"
        );
        assert_eq!(unparented["name"], parented["name"]);
        assert_eq!(unparented["attributes"], parented["attributes"]);
    }
}

/// A hex id of the documented width, never all zero.
fn hex(file: &str, value: &Value, width: usize) -> String {
    let held = value
        .as_str()
        .unwrap_or_else(|| panic!("{file}: an id is a hex string, not {value}"));
    assert_eq!(
        held.len(),
        width,
        "{file}: `{held}` is not {width} hex characters (`docs/trace.md` §12.2)"
    );
    assert!(
        held.chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character)),
        "{file}: `{held}` is not lowercase hex"
    );
    assert!(
        held.chars().any(|character| character != '0'),
        "{file}: `{held}` is all zero, which the W3C trace context forbids"
    );
    held.to_string()
}
