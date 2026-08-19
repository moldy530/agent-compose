//! The trace format, pinned against real runs (`docs/trace.md`).
//!
//! `crates/compose-core/tests/trace_format_inventory.rs` is the other half of
//! this pair, and the two answer different questions. That one reads the emitted
//! `src/runtime.ts` and asks whether the **document** covers what the code
//! declares. This one runs compiled graphs against the mock provider and asks
//! what a reader actually receives — which is not the same question: a field
//! declared and never recorded, a presence rule the document states and the
//! runtime does not keep, and a rename that moved both the declaration and the
//! prose in one commit are all invisible to a source-level check.
//!
//! # What a snapshot here holds
//!
//! The trace **document** a run wrote to disk, with its volatile values
//! redacted: the execution id becomes `<execution-id>` wherever it appears —
//! including inside the idempotency keys, where the remainder is the instance
//! path (grammar §9.4) and is exactly what should stay legible — and the built
//! project's own directory becomes `<project>`. Everything else is kept, because
//! everything else is deterministic: the fixtures script every model answer, and
//! `docs/trace.md` fixes the order of entries, edge decisions and dispatch
//! records so that two runs of one composition produce one document.
//!
//! So an unannounced rename or removal is a diff in a pull request. A snapshot
//! that changes is not by itself a failure — `docs/trace.md` §10.2 lists the
//! changes that are compatible — it is a change to a public surface that has to
//! be looked at, which is what a golden is for.
//!
//! ```sh
//! INSTA_UPDATE=always cargo test -p agent-compose --test trace_format_stability
//! ```
//!
//! # Why these five runs
//!
//! Between them they reach every record type the format has, and every shape of
//! entry: a bounded cycle for guarded edges, budgets and an `else:` escape; a
//! routed fan-out for dispatch records, variants, idempotency keys and a nested
//! subflow trace; a store round trip for read-replay and write-dedupe records; a
//! failover for a model call that was served by its second member; and a spent
//! route for a **failed** run — the shape a reader most often opens a trace for,
//! and the one whose rules (no writes, routing only when routing failed) exist
//! nowhere else.
//!
//! Each goes through `agent-compose run`, so what is snapshotted is the file a
//! reader is handed rather than an in-process value a test could shape for
//! itself.

// See the note on the same line in `tests/compiled_graph_acceptance.rs`: a test
// target is a crate root, so the shared harness is reached by path.
#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use mock_provider::{Client, MockProvider, Outcome, Script, ToolCall};
use serde_json::{Value, json};

/// `model.smart` in every fixture that has one.
const SONNET: &str = "claude-sonnet-4-6";
/// `model.fast`.
const HAIKU: &str = "claude-haiku-4-5";

/// The trace document a run wrote, as a snapshot reads it.
///
/// Two substitutions, and only two, because only two things about a trace differ
/// between runs of one composition:
///
///  * the **execution id**, which is minted per run. It is replaced wherever it
///    appears rather than only at `execution_id`, which is the point: an
///    idempotency key is the id followed by an instance path (grammar §9.4), and
///    substituting the id leaves the path — the part worth reading — in the
///    snapshot;
///  * the **project directory**, which a scratch build puts under a temporary
///    path. Nothing in the document holds one today; it is redacted anyway, so a
///    field that starts carrying a path lands in the snapshot as `<project>`
///    instead of as a value that changes every run.
fn document(run: &harness::Run) -> String {
    let held = run.trace_document();
    let execution = held["execution_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the trace document names its execution: {held}"))
        .to_string();
    // `<project>/.agent-compose/traces/<file>` is where the run wrote it, so the
    // project root is the path with those three components removed.
    let path = run.trace_path();
    let project = std::path::Path::new(&path)
        .ancestors()
        .nth(3)
        .map(|root| root.display().to_string())
        .unwrap_or_default();

    let mut redacted = held;
    redact(&mut redacted, &execution, &project);
    serde_json::to_string_pretty(&redacted).expect("the trace document serializes")
}

/// Replace the two volatile values everywhere in `value`.
fn redact(value: &mut Value, execution: &str, project: &str) {
    match value {
        Value::String(text) => {
            let mut held = text.replace(execution, "<execution-id>");
            if !project.is_empty() {
                held = held.replace(project, "<project>");
            }
            *text = held;
        }
        Value::Array(items) => {
            for item in items {
                redact(item, execution, project);
            }
        }
        Value::Object(fields) => {
            for (_, held) in fields.iter_mut() {
                redact(held, execution, project);
            }
        }
        _ => {}
    }
}

/// A bounded cycle: guarded edges, the budget that ends it, and the `else:`
/// escape (`docs/trace.md` §4).
///
/// The model never approves, so only `max_iterations: 3` can end this run — and
/// the entries are where "what terminated a cycle" is readable: three passes
/// with `budget.used` climbing, then a fourth where the guard still answers
/// `true`, the budget is found spent, and the escape fires.
#[test]
fn a_bounded_cycles_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" })))
            .times(4)
            .matching("a research writer"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "again" })),
        )
        .times(4)
        .matching("meticulous technical reviewer"),
    ]);

    let Some(run) = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A routed fan-out: dispatch records, the variant each item carried, the
/// idempotency key each derived, and a subflow's nested trace
/// (`docs/trace.md` §5, §8).
#[test]
fn a_routed_fan_outs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a report" })),
        )
        .matching("a raw report"),
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "needs_human", "summary": "unclear", "severity": "high" },
                ],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "a patch" }))).matching("a.rs"),
        Script::new(HAIKU, Outcome::structured(json!({ "ticket": "a ticket" })))
            .matching("unclear"),
    ]);

    let Some(run) = harness::run(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A store round trip: a read's recorded answer, a write's key and dedupe flag,
/// and both of PRD 5.8's consumption surfaces (`docs/trace.md` §6).
///
/// The flow's own `store:` nodes supply the `via: "node"` records; the agent in
/// the middle of it calls a **synthesized store tool**, which is the other
/// surface — "recorded as tool calls" in PRD 5.8's own words, and a record the
/// provider transcript alone could not stand in for.
#[test]
fn a_store_using_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    // `agent.grounded` has stores attached, so its tools open a loop: the calls
    // that offer them pin nothing, and the pinned output call ends it.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "prefs_get",
                json!({ "key": "preferences" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    let Some(run) = harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A model call served by its second route member (`docs/trace.md` §7).
///
/// PRD 5.9's own sentence — "served by `model.fast`, fallback #1" — as the
/// record that carries it: the `model.*` the composition named, the member that
/// answered, its ordinal, and the condition the member it replaced refused with.
#[test]
fn a_failed_over_model_calls_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "42" }))),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A run that produced no answer (`docs/trace.md` §9).
///
/// Both members of the route refuse, so the node fails and the run ends. The
/// document is the one a reader opens a trace for: `status: "failed"` with the
/// error that stopped it, and a final entry carrying no `writes`, no `routing`,
/// and the whole spent ladder in `models`.
#[test]
fn a_failed_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::rate_limit()),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.failed();
    insta::assert_snapshot!(document(&run));
}

/// The third delivery surface keeps §1's rule: `serve` reports the version
/// beside the trace, and neither before the run has stopped.
///
/// Snapshotting this one would pin the *app's* report rather than the format —
/// the same entries, wrapped in an execution's status — so what is asserted here
/// is the rule the other four cannot reach: that a surface with an optional
/// `trace` carries the version exactly when it carries entries.
#[test]
fn the_status_route_carries_the_version_beside_its_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let started = app
        .post_json("/direct-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert!(
        finished["trace"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "a settled execution reports the entries it made: {finished}"
    );
    assert_eq!(
        finished["trace_version"],
        json!(1),
        "…and the version that describes them, beside them (`docs/trace.md` §1): \
         {finished}"
    );

    // A `404` is the app's own report about an id it never started, and it
    // carries no trace — so it carries no version either, which is the other
    // half of the rule.
    let unknown = app
        .get("/executions/exec_not_started")
        .expect("the status route answers");
    assert_eq!(unknown.status, 404, "{:?}", unknown.body);
    assert!(
        unknown.json()["trace_version"].is_null(),
        "a report with no trace names no trace version: {:?}",
        unknown.body
    );
}
