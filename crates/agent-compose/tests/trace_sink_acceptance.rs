//! The trace sink, end to end: a real deployment, a real collector socket, and
//! the traces that reach it (grammar §14.5, `docs/trace.md` §1.4, §12, PRD
//! resolved q50, q51).
//!
//! Every test here builds a composition under a **target that declares a
//! `trace_sink:`** and points that sink at a listener this suite owns. Nothing is
//! stubbed on either side: the deploy file is a file, the delivery goes through
//! the journal's ledger, and what the collector receives is the bytes a
//! deployment would really send it.
//!
//! # Why the deploy files are written rather than committed
//!
//! A sink's `url:` is a literal — grammar §4.3 class 3, so no `${ENV}` reaches
//! it, deliberately: the address is written once by an operator rather than
//! resolved per process. Every receiver this suite binds takes a port the
//! operating system chose, so each test copies the fixture and writes the target
//! it needs beside it (`harness::staged_with_deploy`). What that costs is a
//! directory; what it buys is that the thing under test is a target, expressed
//! the way an operator expresses one.
//!
//! # What is asserted, and what is asserted elsewhere
//!
//! Here: that an export **happens**, carries the right bytes to the right
//! address under the right headers, survives a receiver that refuses, and never
//! reaches the run it describes. The OTLP **mapping** is
//! `compose-core`'s `tests/otlp_conformance.rs`, over a corpus that pins exact
//! output; this suite asks only that what arrives is that mapping's shape, so
//! the two do not restate each other.
//!
//! # The second event class
//!
//! PRD resolved q64 gives the sink a second kind of POST: every settled
//! **detached `flow.*` delivery** ships an envelope of its own, headed
//! `detached: true`, `parent_execution` and the grammar §9.4 `idempotency_key`,
//! while its parent's entry keeps exactly the stub `"detached"` record it always
//! had. Detach is legal only under `--target local` (Decision D59), so those
//! tests serve the `detached-trace-sink` fixture under a `deploy/local.yml` —
//! the only place the second class can exist — and the verification the ruling
//! ratified is theirs: two envelopes, the linkage between them, the failed
//! delivery shipping with its outcome, and the re-shipping a recovery does.

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::collections::HashSet;
use std::net::TcpListener;
use std::time::Duration;

use mock_provider::{Client, MockProvider, Outcome, Request, Script};
use serde_json::{Value, json};

/// The fixture every test here drives.
const FIXTURE: &str = "trace-sink";

/// The target name the written deploy files take.
const TARGET: &str = "sink";

/// How long a test waits for a delivery two processes make between them.
const PATIENCE: Duration = Duration::from_secs(30);

/// A schedule short enough for a test, in the shape `docs/durability.md` §3.7
/// documents the override in.
///
/// Three attempts, all due at once, which is what makes "the same delivery is
/// retried" observable inside a test rather than fifteen minutes later. It is a
/// diagnostic surface rather than a deployment knob, and using it here is using
/// it for what it is for.
const FAST_RETRY: &str = "0s,0s,0s";

/// The environment every process in this suite is given.
///
/// `harness::environment` plus the shortened schedule: the fixture needs
/// `${OPS_BIN}` for its two `exec:` nodes and the four credentials the harness
/// seals into every run, and the sink's own identity is two of those four.
fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    let mut held = harness::environment(provider);
    held.push((harness::CALLBACK_RETRY.to_string(), FAST_RETRY.to_string()));
    held
}

/// A deploy file naming this sink, in the shape an operator writes one.
fn sink_target(url: &str, format: Option<&str>, authenticated: bool) -> String {
    let mut written = String::from("version: \"0.1\"\n\ntrace_sink:\n");
    written.push_str(&format!("  url: \"{url}\"\n"));
    if let Some(format) = format {
        written.push_str(&format!("  format: {format}\n"));
    }
    if authenticated {
        written.push_str("  auth:\n");
        written.push_str("    bearer:\n");
        written.push_str(&format!("      token: ${{{}}}\n", harness::DELIVERY_TOKEN));
        written.push_str("    hmac:\n");
        written.push_str(&format!(
            "      secret: ${{{}}}\n",
            harness::DELIVERY_SECRET
        ));
    }
    written
}

/// An address nothing is listening on.
///
/// Bound and released, so the port was free a moment ago and is free now: a
/// connection to it is refused rather than accepted and dropped, which is the
/// "the collector is not there" case rather than "the collector is slow".
fn closed_port() -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    drop(listener);
    format!("http://{address}/v1/traces")
}

/// The one span every export has, and the whole of what a structural assertion
/// needs to find first.
///
/// Found by **what makes it the root** and never by position: the root is the
/// span whose parent is not in this export — absent, because nothing traced into
/// the run, or a caller's span id, because something did (`docs/trace.md` §12.3).
/// Every other span hangs off one emitted beside it.
///
/// Position would be the shorter spelling and the wrong one. `src/otlp.ts`
/// happens to push the root ahead of its walk over the entries, so `spans[0]` is
/// the root today — and a helper leaning on that would go on passing while
/// silently answering with a *child* if that order ever moved, because every span
/// in an export carries the same `traceId` and a child's `parentSpanId` is a
/// well-formed id that is merely the wrong one.
///
/// Exactly one span qualifies, and that is asserted rather than assumed: two
/// would be two trees in one request, which is not a shape this mapping has.
fn root_span(body: &Value) -> &Value {
    let spans = body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .unwrap_or_else(|| panic!("an OTLP export carries spans: {body:#}"));
    let emitted: HashSet<&str> = spans
        .iter()
        .filter_map(|span| span["spanId"].as_str())
        .collect();
    let mut roots =
        spans.iter().filter(
            |span| match span.get("parentSpanId").and_then(Value::as_str) {
                None => true,
                Some(parent) => !emitted.contains(parent),
            },
        );
    let root = roots
        .next()
        .unwrap_or_else(|| panic!("an export has a root span: {body:#}"));
    assert!(
        roots.next().is_none(),
        "an export has exactly one root span: {body:#}"
    );
    root
}

/// **A settled execution's trace reaches the address the target names, signed
/// with the identity it declared** (grammar §14.5, PRD resolved q50).
///
/// The floor of the feature, and four separate claims in one run because they
/// are one delivery: that an export happens at all with nobody having subscribed
/// to it, that it is the envelope `docs/trace.md` §2 specifies rather than a
/// report shaped like one, that it carries the ledger's headers so a receiver
/// can dedupe and order it exactly as it does a `callback:` webhook, and that
/// both halves of the outbound identity are on it — the signature computed here
/// over **the exact bytes that arrived**, because a check that re-serialized the
/// decoded body would agree with an implementation that signed a
/// re-serialization and fail against every receiver written to the published
/// recipe.
#[test]
fn a_settled_trace_reaches_the_sink_signed_with_the_identity_the_target_declared() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-envelope",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, true),
    );
    let Some(project) = harness::scratch_project("sink-envelope") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(Request::post("/greetings").json(&json!({ "subject": "the collector" })))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(
        exported.len(),
        1,
        "one export per settled execution, and no more"
    );
    let delivery = &exported[0];

    // The ledger's headers, which are `docs/grammar.md` §13.3's table: a
    // collector dedupes on the delivery id and orders on the ordinal, exactly as
    // a webhook receiver does.
    assert_eq!(
        delivery.header("x-agentcompose-delivery"),
        Some(format!("{execution}:0").as_str()),
        "{:?}",
        delivery.headers
    );
    assert_eq!(delivery.header("x-agentcompose-ordinal"), Some("0"));
    assert_eq!(
        delivery.header("content-type").map(|held| held
            .split(';')
            .next()
            .unwrap_or_default()
            .trim()
            .to_string()),
        Some("application/json".to_string()),
        "{:?}",
        delivery.headers
    );

    // The identity, both halves, over the bytes that arrived.
    let expected = format!("Bearer {}", harness::credential(harness::DELIVERY_TOKEN));
    assert_eq!(
        delivery.header("authorization"),
        Some(expected.as_str()),
        "{:?}",
        delivery.headers
    );
    let signature = delivery
        .header("x-agentcompose-signature")
        .expect("a signed delivery names its signature");
    assert_eq!(
        signature,
        format!(
            "sha256={}",
            harness::hmac_sha256(
                harness::credential(harness::DELIVERY_SECRET).as_bytes(),
                &delivery.bytes
            )
        ),
        "the signature is not HMAC-SHA256 over the bytes the collector read"
    );

    // The envelope, which is the document the trace file holds.
    let body = &delivery.body;
    assert_eq!(body["trace_version"], 4, "{body:#}");
    assert_eq!(body["flow"], "flow.greet", "{body:#}");
    assert_eq!(body["execution_id"], execution, "{body:#}");
    assert_eq!(body["status"], "completed", "{body:#}");
    let nodes: Vec<&str> = body["entries"]
        .as_array()
        .expect("an envelope carries entries")
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(
        nodes,
        ["say", "note"],
        "the export carries the run's whole trace: {body:#}"
    );
    assert!(
        body.get("trace").is_none() && body.get("outputs").is_none(),
        "the sink ships the **envelope**, not the status route's report: {body:#}"
    );
}

/// **A collector that refuses is retried with the same delivery, and the
/// execution succeeds either way** (`docs/durability.md` §3.7, PRD resolved
/// q50).
///
/// Two claims that are one sentence of the resolution: "a sink outage costs
/// deliveries a retry, never an execution". The first `500` is what makes the
/// retry observable, and the delivery id is what makes it a *retry* rather than
/// a second export — a receiver dedupes on it, so two POSTs under one id are the
/// contract and two ids would be a bug this assertion is written to catch.
#[test]
fn a_collector_that_refuses_once_is_retried_under_one_delivery_id() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    collector.answer_with(&[500]);
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-retry",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-retry") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(Request::post("/greetings").json(&json!({ "subject": "a refusing collector" })))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let arrived = collector.wait_for_event("settled", 2, PATIENCE);
    assert_eq!(
        collector.distinct("settled"),
        [format!("{execution}:0")],
        "two POSTs arrived under two delivery ids, so the second is a second export rather \
         than the retry `docs/durability.md` §3.7 promises: {arrived:?}"
    );
    for delivery in &arrived {
        assert_eq!(
            delivery.bytes, arrived[0].bytes,
            "a retry resends the bytes the intent recorded, or a signature over them means \
             nothing"
        );
    }

    // …and the run itself is untouched by any of it.
    let report = app
        .send(Request::get(format!("/executions/{execution}")))
        .expect("the status route answers");
    assert_eq!(report.status, 200, "{}", report.text());
    assert_eq!(report.json()["status"], "completed", "{}", report.text());

    // The ledger's own account, which is the half the collector cannot show: one
    // row, of the **kind** the export is, ended `delivered` rather than left
    // `pending` for a start that would POST it a fourth time
    // (`docs/durability.md` §3.7).
    let rows = harness::journal_rows(
        &project,
        "SELECT kind, status, attempts FROM deliveries ORDER BY ordinal ASC",
    );
    let held = rows.as_array().expect("the query answers rows");
    assert_eq!(held.len(), 1, "one delivery on this execution: {rows:#}");
    assert_eq!(held[0]["kind"], "trace_sink", "{rows:#}");
    assert_eq!(held[0]["status"], "delivered", "{rows:#}");
    let attempts: Value = serde_json::from_str(
        held[0]["attempts"]
            .as_str()
            .expect("the attempts column is JSON text"),
    )
    .expect("the attempts column parses");
    assert_eq!(
        attempts.as_array().map(Vec::len),
        Some(2),
        "the row records the refused attempt and the one that landed: {attempts:#}"
    );
}

/// **A collector that is not there costs the run nothing** (PRD resolved q50).
///
/// The *fails* half of "never blocks or fails the run it describes": a refused
/// connection is not a failed execution, the exit code is `0`, the outputs are
/// the flow's own and the trace file is still written.
///
/// It is deliberately **not** the *blocks* half, and cannot be: a closed port
/// answers `ECONNREFUSED` at once, so a command that waited on the export before
/// answering would answer just as fast and this test would pass either way. That
/// claim needs a collector that takes the connection and holds it, and it is
/// [`a_collector_that_never_answers_does_not_hold_up_a_runs_report`] below.
#[test]
fn a_collector_that_is_not_there_does_not_fail_a_run() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-absent",
        FIXTURE,
        TARGET,
        &sink_target(&closed_port(), None, false),
    );
    let Some(project) = harness::scratch_project("sink-absent") else {
        return;
    };
    let run = harness::run_target(
        &project,
        &entrypoint,
        TARGET,
        "flow.greet",
        &[("subject", "nobody")],
        &environment(&provider),
    );
    let outputs = run.succeeded().outputs();
    assert_eq!(outputs["greeting"], "hello", "{outputs:#}");
    assert_eq!(outputs["noted"], "seen", "{outputs:#}");
}

/// **The run's answer comes out while its export is still in flight** (PRD
/// resolved q50: the sink "never blocks … the run it describes").
///
/// The *blocks* half, and the only test in this suite that can make it: the
/// collector here takes the connection and never answers, so the one attempt a
/// command makes sits there for the delivery client's whole per-attempt budget —
/// ten seconds, `src/delivery.ts`'s `DELIVERY_TIMEOUT_MS`. Against that budget
/// the ordering in `src/cli.ts` becomes observable rather than merely written
/// down: the report is printed by `executing` and only then is `shipped`
/// awaited, so the answer is on stdout while the POST is outstanding.
///
/// What is asserted is exactly that interleaving, in the order it has to happen:
///
///  1. the complete answer parses off stdout;
///  2. the collector has taken a connection — so the export really is under way,
///     rather than the run having exported nothing at all;
///  3. **two seconds later the command is still going** — it is waiting out the
///     attempt it started after answering.
///
/// Reverse the two lines in `execute` and every step fails at once: the answer
/// would not appear until the attempt timed out, and the process would exit on
/// its heels rather than linger. Two seconds is the slack — a fifth of the
/// budget — that keeps the third step a statement about ordering rather than
/// about how fast a machine is.
///
/// And the run is unharmed: exit `0`, the flow's own outputs, and a ledger row
/// that records the attempt this command really waited out.
///
/// The schedule is **one** attempt rather than [`FAST_RETRY`]'s three, which is
/// this test's own arithmetic rather than a preference: a command attempts every
/// offset already due, so three would be thirty seconds of black hole for a claim
/// ten seconds already makes.
#[test]
fn a_collector_that_never_answers_does_not_hold_up_a_runs_report() {
    let provider = MockProvider::start().expect("a loopback port");
    let hole = harness::Blackhole::start().expect("a loopback socket");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-unanswered",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", hole.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-unanswered") else {
        return;
    };
    let mut once = environment(&provider);
    once.push((harness::CALLBACK_RETRY.to_string(), "0s".to_string()));
    let mut running = harness::run_target_spawned(
        &project,
        &entrypoint,
        TARGET,
        "flow.greet",
        &[("subject", "nobody")],
        &once,
    );

    // 1. The answer, off the pipe of a process that has not exited. The budget is
    //    the command's whole life — it validates, builds and installs before it
    //    runs anything — rather than a claim about how long any of that takes.
    let outputs = running.report(Duration::from_secs(600));
    assert_eq!(outputs["greeting"], "hello", "{outputs:#}");
    assert_eq!(outputs["noted"], "seen", "{outputs:#}");

    // 2. …and the export is under way, which is what makes step 3 about the sink
    //    rather than about a command that happened to be slow to exit.
    harness::until(PATIENCE, || (hole.reached() >= 1).then_some(()));

    // 3. The command is still there, waiting out the attempt it began *after* it
    //    answered.
    std::thread::sleep(Duration::from_secs(2));
    assert!(
        running.still_running(),
        "the run exited while the export it had just started was still unanswered, so the \
         answer above was printed after the delivery rather than before it:\n{}",
        running.said()
    );

    let run = running.finish();
    let outputs = run.succeeded().outputs();
    assert_eq!(outputs["greeting"], "hello", "{outputs:#}");

    // The ledger's account of the attempt the command waited out: one row, of the
    // export's kind, ended by the schedule rather than by an answer — and the
    // detail is the per-attempt budget, which is what the two seconds above were
    // borrowed from.
    let rows = harness::journal_rows(
        &project,
        "SELECT kind, status, attempts FROM deliveries ORDER BY ordinal ASC",
    );
    let held = rows.as_array().expect("the query answers rows");
    assert_eq!(held.len(), 1, "one delivery on this execution: {rows:#}");
    assert_eq!(held[0]["kind"], "trace_sink", "{rows:#}");
    assert_eq!(held[0]["status"], "exhausted", "{rows:#}");
    let attempts: Value = serde_json::from_str(
        held[0]["attempts"]
            .as_str()
            .expect("the attempts column is JSON text"),
    )
    .expect("the attempts column parses");
    assert_eq!(
        attempts.as_array().map(Vec::len),
        Some(1),
        "the one attempt the schedule allowed, and it ended: {attempts:#}"
    );
    assert!(
        attempts[0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("did not answer"),
        "the attempt ended on its own budget rather than on a refusal: {attempts:#}"
    );
}

/// **A `run` under a target that declares a sink exports too** (PRD resolved
/// q50: "the sink applies wherever executions settle under a target that
/// declares it — `run` included, not just `serve`").
///
/// The clause that makes the sink a property of the *target* rather than of a
/// served process, and the one a delivery worker living inside the app could not
/// have kept. The command builds the project, runs one execution to completion
/// and exits; what the collector holds afterwards is the whole of the claim.
#[test]
fn a_run_under_a_target_that_declares_a_sink_exports_its_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-run",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-run") else {
        return;
    };
    let run = harness::run_target(
        &project,
        &entrypoint,
        TARGET,
        "flow.greet",
        &[("subject", "a one-shot run")],
        &environment(&provider),
    );
    run.succeeded();

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    let body = &exported[0].body;
    assert_eq!(body["flow"], "flow.greet", "{body:#}");
    assert_eq!(body["status"], "completed", "{body:#}");
    assert_eq!(
        body["trace_version"], 4,
        "a command's export is the same envelope a served one ships: {body:#}"
    );
    let nodes: Vec<&str> = body["entries"]
        .as_array()
        .expect("an envelope carries entries")
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(nodes, ["say", "note"], "{body:#}");
}

/// **A run that produced no answer exports the trace it made** (`docs/trace.md`
/// §9, §12.4).
///
/// The export is of every execution that *settles*, which includes the ones that
/// settle badly — and it is the export an operator most wants, because a failed
/// run's trace is the one they came to the collector for. The envelope's
/// `status` and `error` are what a receiver routes on.
#[test]
fn a_failed_execution_exports_the_trace_it_made() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-failed",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-failed") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let started = app
        .send(Request::post("/refusals").json(&json!({ "subject": "a run that fails" })))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    let body = &exported[0].body;
    assert_eq!(body["status"], "failed", "{body:#}");
    assert!(
        body["error"].as_str().is_some_and(|held| !held.is_empty()),
        "a failed export names what stopped the run: {body:#}"
    );
    let entries = body["entries"].as_array().expect("entries");
    assert_eq!(
        entries.len(),
        1,
        "the aborting node's entry is the trace a failed run made: {body:#}"
    );
    assert_eq!(entries[0]["node"], "fail", "{body:#}");
    assert_eq!(entries[0]["outcome"], "failed", "{body:#}");
}

/// **`format: otlp` delivers an `ExportTraceServiceRequest`** (PRD resolved q51,
/// `docs/trace.md` §12).
///
/// What is asserted here is the *shape on the wire* — one resource, one scope,
/// spans with hex ids and string timestamps, the root named for the flow, an
/// entry span per node — and deliberately not the mapping's every field:
/// `compose-core`'s `tests/otlp_conformance.rs` pins that byte for byte over a
/// corpus. This is the claim that corpus cannot make, which is that the bytes
/// really leave a deployment and arrive at a collector's socket.
#[test]
fn an_otlp_sink_receives_a_well_formed_export_trace_service_request() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-otlp",
        FIXTURE,
        TARGET,
        &sink_target(
            &format!("{}/v1/traces", collector.base_url),
            Some("otlp"),
            false,
        ),
    );
    let Some(project) = harness::scratch_project("sink-otlp") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let started = app
        .send(Request::post("/greetings").json(&json!({ "subject": "an OTLP collector" })))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    let body = &exported[0].body;
    assert!(
        body.get("entries").is_none() && body.get("trace_version").is_none(),
        "`format: otlp` ships spans rather than the envelope: {body:#}"
    );
    let resources = body["resourceSpans"]
        .as_array()
        .unwrap_or_else(|| panic!("an OTLP export carries `resourceSpans`: {body:#}"));
    assert_eq!(resources.len(), 1, "{body:#}");
    let scopes = resources[0]["scopeSpans"]
        .as_array()
        .unwrap_or_else(|| panic!("a resource carries `scopeSpans`: {body:#}"));
    assert_eq!(scopes.len(), 1, "{body:#}");
    assert_eq!(scopes[0]["scope"]["name"], "agent-compose", "{body:#}");

    let attributes: Vec<&str> = resources[0]["resource"]["attributes"]
        .as_array()
        .expect("resource attributes")
        .iter()
        .filter_map(|attribute| attribute["key"].as_str())
        .collect();
    assert_eq!(
        attributes,
        compose_core::codegen::otlp::RESOURCE_ATTRIBUTES,
        "the resource attributes a deployment really sends are not the documented ones \
         (`docs/trace.md` §12.6)"
    );
    let target = resources[0]["resource"]["attributes"]
        .as_array()
        .expect("resource attributes")
        .iter()
        .find(|attribute| attribute["key"] == "deployment.environment.name")
        .expect("the environment attribute");
    assert_eq!(
        target["value"]["stringValue"], TARGET,
        "the export names the target it was built for: {body:#}"
    );

    let spans = scopes[0]["spans"].as_array().expect("spans");
    assert_eq!(spans.len(), 3, "one root and one span per entry: {body:#}");
    let names: Vec<&str> = spans
        .iter()
        .filter_map(|span| span["name"].as_str())
        .collect();
    assert_eq!(names, ["flow.greet", "say", "note"], "{body:#}");

    let root = &spans[0];
    assert!(
        root.get("parentSpanId").is_none(),
        "a run nobody traced into has no parent: {root:#}"
    );
    for span in spans {
        let trace_id = span["traceId"].as_str().expect("a hex trace id");
        let span_id = span["spanId"].as_str().expect("a hex span id");
        assert_eq!(trace_id.len(), 32, "{span:#}");
        assert_eq!(span_id.len(), 16, "{span:#}");
        assert!(
            trace_id
                .chars()
                .all(|held| held.is_ascii_digit() || ('a'..='f').contains(&held)),
            "{span:#}"
        );
        assert!(
            span["startTimeUnixNano"]
                .as_str()
                .is_some_and(|held| held.chars().all(|one| one.is_ascii_digit())),
            "a timestamp is an integer written as a string (proto3 JSON): {span:#}"
        );
    }
    let carried = root["attributes"]
        .as_array()
        .expect("root attributes")
        .iter()
        .find(|attribute| attribute["key"] == "agentcompose.execution.id")
        .expect("the execution id attribute");
    assert_eq!(carried["value"]["stringValue"], execution, "{root:#}");
}

/// **A caller's `traceparent` becomes the exported root's parent** (PRD resolved
/// q51's first amendment, `docs/trace.md` §12.3).
///
/// The door that makes an embedded graph joinable: a request carrying a W3C
/// `traceparent` exports **into** its caller's trace rather than beside it. Both
/// halves are asserted in one run, because the second is only meaningful against
/// the first — a header that is not valid is ignored silently, which is the W3C
/// behaviour, so the second execution's export has a trace id of its own and no
/// parent at all.
#[test]
fn an_inbound_traceparent_becomes_the_exported_roots_parent_and_a_bad_one_is_ignored() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-traceparent",
        FIXTURE,
        TARGET,
        &sink_target(
            &format!("{}/v1/traces", collector.base_url),
            Some("otlp"),
            false,
        ),
    );
    let Some(project) = harness::scratch_project("sink-traceparent") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let caller_trace = "4bf92f3577b34da6a3ce929d0e0e4736";
    let caller_span = "00f067aa0ba902b7";
    let started = app
        .send(
            Request::post("/greetings")
                .json(&json!({ "subject": "a caller with a trace" }))
                .header("traceparent", format!("00-{caller_trace}-{caller_span}-01")),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    let root = root_span(&exported[0].body);
    assert_eq!(
        root["traceId"], caller_trace,
        "the export adopts the caller's trace: {root:#}"
    );
    assert_eq!(
        root["parentSpanId"], caller_span,
        "…and hangs off the caller's span: {root:#}"
    );

    // The same route, with a header no parser may read.
    let refused = app
        .send(
            Request::post("/greetings")
                .json(&json!({ "subject": "a caller with a broken header" }))
                .header("traceparent", "not-a-traceparent"),
        )
        .expect("the trigger's route answers");
    assert_eq!(
        refused.status,
        202,
        "a malformed `traceparent` is not a bad request: {}",
        refused.text()
    );
    let both = collector.wait_for_event("settled", 2, PATIENCE);
    let second = root_span(&both[1].body);
    assert_ne!(
        second["traceId"], caller_trace,
        "a header that is not valid is ignored, so the run gets a trace of its own: {second:#}"
    );
    assert!(
        second.get("parentSpanId").is_none(),
        "…and no parent at all: {second:#}"
    );
}

/// **A row a command left behind is finished by the next `serve`**
/// (`docs/durability.md` §3.7, §6.1).
///
/// The other half of the `run` clause, and the reason a command works only what
/// is *due* rather than the whole schedule: an export whose collector was
/// refusing when the command exited stays `pending` on the ledger with the
/// attempts it really made, and the next start of the same project picks it up
/// from the offset it reached. That path is a sink row reaching the resume walk,
/// where a callback row would be matched against its trigger's allowlist and a
/// sink row is signed by the deployment instead — so it is worth an execution
/// rather than an argument.
///
/// The two processes run under **different schedules**, which is what makes it a
/// test rather than a wait: the command's names one offset that is due and one
/// that is a minute away, so it makes exactly one attempt and leaves; the start's
/// names two that are due, so it takes the second one at once.
#[test]
fn an_export_a_command_could_not_finish_is_picked_up_by_the_next_serve() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    // One refusal, then the collector is well again — so what decides whether
    // the trace ever lands is whether the second process picks the row up.
    collector.answer_with(&[500]);
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-resumed",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-resumed") else {
        return;
    };
    let mut command = harness::environment(&provider);
    command.push((harness::CALLBACK_RETRY.to_string(), "0s,60s".to_string()));
    let run = harness::run_target(
        &project,
        &entrypoint,
        TARGET,
        "flow.greet",
        &[("subject", "a command that could not finish")],
        &command,
    );
    run.succeeded();

    let refused = collector.wait_for_event("settled", 1, PATIENCE);
    let execution = refused[0]
        .header("x-agentcompose-delivery")
        .and_then(|held| held.split_once(':').map(|(id, _)| id.to_string()))
        .expect("a delivery names its execution");
    let owed = harness::journal_rows(
        &project,
        "SELECT kind, status FROM deliveries ORDER BY ordinal ASC",
    );
    assert_eq!(owed[0]["kind"], "trace_sink", "{owed:#}");
    assert_eq!(
        owed[0]["status"], "pending",
        "a command leaves the offsets it will not be here for on the row: {owed:#}"
    );

    // The same project directory, so the same journal — and a schedule whose
    // second offset is due now.
    let mut started = harness::environment(&provider);
    started.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let Some(_served) = harness::serve_target_into(&project, &entrypoint, TARGET, &started) else {
        return;
    };

    let arrived = collector.wait_for_event("settled", 2, PATIENCE);
    assert_eq!(
        collector.distinct("settled"),
        [format!("{execution}:0")],
        "the start sent a second export rather than finishing the one the command left: \
         {arrived:?}"
    );
    let finished = harness::journal_rows(
        &project,
        "SELECT kind, status FROM deliveries ORDER BY ordinal ASC",
    );
    assert_eq!(
        finished[0]["status"], "delivered",
        "the row a command left `pending` was never finished: {finished:#}"
    );
}

/// **A `callback:` webhook and a trace export are two rows on one ledger**
/// (`docs/durability.md` §3.7).
///
/// The consequence of making the sink a delivery *kind* rather than a second
/// ledger, and the one place the two are observable together: one execution
/// settles, two POSTs go out to two addresses under consecutive ordinals, and
/// each carries the document its own surface specifies. A guard that asked
/// "is there a `settled` row already?" without asking which kind would have
/// silenced one of them, which is exactly what this catches.
#[test]
fn a_callback_and_a_trace_export_are_two_deliveries_of_one_settle() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let subscriber = harness::Receiver::start().expect("a loopback subscriber");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-and-callback",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-and-callback") else {
        return;
    };
    let environment = environment(&provider);
    let Some(served) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(Request::post("/watched-greetings").json(&json!({
            "subject": "a subscriber and a collector",
            "callback_url": format!("{}/done", subscriber.base_url),
        })))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let pushed = subscriber.wait_for_event("settled", 1, PATIENCE);
    let exported = collector.wait_for_event("settled", 1, PATIENCE);

    // The webhook's body is the status route's report; the export's is the
    // envelope. Two documents, two surfaces, one execution.
    assert_eq!(
        pushed[0].body["execution_id"], execution,
        "{:#}",
        pushed[0].body
    );
    assert_eq!(
        pushed[0].body["outputs"]["greeting"], "hello",
        "{:#}",
        pushed[0].body
    );
    assert_eq!(
        exported[0].body["execution_id"], execution,
        "{:#}",
        exported[0].body
    );
    assert!(
        exported[0].body.get("outputs").is_none(),
        "an export is the envelope, which carries no outputs: {:#}",
        exported[0].body
    );

    // Consecutive ordinals on one execution's ledger, the webhook first because
    // it is journaled first.
    assert_eq!(
        pushed[0].header("x-agentcompose-delivery"),
        Some(format!("{execution}:0").as_str())
    );
    assert_eq!(
        exported[0].header("x-agentcompose-delivery"),
        Some(format!("{execution}:1").as_str())
    );
}

/// **A generation that journaled the webhook and died still exports on
/// recovery** (`docs/trace.md` §1.4, `docs/durability.md` §3.7, §6.1).
///
/// The other side of the coin the test above turns over. A settle under a target
/// declaring both a `callback:` and a `trace_sink:` writes **two** rows, one
/// after the other, while the lifecycle row is still open — and a process killed
/// between them leaves an execution that is still `open` with only the webhook
/// down. The start that recovers it replays it back to the same hook, and the
/// question that hook asks there decides everything: a guard reading "is there a
/// `settled` row already?" would find the dead generation's webhook, conclude the
/// settle was made, and return — and that execution's trace would never be
/// exported by anything, ever, because `resumeDeliveries` can only finish rows
/// that exist. Silent, permanent, and against §1.4's "a recovered execution's"
/// alike.
///
/// **The state is made rather than raced**, which is the only way a test reaches
/// it: a real settle is journaled, and then the journal is put back into the
/// shape the kill would have left it in — the export row removed, the lifecycle
/// row reopened. That is what `journal_sql` is for here as elsewhere, and it is
/// honest because both halves of the state are what a generation really wrote.
///
/// Two claims come out of it, and they are opposite guards rather than one: the
/// export is journaled and sent by the second generation, and the webhook is
/// **not journaled again** — a second `settled` row would be a second delivery
/// id, which is a subscriber told that one execution finished twice. That is
/// what asking about the delivery's *kind* buys, and it is why removing the
/// early return did not cost it.
#[test]
fn a_generation_that_journaled_only_the_webhook_still_exports_on_recovery() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let subscriber = harness::Receiver::start().expect("a loopback subscriber");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-half-journaled",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, false),
    );
    let Some(project) = harness::scratch_project("sink-half-journaled") else {
        return;
    };
    let environment = environment(&provider);

    let execution;
    {
        let Some(first) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
        else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .send(Request::post("/watched-greetings").json(&json!({
                "subject": "a generation that got half way",
                "callback_url": format!("{}/done", subscriber.base_url),
            })))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // Both rows really down and both deliveries really made, so what the
        // surgery below removes is a row this deployment wrote rather than one
        // this test invented.
        subscriber.wait_for_event("settled", 1, PATIENCE);
        collector.wait_for_event("settled", 1, PATIENCE);
    }

    // The state a process killed between the two intents leaves behind.
    harness::journal_sql(
        &project,
        &format!(
            "DELETE FROM deliveries WHERE execution = '{execution}' AND kind = 'trace_sink';\n\
             UPDATE executions SET status = 'open', ended_at = NULL WHERE id = '{execution}';"
        ),
    );
    let halfway = harness::journal_rows(
        &project,
        &format!(
            "SELECT kind FROM deliveries WHERE execution = '{execution}' ORDER BY ordinal ASC"
        ),
    );
    assert_eq!(
        halfway.as_array().map(Vec::len),
        Some(1),
        "the surgery leaves exactly the webhook: {halfway:#}"
    );
    assert_eq!(halfway[0]["kind"], "callback", "{halfway:#}");

    let Some(_second) = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)
    else {
        return;
    };

    // The claim: a second export arrives — the recovered execution's — and the
    // journal holds its row again.
    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    assert_eq!(
        exported[1].body["execution_id"], execution,
        "the recovered execution's trace is the one that shipped: {:#}",
        exported[1].body
    );
    let finished = harness::journal_rows(
        &project,
        &format!(
            "SELECT kind, status FROM deliveries WHERE execution = '{execution}' \
             ORDER BY ordinal ASC"
        ),
    );
    assert_eq!(finished[1]["kind"], "trace_sink", "{finished:#}");
    assert_eq!(
        finished[1]["status"], "delivered",
        "the export the recovery journaled was never sent: {finished:#}"
    );

    // …and the webhook the dead generation already journaled is **not** journaled
    // a second time. Asserted on the ledger and on the delivery ids rather than
    // on the count of POSTs, because the count is not the promise: a row the kill
    // caught between its attempt and its outcome is `pending`, and the resume walk
    // finishing it is at-least-once delivery working (a receiver dedupes on the
    // id). What would be the bug is a *second* `settled` row — a second id — which
    // is a receiver being told one execution finished twice.
    assert_eq!(
        finished.as_array().map(Vec::len),
        Some(2),
        "one webhook and one export, and no more: {finished:#}"
    );
    assert_eq!(finished[0]["kind"], "callback", "{finished:#}");
    assert_eq!(
        subscriber.distinct("settled"),
        [format!("{execution}:0")],
        "a settle is announced once per execution across generations: {:?}",
        subscriber.distinct("settled")
    );
}

/// **A sink credential set to nothing refuses the app at launch, naming the
/// variable** (grammar §13.3, §14.5 rule 3).
///
/// `trace_sink.auth:` is §13.3's outbound block unchanged, and §13.3 is explicit
/// about what an empty credential is: not a missing setting but an open door,
/// refused at launch. `src/env.ts` counts `TRACE_SINK_TOKEN=` as present — §4.3's
/// own rule, and the right one for a `base_url:` — so without a check of its own
/// the deployment would serve, and every trace it ever exported would go out
/// under an `Authorization: Bearer ` with nothing after it. The collector answers
/// `401`, the deliveries exhaust on the ledger, and nothing in a run says so:
/// exactly the silence at-least-once delivery is built to keep.
///
/// Asserted with the sink's `hmac:` half emptied as well as its `bearer:` half,
/// because a signature over a key of no bytes is the worse of the two and the
/// quieter: it is a header a receiver can verify, computed with nothing.
#[test]
fn a_sink_credential_set_to_nothing_refuses_the_app_at_launch() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-blank-credential",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, true),
    );
    let Some(project) = harness::scratch_project("sink-blank-credential") else {
        return;
    };
    for blanked in [harness::DELIVERY_TOKEN, harness::DELIVERY_SECRET] {
        let mut held = environment(&provider);
        for entry in &mut held {
            if entry.0 == blanked {
                entry.1 = String::new();
            }
        }
        let refused = harness::serve_refused_target(&project, &entrypoint, TARGET, &held);
        let said = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(
            refused.status.code(),
            Some(2),
            "a variable the deployment has to fix is a usage error: {said}"
        );
        assert!(
            said.contains(blanked),
            "…and the refusal names what to set: {said}"
        );
        assert!(
            String::from_utf8_lossy(&refused.stdout).is_empty(),
            "nothing was served: a readiness line would mean the routes were mounted: {}",
            String::from_utf8_lossy(&refused.stdout)
        );
    }
}

/// **A `run` whose sink credential resolved to nothing leaves the export in the
/// journal rather than signing it with nothing** (grammar §13.3, §14.5).
///
/// The launch refusal above is `serve`'s, and a command has no launch to put one
/// in — but PRD resolved q50 puts the sink "wherever executions settle, `run`
/// included", so a command really does spend this credential. Three things are
/// therefore true at once and each is a separate half of the resolution: the run
/// **succeeds** and prints its answer, because a credential is not an outcome; the
/// collector is sent **nothing**, because a delivery signed with nothing is worse
/// than a delivery not made; and the row is left **`pending`** with no attempt
/// against it, so the next `serve` — which will refuse to start until the
/// variable is set — is what finally sends it.
#[test]
fn a_run_whose_sink_credential_is_blank_journals_the_export_and_sends_nothing() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "sink-blank-run",
        FIXTURE,
        TARGET,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None, true),
    );
    let Some(project) = harness::scratch_project("sink-blank-run") else {
        return;
    };
    let mut held = environment(&provider);
    for entry in &mut held {
        if entry.0 == harness::DELIVERY_TOKEN {
            entry.1 = String::new();
        }
    }

    let run = harness::run_target(
        &project,
        &entrypoint,
        TARGET,
        "flow.greet",
        &[("subject", "a token that expanded to nothing")],
        &held,
    );
    run.succeeded();
    assert_eq!(
        run.outputs(),
        json!({ "greeting": "hello", "noted": "seen" }),
        "{}",
        run.stderr()
    );
    assert!(
        run.stderr().contains(harness::DELIVERY_TOKEN),
        "the command says which variable stopped the export: {}",
        run.stderr()
    );

    let owed = harness::journal_rows(
        &project,
        "SELECT kind, status, attempts FROM deliveries ORDER BY ordinal ASC",
    );
    assert_eq!(owed[0]["kind"], "trace_sink", "{owed:#}");
    assert_eq!(
        owed[0]["status"], "pending",
        "the export is left for a start that can sign it: {owed:#}"
    );
    assert_eq!(
        owed[0]["attempts"], "[]",
        "…with no attempt spent against its schedule: {owed:#}"
    );
    assert!(
        collector.of_event("settled").is_empty(),
        "a delivery signed with nothing was sent anyway: {:?}",
        collector.of_event("settled")
    );
}

// ---------------------------------------------------------------------------
// The second event class: a detached `flow.*` delivery's own envelope
// ---------------------------------------------------------------------------

/// The fixture the second event class is served from.
const DETACHED_FIXTURE: &str = "detached-trace-sink";

/// The one target `detach: true` is legal under (grammar 8.6 rule 7, Decision
/// D59), and so the one target the second event class exists under.
const LOCAL: &str = "local";

/// The provider-native id `model.local` resolves to in that fixture.
const REVIEWER_MODEL: &str = "qwen3-coder-30b";

/// What [`served_detached`] hands back: the staged composition (whose
/// entrypoint is only valid while it lives), that entrypoint, the project
/// directory a restart serves again, and the running app.
type DetachedServe = (
    harness::Scratch,
    std::path::PathBuf,
    std::path::PathBuf,
    harness::Served,
);

/// Stage and serve the detached fixture under `deploy/local.yml`, naming a sink.
fn served_detached(
    purpose: &str,
    provider: &MockProvider,
    collector: &harness::Receiver,
    format: Option<&str>,
) -> Option<DetachedServe> {
    let (composition, entrypoint) = harness::staged_with_deploy(
        purpose,
        DETACHED_FIXTURE,
        LOCAL,
        &sink_target(&format!("{}/v1/traces", collector.base_url), format, false),
    );
    let project = harness::scratch_project(purpose)?;
    let served = harness::serve_target_into(&project, &entrypoint, LOCAL, &environment(provider))?;
    Some((composition, entrypoint, project, served))
}

/// Start one execution through `route` and answer its id.
fn started(app: &Client, route: &str) -> String {
    let answered = app
        .send(Request::post(route).json(&json!({ "subjects": ["the collector"] })))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    answered.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string()
}

/// Split envelope-format exports into the executions' own and the detached
/// deliveries', by the head that tells them apart (`docs/trace.md` §2).
fn by_class(
    exported: &[harness::Delivered],
) -> (Vec<&harness::Delivered>, Vec<&harness::Delivered>) {
    exported
        .iter()
        .partition(|delivered| delivered.body.get("detached").is_none())
}

/// The stub record a detached dispatch leaves on its map node's entry.
fn stub_record<'a>(parent: &'a Value, node: &str) -> &'a Value {
    let entry = parent["entries"]
        .as_array()
        .expect("an envelope carries entries")
        .iter()
        .find(|entry| entry["node"] == node)
        .unwrap_or_else(|| panic!("the parent's trace has a `{node}` entry: {parent:#}"));
    let dispatches = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("a map node's entry carries its dispatches: {entry:#}"));
    assert_eq!(dispatches.len(), 1, "one subject, one dispatch: {entry:#}");
    &dispatches[0]
}

/// One span attribute's value, by key.
fn attribute(span: &Value, key: &str) -> Option<Value> {
    span["attributes"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|held| held["key"] == key)
        .map(|held| held["value"].clone())
}

/// **A settled detached delivery ships its own envelope, and its parent's entry
/// does not move an inch** (PRD resolved q64, `docs/trace.md` §1.4, §2, §5.1).
///
/// The ruling's verification, both halves in one run. The parent's export is the
/// one it always was: its map node's entry carries exactly the stub `"detached"`
/// record — `attempts: 0`, no `inner` — and no model call, because the parent
/// makes none and the delivery's is not the parent's (D94). The delivery's own
/// envelope is the second POST: headed `detached: true`, `parent_execution` and
/// the grammar §9.4 `idempotency_key`, which is **byte for byte** the stub
/// record's key — the join between the two is string equality — and carrying the
/// delivery's own trace with its model call on it.
///
/// Both ride the parent's ledger as two deliveries of one execution: two ids,
/// both `settled` events, so a receiver's dedupe on the delivery id cannot fold
/// the one into the other.
#[test]
fn a_settled_detached_delivery_ships_its_own_envelope_beside_its_parents() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, _project, served)) =
        served_detached("sink-detached", &provider, &collector, None)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/reviews");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, deliveries) = by_class(&exported);
    assert_eq!(
        (parents.len(), deliveries.len()),
        (1, 1),
        "the sink received the execution's export and the delivery's envelope, one each: \
         {exported:#?}"
    );
    let parent = &parents[0].body;
    let delivery = &deliveries[0].body;

    // The parent's export: the first event class, unchanged.
    assert_eq!(parent["execution_id"], execution, "{parent:#}");
    assert_eq!(parent["flow"], "flow.dispatch", "{parent:#}");
    assert_eq!(parent["status"], "completed", "{parent:#}");
    for head in ["detached", "parent_execution", "idempotency_key"] {
        assert!(
            parent.get(head).is_none(),
            "`{head}` heads a detached delivery's envelope and no other: {parent:#}"
        );
    }
    let stub = stub_record(parent, "review");
    assert_eq!(stub["outcome"], "detached", "{stub:#}");
    assert_eq!(stub["attempts"], 0, "{stub:#}");
    assert_eq!(stub["target"], "flow.review", "{stub:#}");
    assert!(
        stub.get("inner").is_none(),
        "the stub record is the parent's whole account of the dispatch (`docs/trace.md` \
         §5.1): {stub:#}"
    );
    for entry in parent["entries"].as_array().expect("entries") {
        for collected in ["models", "stores", "toolDispatches", "harness", "inner"] {
            assert!(
                entry.get(collected).is_none(),
                "the parent made no `{collected}` of its own, so one on its `{}` entry is the \
                 delivery's leaking into its parent's collector: {parent:#}",
                entry["node"]
            );
        }
    }

    // The delivery's own envelope: the second event class.
    let key = stub["idempotencyKey"]
        .as_str()
        .expect("a dispatch record carries its key");
    assert_eq!(
        key,
        format!("{execution}/review/0/0"),
        "grammar §9.4's key for item 0 of map node `review` on its first traversal"
    );
    assert_eq!(delivery["trace_version"], 4, "{delivery:#}");
    assert_eq!(delivery["detached"], true, "{delivery:#}");
    assert_eq!(delivery["parent_execution"], execution, "{delivery:#}");
    assert_eq!(
        delivery["idempotency_key"], key,
        "the join between the parent's stub record and this envelope is string equality on \
         the key: {delivery:#}"
    );
    assert_eq!(
        delivery["execution_id"], execution,
        "a detached delivery is not an execution of its own, so its entries belong to the one \
         it ran under: {delivery:#}"
    );
    assert_eq!(delivery["flow"], "flow.review", "{delivery:#}");
    assert_eq!(delivery["status"], "completed", "{delivery:#}");
    assert!(delivery.get("error").is_none(), "{delivery:#}");
    let head = String::from_utf8_lossy(&deliveries[0].bytes);
    assert!(
        head.starts_with(&format!(
            "{{\"trace_version\":4,\"detached\":true,\"parent_execution\":\"{execution}\",\
             \"idempotency_key\":\"{key}\","
        )),
        "the envelope is **headed** by the version and the linkage, in that order: {head}"
    );
    let entries = delivery["entries"]
        .as_array()
        .expect("a detached envelope carries entries");
    assert_eq!(entries.len(), 1, "{delivery:#}");
    let judged = &entries[0];
    assert_eq!(judged["node"], "judge", "{judged:#}");
    assert_eq!(judged["flow"], "flow.review", "{judged:#}");
    assert_eq!(
        judged["step"], 1,
        "an instance numbers its own steps: {judged:#}"
    );
    assert_eq!(judged["outcome"], "completed", "{judged:#}");
    let models = judged["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the delivery's model call is on its own entry: {judged:#}"));
    assert_eq!(models.len(), 1, "{judged:#}");
    assert_eq!(models[0]["model"], "model.local", "{judged:#}");
    assert_eq!(models[0]["servedBy"], "model.local", "{judged:#}");

    // Two deliveries on the parent's ledger, told apart by id.
    let ids: Vec<&str> = exported
        .iter()
        .map(|delivered| {
            delivered
                .header("x-agentcompose-delivery")
                .expect("a delivery names its id")
        })
        .collect();
    assert_ne!(ids[0], ids[1], "two envelopes are two deliveries: {ids:?}");
    for id in &ids {
        assert!(
            id.starts_with(&format!("{execution}:")),
            "both ride the ledger of the execution the delivery ran under: {ids:?}"
        );
    }
    assert_eq!(
        collector.distinct("settled").len(),
        2,
        "no third export: {:?}",
        collector.distinct("settled")
    );
}

/// **A delivery that failed ships too, its outcome on the envelope** (PRD
/// resolved q64).
///
/// The debugging story the ruling says is strongest exactly here: the review ran
/// and made its model call, the filing step failed, and none of it touched the
/// parent — which settled `completed`, because nothing a detached dispatch does
/// can fail the enclosing flow instance (grammar §8.6 rule 7). The envelope says
/// `status: "failed"`, names what stopped the delivery, and carries both of the
/// delivery's entries, the aborting one last (`docs/trace.md` §9).
#[test]
fn a_detached_delivery_that_failed_ships_its_envelope_with_its_outcome() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, _project, served)) =
        served_detached("sink-detached-failed", &provider, &collector, None)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/filings");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, deliveries) = by_class(&exported);
    assert_eq!((parents.len(), deliveries.len()), (1, 1), "{exported:#?}");
    let parent = &parents[0].body;
    let delivery = &deliveries[0].body;

    assert_eq!(
        parent["status"], "completed",
        "a delivery's failure is not its parent's: {parent:#}"
    );
    let stub = stub_record(parent, "review");
    assert_eq!(
        stub["outcome"], "detached",
        "the join never observes a detached delivery's outcome (D94): {stub:#}"
    );
    assert!(stub.get("error").is_none(), "{stub:#}");

    assert_eq!(delivery["detached"], true, "{delivery:#}");
    assert_eq!(delivery["parent_execution"], execution, "{delivery:#}");
    assert_eq!(
        delivery["idempotency_key"], stub["idempotencyKey"],
        "{delivery:#}"
    );
    assert_eq!(delivery["flow"], "flow.review_and_file", "{delivery:#}");
    assert_eq!(delivery["status"], "failed", "{delivery:#}");
    assert!(
        delivery["error"]
            .as_str()
            .is_some_and(|held| !held.is_empty()),
        "a failed delivery's envelope names what stopped it: {delivery:#}"
    );
    let entries = delivery["entries"].as_array().expect("entries");
    let nodes: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(
        nodes,
        ["judge", "file"],
        "the review that ran, then the filing it failed at: {delivery:#}"
    );
    assert_eq!(entries[0]["outcome"], "completed", "{delivery:#}");
    assert!(
        entries[0]["models"]
            .as_array()
            .is_some_and(|models| models.len() == 1),
        "the model call the delivery made before it failed is on its envelope: {delivery:#}"
    );
    assert_eq!(entries[1]["outcome"], "failed", "{delivery:#}");
}

/// **Under `format: otlp` the head rides the delivery's root span, in its
/// parent's trace** (PRD resolved q64, `docs/trace.md` §12.2, §12.5).
///
/// The same two POSTs mapped to spans. The delivery's export carries
/// `agentcompose.detached`, `agentcompose.parent_execution` and
/// `agentcompose.idempotency_key` on its root, the key equal to the instance
/// path the parent's dispatch span reports — the OTLP spelling of the same
/// string-equality join — and its root is hung off the parent execution's root
/// span in the parent's trace, so a collector files the delivery under the run it
/// was dispatched from rather than beside it.
#[test]
fn an_otlp_sink_receives_the_detached_head_on_the_deliverys_root_span() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, _project, served)) =
        served_detached("sink-detached-otlp", &provider, &collector, Some("otlp"))
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/reviews");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (deliveries, parents): (Vec<&harness::Delivered>, Vec<&harness::Delivered>) =
        exported.iter().partition(|delivered| {
            attribute(root_span(&delivered.body), "agentcompose.detached").is_some()
        });
    assert_eq!((parents.len(), deliveries.len()), (1, 1), "{exported:#?}");
    let parent_root = root_span(&parents[0].body);
    let delivery_root = root_span(&deliveries[0].body);

    assert_eq!(
        attribute(parent_root, "agentcompose.execution.id"),
        Some(json!({ "stringValue": execution })),
        "{parent_root:#}"
    );
    for head in [
        "agentcompose.detached",
        "agentcompose.parent_execution",
        "agentcompose.idempotency_key",
    ] {
        assert!(
            attribute(parent_root, head).is_none(),
            "`{head}` is on a detached delivery's root and no other: {parent_root:#}"
        );
    }

    // The parent's dispatch span, whose instance path is the stub's key.
    let dispatched = parents[0].body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .expect("spans")
        .iter()
        .find(|span| attribute(span, "agentcompose.dispatch.outcome").is_some())
        .unwrap_or_else(|| {
            panic!(
                "the parent's export has a dispatch span: {:#}",
                parents[0].body
            )
        });
    let key = attribute(dispatched, "agentcompose.instance_path")
        .and_then(|held| held["stringValue"].as_str().map(str::to_string))
        .expect("a dispatch span reports its instance path");

    assert_eq!(
        attribute(delivery_root, "agentcompose.detached"),
        Some(json!({ "boolValue": true })),
        "{delivery_root:#}"
    );
    assert_eq!(
        attribute(delivery_root, "agentcompose.parent_execution"),
        Some(json!({ "stringValue": execution })),
        "{delivery_root:#}"
    );
    assert_eq!(
        attribute(delivery_root, "agentcompose.idempotency_key"),
        Some(json!({ "stringValue": key })),
        "the delivery's key is the parent's dispatch span's instance path: {delivery_root:#}"
    );
    assert_eq!(delivery_root["name"], "flow.review", "{delivery_root:#}");
    assert_eq!(
        delivery_root["traceId"], parent_root["traceId"],
        "the delivery is exported into its parent's trace"
    );
    assert_eq!(
        delivery_root["parentSpanId"], parent_root["spanId"],
        "…hung off the parent execution's root span"
    );
    assert_ne!(
        delivery_root["spanId"], parent_root["spanId"],
        "two roots of one id would be one span to a collector"
    );
    let spans = deliveries[0].body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .expect("spans");
    assert!(
        spans.iter().any(|span| span["name"] == "model.local"),
        "the delivery's model call is a span of its own export: {:#}",
        deliveries[0].body
    );
}

/// **A recovered execution re-ships its detached delivery on that delivery's own
/// settlement, and still ships its own export when it settles** (PRD resolved
/// q64, `docs/trace.md` §1.4, §8, `docs/durability.md` §3.2).
///
/// The at-least-once half. The parent detaches a review and parks at a `human`
/// node, so its lifecycle row stays open with the delivery already settled and
/// its envelope already shipped. The app is killed and started again over the
/// same journal; recovery re-runs the execution, the delivery with it — its model
/// call replayed from the journal rather than asked again — and on that
/// settlement the envelope ships a **second** time, under a second delivery id
/// and the same `(parent_execution, idempotency_key)`, which is the pair a
/// receiver dedupes on.
///
/// Then the pause is answered and the parent settles, and its own export arrives
/// — with two `trace_sink` rows of the delivery's already on its ledger. A guard
/// that read any such row as "this execution is exported" would have silenced it.
#[test]
fn a_recovered_execution_reships_its_detached_delivery_and_still_exports_itself() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, entrypoint, project, mut first)) =
        served_detached("sink-detached-recovered", &provider, &collector, None)
    else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let execution = started(&app, "/awaited-reviews");

    let shipped = collector.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(
        shipped[0].body["detached"], true,
        "a parked parent has not settled, so the first POST is the delivery's: {:#}",
        shipped[0].body
    );
    let parked = harness::settled(&app, &execution);
    assert_eq!(parked["status"], "interrupted", "{parked}");
    let asked = provider.requests().len();

    first.stop();
    let Some(second) =
        harness::serve_target_into(&project, &entrypoint, LOCAL, &environment(&provider))
    else {
        return;
    };

    let reshipped = collector.wait_for_event("settled", 2, PATIENCE);
    let (original, again) = (&reshipped[0], &reshipped[1]);
    assert_eq!(again.body["detached"], true, "{:#}", again.body);
    assert_eq!(
        (
            &again.body["parent_execution"],
            &again.body["idempotency_key"]
        ),
        (
            &original.body["parent_execution"],
            &original.body["idempotency_key"]
        ),
        "a re-shipped envelope carries the pair a receiver dedupes on, unchanged"
    );
    assert_eq!(
        again.body["entries"], original.body["entries"],
        "the recovered delivery replayed what the first one did, so it reports the same trace"
    );
    assert_ne!(
        again.header("x-agentcompose-delivery"),
        original.header("x-agentcompose-delivery"),
        "the re-ship is a second delivery, which is why receivers dedupe on the pair rather \
         than on the delivery id"
    );
    assert_eq!(
        provider.requests().len(),
        asked,
        "the recovered delivery's model call was replayed from the journal, not asked again"
    );

    // The parent settles, and its own export ships beside the delivery's two.
    let app = Client::new(&second.base_url).expect("a client for the restarted app");
    harness::until(PATIENCE, || {
        let answered = app
            .post_json(
                &format!("/executions/{execution}/resume"),
                &json!({ "decision": "approve" }),
            )
            .expect("the resume route answers");
        (answered.status == 202).then_some(())
    });
    let all = collector.wait_for_event("settled", 3, PATIENCE);
    let (parents, deliveries) = by_class(&all);
    assert_eq!(
        (parents.len(), deliveries.len()),
        (1, 2),
        "the parent's export is not mistaken for — or silenced by — its delivery's: {all:#?}"
    );
    let parent = &parents[0].body;
    assert_eq!(parent["execution_id"], execution, "{parent:#}");
    assert_eq!(parent["status"], "completed", "{parent:#}");
    assert_eq!(
        stub_record(parent, "review")["idempotencyKey"],
        deliveries[0].body["idempotency_key"],
        "{parent:#}"
    );
}
