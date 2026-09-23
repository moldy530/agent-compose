//! Child executions, end to end: a detached `flow.*` dispatch starts an execution
//! of its own — its own id, its own journal, its own recovery and its own export
//! — and its parent's account of it does not move (PRD resolved q65,
//! `docs/durability.md` §3.2, §3.5, §6.1, §11.2, `docs/trace.md` §1.4, §2, §12.2).
//!
//! Every test here serves or runs the `child-executions` fixture under a
//! `deploy/local.yml` the test writes beside a copy of it, naming a trace sink
//! this suite owns: `detach: true` is legal under `--target local` and under no
//! other target (grammar §8.6 rule 7, Decision D59), so that is the one place a
//! child execution can exist. Nothing is stubbed on either side — the provider is
//! the mock, the collector is a real socket, and the journal is read back through
//! the very driver the emitted project writes it with (`harness::journal_rows`).
//!
//! # What the ruling ratified, and where each clause is
//!
//! * **the child's execution row, with its lineage** —
//!   [`a_detached_dispatch_starts_a_child_execution_with_a_record_of_its_own`];
//! * **its effect rows, carrying the actual payloads** — the same test reads the
//!   verdict the child's model call answered back out of the child's journal;
//! * **its harness record** — a `coder:` node needs a harness SDK no served app
//!   in this suite can reach, so that clause is `compose-core`'s
//!   `a_fanned_out_coder_works_per_item_and_a_fresh_one_starts_empty`, which runs
//!   a detached coder review through the scripted driver `registerHarnessDriver`
//!   hosts and reads its `harness` record off the child's journal;
//! * **its envelope under its own id with the lineage head, and the parent's
//!   byte-unchanged with its stub** — the first test, over `format: envelope`,
//!   and [`an_otlp_sink_roots_a_child_executions_export_under_its_parent`] —
//!   and, down a chain of children, one trace:
//!   [`a_grandchild_is_exported_into_the_trace_its_parent_landed_in`];
//! * **the child runs on what its dispatch bound, as its joined twin does** —
//!   [`a_detached_dispatch_runs_on_the_input_its_joined_twin_runs_on`] — **and
//!   has an execution's own lifetimes**, its `scope: execution` store among them:
//!   [`a_child_executions_execution_scoped_store_is_its_own_partition`];
//! * **a failed child ships its failed envelope** —
//!   [`a_child_execution_that_failed_ships_its_failed_envelope`];
//! * **recovery with the parent settled** —
//!   [`a_child_caught_mid_run_after_its_parent_settled_is_recovered_without_asking_again`]
//!   by a restarted `serve`, and by hand, through `agent-compose resume` given
//!   the child's own id, on the input its dispatch bound and with its trace file
//!   headed by its lineage —
//!   [`a_child_resumed_by_its_own_id_runs_on_the_input_its_dispatch_bound`];
//! * **re-dispatch idempotence** —
//!   [`a_recovered_parent_that_re_issues_its_dispatch_joins_the_child_it_started`]
//!   for a child caught mid-run, and
//!   [`a_recovered_parent_finds_its_settled_child_and_starts_no_second`] for one
//!   that had finished;
//! * **a `run` waits for the children it started** —
//!   [`a_run_waits_for_its_child_executions_and_ships_their_envelopes`],
//!   [`a_run_waits_for_a_child_still_running_after_its_own_export_went`] and,
//!   for one still queued behind its node's `max_concurrency:`,
//!   [`a_run_waits_for_a_child_still_queued_behind_its_nodes_bound`];
//! * **a queued child is an execution from the moment it is issued** — its row
//!   is down before it has a permit, so a crash leaves it to recover:
//!   [`a_child_still_queued_when_its_process_stopped_is_recovered_and_run`];
//! * **a journal an earlier build wrote** —
//!   [`a_delivery_an_earlier_build_journaled_under_its_parent_is_replayed_as_a_child`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::collections::HashSet;
use std::time::Duration;

use mock_provider::{Client, MockProvider, Outcome, Request, Script};
use serde_json::{Value, json};

/// The fixture every test here drives.
const FIXTURE: &str = "child-executions";

/// The one target `detach: true` is legal under (grammar §8.6 rule 7, Decision
/// D59), and so the one target a child execution exists under.
const LOCAL: &str = "local";

/// The provider-native id `model.local` resolves to in that fixture.
const REVIEWER_MODEL: &str = "qwen3-coder-30b";

/// How long a test waits for a delivery two processes make between them.
const PATIENCE: Duration = Duration::from_secs(30);

/// Three attempts, all due at once (`docs/durability.md` §3.7's diagnostic
/// override), so a delivery a test is waiting for is not fifteen minutes out.
const FAST_RETRY: &str = "0s,0s,0s";

/// The environment every process in this suite is given.
fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    let mut held = harness::environment(provider);
    held.push((harness::CALLBACK_RETRY.to_string(), FAST_RETRY.to_string()));
    held
}

/// The same, with `${TALLY_BIN}/stall` pointing at a shim that stalls on its
/// **first** run and returns at once on every later one, logging each run.
///
/// The one node that reaches it is `flow.review_and_stall`'s `stall`, which is
/// where a test kills the process with the child mid-run: after its model call is
/// on its record, and before it has settled. The first run appends `first` to
/// `$TALLY_LOG` and sleeps far longer than any test waits; every later run
/// appends `again` and exits — so a recovered child finishes promptly, and "the
/// stalled step ran once per generation" is a line count.
fn stalling(provider: &MockProvider, shims: &harness::Scratch) -> Vec<(String, String)> {
    let log = shims.path().join("stall.log");
    harness::shim(
        shims.path(),
        "stall",
        "if [ -s \"$TALLY_LOG\" ]; then printf 'again\\n' >> \"$TALLY_LOG\"; exit 0; fi\n\
         printf 'first\\n' >> \"$TALLY_LOG\"\nsleep 600\n",
    );
    let mut held = environment(provider);
    held.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    held.push((harness::TALLY_LOG.to_string(), log.display().to_string()));
    held
}

/// A deploy file naming this sink, in the shape an operator writes one.
fn sink_target(url: &str, format: Option<&str>) -> String {
    let mut written = String::from("version: \"0.1\"\n\ntrace_sink:\n");
    written.push_str(&format!("  url: \"{url}\"\n"));
    if let Some(format) = format {
        written.push_str(&format!("  format: {format}\n"));
    }
    written
}

/// What [`served`] hands back: the staged composition (whose entrypoint is only
/// valid while it lives), that entrypoint, the project directory a restart serves
/// again, and the running app.
type Staged = (
    harness::Scratch,
    std::path::PathBuf,
    std::path::PathBuf,
    harness::Served,
);

/// Stage and serve the fixture under `deploy/local.yml`, naming a sink.
fn served(
    purpose: &str,
    collector: &harness::Receiver,
    format: Option<&str>,
    environment: &[(String, String)],
) -> Option<Staged> {
    let (composition, entrypoint) = harness::staged_with_deploy(
        purpose,
        FIXTURE,
        LOCAL,
        &sink_target(&format!("{}/v1/traces", collector.base_url), format),
    );
    let project = harness::scratch_project(purpose)?;
    let served = harness::serve_target_into(&project, &entrypoint, LOCAL, environment)?;
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

/// Split envelope-format exports into those of executions a trigger started and
/// those of child executions, by the head that tells them apart
/// (`docs/trace.md` §2).
fn by_lineage(
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

/// The span of an export whose parent is not in that export — its root.
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

/// Whether a recorded payload says the verdict was `verdict`, wherever in the
/// answer the provider's wire put it.
///
/// A model record holds the whole answer the loop accepted
/// (`docs/durability.md` §3.1), and a structured answer arrives as a parsed
/// object on one wire and as the assistant's JSON text on another — so the
/// verdict is looked for as a field, and inside any string that is itself a JSON
/// document, rather than at one path a wire change would move.
fn records_verdict(payload: &Value, verdict: &str) -> bool {
    match payload {
        Value::Object(fields) => fields.iter().any(|(key, value)| {
            (key == "verdict" && value == verdict) || records_verdict(value, verdict)
        }),
        Value::Array(items) => items.iter().any(|item| records_verdict(item, verdict)),
        Value::String(text) => serde_json::from_str::<Value>(text)
            .is_ok_and(|parsed| !parsed.is_string() && records_verdict(&parsed, verdict)),
        _ => false,
    }
}

/// One execution's effect rows, as `(key, kind)` pairs in key order.
fn effect_keys(project: &std::path::Path, execution: &str) -> Vec<(String, String)> {
    let rows = harness::journal_rows(
        project,
        &format!(
            "SELECT \"key\" AS k, kind FROM effects WHERE execution = '{execution}' ORDER BY \"key\""
        ),
    );
    rows.as_array()
        .expect("rows")
        .iter()
        .map(|row| {
            (
                row["k"].as_str().expect("a key").to_string(),
                row["kind"].as_str().expect("a kind").to_string(),
            )
        })
        .collect()
}

/// How many lines the stall shim has logged: one per run of the stalled step.
fn stalls(environment: &[(String, String)]) -> usize {
    let log = environment
        .iter()
        .find(|(name, _)| name == harness::TALLY_LOG)
        .map(|(_, value)| std::path::PathBuf::from(value))
        .expect("the stalling environment names its log");
    harness::lines_in(&log)
}

/// **A detached dispatch starts a child execution with a record of its own, and
/// its parent's account of it does not move** (PRD resolved q65).
///
/// The ruling's first verification, every clause in one run. The parent detaches
/// one review and settles; the child runs it.
///
/// * **The child is an execution**: its lifecycle row is in the journal under an
///   id derived — independently, here, from the ruling's own sentence — from the
///   parent's id and the dispatch's grammar §9.4 key, and its lineage row names
///   that parent, that key and the item's index.
/// * **Its effects are its own, with their payloads**: the child's model call is
///   on the child's record at the child's own site, `judge/0`, and the verdict
///   the model answered is read back out of it — while the parent's record holds
///   nothing at or under the dispatch's site.
/// * **It exports under its own id**: its envelope arrives on its own ledger,
///   headed `detached: true`, `parent_execution` and an `idempotency_key` equal
///   to the parent's stub record's, with the model call on its own entry.
/// * **The parent is unchanged**: its export carries the stub `"detached"`
///   record — `attempts: 0`, no `inner` — and no call of the child's, and its
///   report on the status route lists no delivery of the child's.
/// * **The child answers on the status route by its own id.**
#[test]
fn a_detached_dispatch_starts_a_child_execution_with_a_record_of_its_own() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, project, served)) =
        served("child-record", &collector, None, &environment(&provider))
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/reviews");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, children) = by_lineage(&exported);
    assert_eq!(
        (parents.len(), children.len()),
        (1, 1),
        "the sink received the parent's export and the child's, one each: {exported:#?}"
    );
    let parent = &parents[0].body;
    let child = &children[0].body;

    // The parent's export, byte for byte the shape it always was.
    assert_eq!(parent["trace_version"], 5, "{parent:#}");
    assert_eq!(parent["execution_id"], parent_id, "{parent:#}");
    assert_eq!(parent["flow"], "flow.dispatch", "{parent:#}");
    assert_eq!(parent["status"], "completed", "{parent:#}");
    for head in ["detached", "parent_execution", "idempotency_key"] {
        assert!(
            parent.get(head).is_none(),
            "`{head}` heads a child execution's envelope and no other: {parent:#}"
        );
    }
    let stub = stub_record(parent, "review");
    assert_eq!(stub["outcome"], "detached", "{stub:#}");
    assert_eq!(stub["attempts"], 0, "{stub:#}");
    assert_eq!(stub["target"], "flow.review", "{stub:#}");
    assert!(
        stub.get("inner").is_none(),
        "the stub record is the parent's whole account of the dispatch: {stub:#}"
    );
    for entry in parent["entries"].as_array().expect("entries") {
        for collected in ["models", "stores", "toolDispatches", "harness", "inner"] {
            assert!(
                entry.get(collected).is_none(),
                "the parent made no `{collected}` of its own, so one on its `{}` entry is the \
                 child's leaking into its parent's collector: {parent:#}",
                entry["node"]
            );
        }
    }
    let key = stub["idempotencyKey"]
        .as_str()
        .expect("a dispatch record carries its key")
        .to_string();
    assert_eq!(
        key,
        format!("{parent_id}/review/0/0"),
        "grammar §9.4's key for item 0 of map node `review` on its first traversal"
    );

    // The child's identity: derived, not minted.
    let child_id = harness::child_execution_id(&parent_id, &key);
    assert!(child_id.starts_with("exec_"), "{child_id}");
    assert_ne!(child_id, parent_id);

    // The child's envelope: an ordinary settled-execution export, under its own
    // id, headed by its lineage.
    assert_eq!(child["trace_version"], 5, "{child:#}");
    assert_eq!(child["detached"], true, "{child:#}");
    assert_eq!(child["parent_execution"], parent_id, "{child:#}");
    assert_eq!(
        child["idempotency_key"], key,
        "the join between the parent's stub record and this envelope is string equality on \
         the key: {child:#}"
    );
    assert_eq!(
        child["execution_id"], child_id,
        "a child execution's envelope carries its **own** id, derived from its parent's and the \
         dispatch's key: {child:#}"
    );
    assert_eq!(child["flow"], "flow.review", "{child:#}");
    assert_eq!(child["status"], "completed", "{child:#}");
    let head = String::from_utf8_lossy(&children[0].bytes);
    assert!(
        head.starts_with(&format!(
            "{{\"trace_version\":5,\"detached\":true,\"parent_execution\":\"{parent_id}\",\
             \"idempotency_key\":\"{key}\",\"flow\":\"flow.review\",\"execution_id\":\"{child_id}\","
        )),
        "the envelope is **headed** by the version and the lineage, in that order: {head}"
    );
    let entries = child["entries"].as_array().expect("entries");
    assert_eq!(entries.len(), 1, "{child:#}");
    assert_eq!(entries[0]["node"], "judge", "{child:#}");
    assert_eq!(entries[0]["step"], 1, "{child:#}");
    let models = entries[0]["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the child's model call is on its own entry: {child:#}"));
    assert_eq!(models.len(), 1, "{child:#}");
    assert_eq!(models[0]["servedBy"], "model.local", "{child:#}");

    // Two ledgers, one delivery each.
    let parent_delivery = parents[0]
        .header("x-agentcompose-delivery")
        .expect("a delivery names its id");
    let child_delivery = children[0]
        .header("x-agentcompose-delivery")
        .expect("a delivery names its id");
    assert_eq!(parent_delivery, format!("{parent_id}:0"));
    assert_eq!(
        child_delivery,
        format!("{child_id}:0"),
        "the child's export is the first event on **its own** ledger"
    );

    // The journal: two lifecycle rows, one lineage, and the child's effects on
    // the child's record.
    let executions = harness::journal_rows(
        &project,
        "SELECT id, flow, trigger_kind, status FROM executions ORDER BY started_at, id",
    );
    let rows = executions.as_array().expect("rows");
    assert_eq!(rows.len(), 2, "{executions:#}");
    let child_row = rows
        .iter()
        .find(|row| row["id"] == child_id)
        .unwrap_or_else(|| panic!("the child has a lifecycle row: {executions:#}"));
    assert_eq!(child_row["flow"], "flow.review", "{executions:#}");
    assert_eq!(child_row["status"], "completed", "{executions:#}");
    assert_eq!(
        child_row["trigger_kind"], "on_review_request",
        "a child's row names its parent's trigger, whose `auth:` guards it: {executions:#}"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT execution, parent, idempotency_key, item_index FROM lineage"
        ),
        json!([{
            "execution": child_id,
            "parent": parent_id,
            "idempotency_key": key,
            "item_index": 0,
        }]),
        "one lineage row, naming the dispatch that started the child"
    );
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())],
        "the child's model call is on the child's record, at a site rooted at the child's flow"
    );
    let parent_effects = effect_keys(&project, &parent_id);
    assert!(
        parent_effects
            .iter()
            .all(|(key, _)| !key.starts_with("review/")),
        "nothing the child did is on its parent's record: {parent_effects:?}"
    );
    let payload = harness::journal_rows(
        &project,
        &format!("SELECT payload FROM effects WHERE execution = '{child_id}'"),
    );
    let answered: Value = serde_json::from_str(
        payload[0]["payload"]
            .as_str()
            .expect("an effect row carries its payload"),
    )
    .expect("a payload is JSON");
    assert!(
        records_verdict(&answered, "approve"),
        "the verdict the child's model answered is readable back out of the child's journal: \
         {answered:#}"
    );

    // The status route answers the child by its own id, with its own report —
    // and the parent's report is the one it always was.
    let reported = harness::settled(&app, &child_id);
    assert_eq!(reported["execution_id"], child_id, "{reported:#}");
    assert_eq!(reported["flow"], "flow.review", "{reported:#}");
    assert_eq!(reported["status"], "completed", "{reported:#}");
    assert_eq!(reported["trace_version"], 5, "{reported:#}");
    assert_eq!(reported["trace"][0]["node"], "judge", "{reported:#}");
    let parent_report = harness::settled(&app, &parent_id);
    let listed: Vec<&str> = parent_report["deliveries"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|delivery| delivery["delivery_id"].as_str())
        .collect();
    assert!(
        listed
            .iter()
            .all(|id| id.starts_with(&format!("{parent_id}:"))),
        "the parent's report lists only its own ledger: {parent_report:#}"
    );
}

/// **A child execution that failed ships its failed envelope, and its failure is
/// not its parent's** (PRD resolved q64, q65).
///
/// The review ran and made its model call, the filing step failed, and none of it
/// touched the parent — which settled `completed`, because nothing a detached
/// dispatch does can fail the flow instance that issued it (grammar §8.6 rule 7).
/// The child's row closed `failed`, and its envelope says so under its own id,
/// carrying both of its entries, the aborting one last (`docs/trace.md` §9).
#[test]
fn a_child_execution_that_failed_ships_its_failed_envelope() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, project, served)) =
        served("child-failed", &collector, None, &environment(&provider))
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/filings");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, children) = by_lineage(&exported);
    assert_eq!((parents.len(), children.len()), (1, 1), "{exported:#?}");
    let parent = &parents[0].body;
    let child = &children[0].body;

    assert_eq!(
        parent["status"], "completed",
        "a child's failure is not its parent's: {parent:#}"
    );
    let stub = stub_record(parent, "review");
    assert_eq!(
        stub["outcome"], "detached",
        "the join never observes a detached dispatch's outcome (D94): {stub:#}"
    );
    assert!(stub.get("error").is_none(), "{stub:#}");
    let key = stub["idempotencyKey"].as_str().expect("a key");
    let child_id = harness::child_execution_id(&parent_id, key);

    assert_eq!(child["execution_id"], child_id, "{child:#}");
    assert_eq!(child["detached"], true, "{child:#}");
    assert_eq!(child["parent_execution"], parent_id, "{child:#}");
    assert_eq!(child["idempotency_key"], key, "{child:#}");
    assert_eq!(child["flow"], "flow.review_and_file", "{child:#}");
    assert_eq!(child["status"], "failed", "{child:#}");
    assert!(
        child["error"].as_str().is_some_and(|held| !held.is_empty()),
        "a failed child's envelope names what stopped it: {child:#}"
    );
    let entries = child["entries"].as_array().expect("entries");
    let nodes: Vec<&str> = entries
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(
        nodes,
        ["judge", "file"],
        "the review that ran, then the filing it failed at: {child:#}"
    );
    assert_eq!(entries[1]["outcome"], "failed", "{child:#}");

    let row = harness::journal_rows(
        &project,
        &format!("SELECT status FROM executions WHERE id = '{child_id}'"),
    );
    assert_eq!(
        row,
        json!([{ "status": "failed" }]),
        "the child's own row closed `failed`, so no restart resumes it"
    );
    let failed = harness::settled(&app, &child_id);
    assert_eq!(failed["status"], "failed", "{failed:#}");
}

/// **Under `format: otlp` a child's export lands in its parent's trace, rooted
/// under its parent's root span, with its lineage on its root** (PRD resolved
/// q64, q65, `docs/trace.md` §12.2, §12.5).
///
/// The child's spans are its own — its root carries its own execution id — and
/// the trace they land in is its parent's, hung off the parent execution's root:
/// a collector files a child under the run that dispatched it rather than beside
/// it. The root carries `agentcompose.detached`, `agentcompose.parent_execution`
/// and `agentcompose.idempotency_key`, the key equal to the instance path the
/// parent's dispatch span reports.
#[test]
fn an_otlp_sink_roots_a_child_executions_export_under_its_parent() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, _project, served)) = served(
        "child-otlp",
        &collector,
        Some("otlp"),
        &environment(&provider),
    ) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/reviews");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (children, parents): (Vec<&harness::Delivered>, Vec<&harness::Delivered>) =
        exported.iter().partition(|delivered| {
            attribute(root_span(&delivered.body), "agentcompose.detached").is_some()
        });
    assert_eq!((parents.len(), children.len()), (1, 1), "{exported:#?}");
    let parent_root = root_span(&parents[0].body);
    let child_root = root_span(&children[0].body);

    assert_eq!(
        attribute(parent_root, "agentcompose.execution.id"),
        Some(json!({ "stringValue": parent_id })),
        "{parent_root:#}"
    );
    for head in [
        "agentcompose.detached",
        "agentcompose.parent_execution",
        "agentcompose.idempotency_key",
    ] {
        assert!(
            attribute(parent_root, head).is_none(),
            "`{head}` is on a child execution's root and no other: {parent_root:#}"
        );
    }
    let dispatched = parents[0].body["resourceSpans"][0]["scopeSpans"][0]["spans"]
        .as_array()
        .expect("spans")
        .iter()
        .find(|span| attribute(span, "agentcompose.dispatch.outcome").is_some())
        .unwrap_or_else(|| panic!("the parent's export has a dispatch span"));
    let key = attribute(dispatched, "agentcompose.instance_path")
        .and_then(|held| held["stringValue"].as_str().map(str::to_string))
        .expect("a dispatch span reports its instance path");
    let child_id = harness::child_execution_id(&parent_id, &key);

    assert_eq!(
        attribute(child_root, "agentcompose.execution.id"),
        Some(json!({ "stringValue": child_id })),
        "the child's root carries its own execution id: {child_root:#}"
    );
    assert_eq!(
        attribute(child_root, "agentcompose.detached"),
        Some(json!({ "boolValue": true })),
        "{child_root:#}"
    );
    assert_eq!(
        attribute(child_root, "agentcompose.parent_execution"),
        Some(json!({ "stringValue": parent_id })),
        "{child_root:#}"
    );
    assert_eq!(
        attribute(child_root, "agentcompose.idempotency_key"),
        Some(json!({ "stringValue": key })),
        "{child_root:#}"
    );
    assert_eq!(
        attribute(child_root, "agentcompose.trace_version"),
        Some(json!({ "intValue": "5" })),
        "{child_root:#}"
    );
    assert_eq!(child_root["name"], "flow.review", "{child_root:#}");
    assert_eq!(
        child_root["traceId"], parent_root["traceId"],
        "a child is exported into its parent's trace"
    );
    assert_eq!(
        child_root["parentSpanId"], parent_root["spanId"],
        "…hung off the parent execution's root span"
    );
    assert_ne!(child_root["spanId"], parent_root["spanId"]);
}

/// **A grandchild is exported into the trace its parent's export landed in —
/// the head's — under its parent's root span** (PRD resolved q65,
/// `docs/trace.md` §12.2).
///
/// `flow.dispatch_nested` detaches a `flow.relay`, which detaches the review:
/// three executions, three exports. The envelope names only the immediate
/// parent, and deriving a grandchild's trace from that parent's id would file it
/// in a trace nothing else is exported into, under a root span that lives in
/// another one — two traces and a dangling parent on any collector. So all three
/// share the head's trace, and each root hangs off the root of the execution that
/// dispatched it.
#[test]
fn a_grandchild_is_exported_into_the_trace_its_parent_landed_in() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, project, served)) = served(
        "child-nested-otlp",
        &collector,
        Some("otlp"),
        &environment(&provider),
    ) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let head_id = started(&app, "/nested-reviews");

    let exported = collector.wait_for_event("settled", 3, PATIENCE);
    assert_eq!(
        exported.len(),
        3,
        "three executions, three exports: {exported:#?}"
    );
    let execution_of = |delivered: &harness::Delivered| -> String {
        attribute(root_span(&delivered.body), "agentcompose.execution.id")
            .and_then(|held| held["stringValue"].as_str().map(str::to_string))
            .expect("a root names its execution")
    };
    let parent_of = |delivered: &harness::Delivered| -> Option<String> {
        attribute(root_span(&delivered.body), "agentcompose.parent_execution")
            .and_then(|held| held["stringValue"].as_str().map(str::to_string))
    };
    let head = exported
        .iter()
        .find(|delivered| parent_of(delivered).is_none())
        .unwrap_or_else(|| panic!("the head's export arrived: {exported:#?}"));
    assert_eq!(execution_of(head), head_id);
    let child = exported
        .iter()
        .find(|delivered| parent_of(delivered).as_deref() == Some(head_id.as_str()))
        .unwrap_or_else(|| panic!("the relay's export arrived: {exported:#?}"));
    let child_id = execution_of(child);
    assert_eq!(
        child_id,
        harness::child_execution_id(&head_id, &format!("{head_id}/relay/0/0")),
        "the relay is the child the head's `relay` dispatch derives"
    );
    let grandchild = exported
        .iter()
        .find(|delivered| parent_of(delivered).as_deref() == Some(child_id.as_str()))
        .unwrap_or_else(|| panic!("the review's export arrived: {exported:#?}"));
    let grandchild_id = execution_of(grandchild);
    assert_eq!(
        grandchild_id,
        harness::child_execution_id(&child_id, &format!("{child_id}/hand_on/0/0")),
        "the review is the child the relay's `hand_on` dispatch derives"
    );
    assert_eq!(
        root_span(&grandchild.body)["name"],
        "flow.review",
        "{:#}",
        grandchild.body
    );

    let (head_root, child_root, grandchild_root) = (
        root_span(&head.body),
        root_span(&child.body),
        root_span(&grandchild.body),
    );
    assert_eq!(
        child_root["traceId"], head_root["traceId"],
        "a child is exported into its parent's trace"
    );
    assert_eq!(
        grandchild_root["traceId"], head_root["traceId"],
        "a grandchild is exported into the trace its parent's export landed in — the head's — \
         not into one its parent's own id derives"
    );
    for spans in [&child.body, &grandchild.body] {
        for span in spans["resourceSpans"][0]["scopeSpans"][0]["spans"]
            .as_array()
            .expect("spans")
        {
            assert_eq!(
                span["traceId"], head_root["traceId"],
                "every span of a descendant's export is in the head's trace: {span:#}"
            );
        }
    }
    assert_eq!(
        child_root["parentSpanId"], head_root["spanId"],
        "the relay hangs off the head's root"
    );
    assert_eq!(
        grandchild_root["parentSpanId"], child_root["spanId"],
        "the review hangs off the relay's root — the span of the execution that dispatched \
         it, which is in the same trace"
    );

    // Two lineages: each child names the execution that dispatched it.
    for (execution, parent) in [(&child_id, &head_id), (&grandchild_id, &child_id)] {
        assert_eq!(
            harness::journal_rows(
                &project,
                &format!("SELECT parent FROM lineage WHERE execution = '{execution}'")
            ),
            json!([{ "parent": parent }]),
            "`{execution}`'s lineage names the execution that dispatched it"
        );
    }
}

/// **A child execution's `scope: execution` store is its own partition** (PRD
/// resolved q65, `docs/grammar.md` §11.3, Decision D150).
///
/// `flow.dispatch_scratch` sets `handoff` in `store.scratch` and then detaches a
/// `flow.read_scratch` that gets the same key. The partition of an
/// execution-scoped store is keyed by `execution.id`, and inside a child that is
/// the child's own — so the key its parent set is in another partition, and the
/// child's read answers `found: false`. (Before the ruling the detached flow ran
/// under its parent's id and read its parent's partition, for as long as the
/// parent had not yet settled and released it — which a detached flow exists to
/// outlive.) The parent's own read-your-write is untouched: its `set` is on its
/// own entry, and a `scope: session` or `scope: global` store is what a parent
/// that hands data on to a child declares.
#[test]
fn a_child_executions_execution_scoped_store_is_its_own_partition() {
    let provider = MockProvider::start().expect("a loopback port");
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, project, served)) =
        served("child-scratch", &collector, None, &environment(&provider))
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/scratch-reviews");

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, children) = by_lineage(&exported);
    assert_eq!((parents.len(), children.len()), (1, 1), "{exported:#?}");
    let parent = &parents[0].body;
    assert_eq!(parent["status"], "completed", "{parent:#}");
    let left = parent["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["node"] == "leave")
        .unwrap_or_else(|| panic!("the parent's `leave` ran: {parent:#}"));
    assert_eq!(
        left["stores"][0]["op"], "set",
        "the parent set the key in its own partition: {left:#}"
    );
    assert_eq!(left["stores"][0]["scope"], "execution", "{left:#}");

    let child = &children[0].body;
    let key = stub_record(parent, "review")["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    assert_eq!(
        child["execution_id"],
        harness::child_execution_id(&parent_id, &key),
        "{child:#}"
    );
    assert_eq!(child["status"], "completed", "{child:#}");
    let looked = &child["entries"][0];
    assert_eq!(looked["node"], "look", "{child:#}");
    let read = &looked["stores"][0];
    assert_eq!(read["op"], "get", "{looked:#}");
    assert_eq!(read["scope"], "execution", "{looked:#}");
    assert_eq!(
        read["answer"]["found"], false,
        "a child execution read its parent's `scope: execution` partition; its own is the one \
         its id keys, and it begins empty (`docs/grammar.md` §11.3): {looked:#}"
    );
    assert_eq!(
        effect_keys(&project, &harness::child_execution_id(&parent_id, &key)),
        [("look/0#store/0".to_string(), "store".to_string())],
        "the child's read is on the child's record"
    );
}

/// **A detached dispatch runs on the input its joined twin runs on** (PRD
/// resolved q65, Decision D150).
///
/// `flow.dispatch_both` maps `flow.review` over the same subjects twice, joined
/// and then detached, and the subject is the empty string — which `flow.review`'s
/// own `inputs:` (`min_length: 1`) would refuse at an **invocation**. A dispatch
/// is not one: grammar §8.6 rule 12 checks what a `map` binds when the
/// composition is built, and the instance runs on what it was bound. So both
/// run. Before this was held, the joined instance completed while the detached
/// one was refused before its row opened — no row, no lineage, no journal, no
/// envelope, one line on stderr — so the same dispatch meant two things by
/// `detach:`. Now the child is an execution like any other: its row with its
/// lineage, its model call on its record, its envelope under its own id.
#[test]
fn a_detached_dispatch_runs_on_the_input_its_joined_twin_runs_on() {
    let provider = MockProvider::start().expect("a loopback port");
    for _ in 0..2 {
        provider.enqueue(Script::new(
            REVIEWER_MODEL,
            Outcome::structured(json!({ "verdict": "revise" })),
        ));
    }
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, _entrypoint, project, served)) =
        served("child-unparsed", &collector, None, &environment(&provider))
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let answered = app
        .send(Request::post("/both-reviews").json(&json!({ "subjects": [""] })))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    let parent_id = answered.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let exported = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, children) = by_lineage(&exported);
    assert_eq!(
        (parents.len(), children.len()),
        (1, 1),
        "the parent and its child each shipped: {exported:#?}"
    );
    let parent = &parents[0].body;
    assert_eq!(parent["status"], "completed", "{parent:#}");
    let joined = parent["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["node"] == "joined")
        .unwrap_or_else(|| panic!("the joined map ran: {parent:#}"));
    assert_eq!(
        joined["dispatches"][0]["outcome"], "completed",
        "the joined instance ran on the empty subject: {joined:#}"
    );

    let key = stub_record(parent, "review")["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    let child_id = harness::child_execution_id(&parent_id, &key);
    let child = &children[0].body;
    assert_eq!(child["execution_id"], child_id, "{child:#}");
    assert_eq!(
        child["status"], "completed",
        "the detached instance ran on the same subject its joined twin did: {child:#}"
    );
    assert_eq!(child["entries"][0]["node"], "judge", "{child:#}");

    assert_eq!(
        harness::journal_rows(
            &project,
            &format!("SELECT status, inputs FROM executions WHERE id = '{child_id}'")
        ),
        json!([{ "status": "completed", "inputs": "{\"subject\":\"\"}" }]),
        "the child's row holds the input its dispatch bound"
    );
    assert_eq!(
        harness::journal_rows(&project, "SELECT execution, parent FROM lineage"),
        json!([{ "execution": child_id, "parent": parent_id }])
    );
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())]
    );
    assert_eq!(
        provider.requests().len(),
        2,
        "one review joined, one detached"
    );
}

/// **A child caught mid-run after its parent had settled is recovered by the
/// next `serve`, replays to its frontier without asking its model again, and
/// ships its envelope** (PRD resolved q65, `docs/durability.md` §6.1).
///
/// The crash the ruling was ratified from. The parent detaches a review whose
/// second step stalls, and settles — its export arrives. The child's model call
/// is on its record and the stall is running when the process is killed. Before
/// the ruling nothing resumed a settled execution, and the delivery that outlived
/// it was lost with the process. Now the child is an open execution of its own:
/// the restarted app recovers it natively, its model call is replayed from the
/// journal — the provider is asked nothing more — the stalled step runs once
/// more, live, and the child settles and ships.
#[test]
fn a_child_caught_mid_run_after_its_parent_settled_is_recovered_without_asking_again() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let shims = harness::Scratch::new("child-stall-shims");
    let environment = stalling(&provider, &shims);
    let Some((_composition, entrypoint, project, mut first)) =
        served("child-recovered", &collector, None, &environment)
    else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/stalling-reviews");

    // The parent settles and exports while its child is still running.
    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    assert!(
        exported[0].body.get("detached").is_none(),
        "the first export is the parent's: {:#}",
        exported[0].body
    );
    let key = stub_record(&exported[0].body, "review")["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    let child_id = harness::child_execution_id(&parent_id, &key);
    // …and the child is mid-run: its model call recorded, its stall started.
    harness::until(PATIENCE, || (stalls(&environment) == 1).then_some(()));
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())],
        "the child's model call is on its record before the crash"
    );
    let asked = provider.requests().len();
    assert_eq!(asked, 1, "the child asked its model once");

    first.stop();
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT id, status FROM executions ORDER BY started_at, id"
        ),
        json!([
            { "id": parent_id, "status": "completed" },
            { "id": child_id, "status": "open" },
        ]),
        "the crash left the parent settled and its child open"
    );

    let Some(_second) = harness::serve_target_into(&project, &entrypoint, LOCAL, &environment)
    else {
        return;
    };
    let all = collector.wait_for_event("settled", 2, PATIENCE);
    let (_, children) = by_lineage(&all);
    assert_eq!(children.len(), 1, "{all:#?}");
    let child = &children[0].body;
    assert_eq!(child["execution_id"], child_id, "{child:#}");
    assert_eq!(child["parent_execution"], parent_id, "{child:#}");
    assert_eq!(child["idempotency_key"], key, "{child:#}");
    assert_eq!(child["status"], "completed", "{child:#}");
    let nodes: Vec<&str> = child["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(nodes, ["judge", "stall"], "{child:#}");
    assert_eq!(
        provider.requests().len(),
        asked,
        "the recovered child's model call was replayed from its journal, not asked again"
    );
    assert_eq!(
        stalls(&environment),
        2,
        "the stalled step ran once in the generation that died and once in the one that \
         recovered it — and in no third"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            &format!("SELECT status FROM executions WHERE id = '{child_id}'")
        ),
        json!([{ "status": "completed" }])
    );
    assert_eq!(
        collector.distinct("settled").len(),
        2,
        "one export per execution: {:?}",
        collector.distinct("settled")
    );
}

/// **A child resumed by its own id runs on the input its dispatch bound, and the
/// trace file it writes is headed by its lineage** (PRD resolved q65, Decision
/// D150, `docs/durability.md` §6.2, `docs/trace.md` §2).
///
/// The recovery a person makes by hand. The parent detaches a review of the
/// empty subject — which `flow.review_and_stall`'s own `inputs:`
/// (`min_length: 1`) would refuse at an **invocation**, and which the dispatch
/// ran on — and settles while the child stalls; the process is killed with the
/// child mid-run. The parent has completed, so `agent-compose resume` refuses it,
/// and the one recovery left on the command line is `resume` given the
/// **child's** id. That is a resume of a child, not an invocation: the child
/// replays under the input its row recorded, unparsed, exactly as its dispatch
/// ran it and as a `serve`'s recovery would — so whether a child can be resumed
/// does not depend on which surface resumes it. Its model is not asked again,
/// the stalled step runs once more, and it completes; the trace file the command
/// names carries the lineage head, and the export it ships is the child's.
#[test]
fn a_child_resumed_by_its_own_id_runs_on_the_input_its_dispatch_bound() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let shims = harness::Scratch::new("child-by-id-shims");
    let environment = stalling(&provider, &shims);
    let Some((_composition, entrypoint, project, mut first)) =
        served("child-by-id", &collector, None, &environment)
    else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let answered = app
        .send(Request::post("/stalling-reviews").json(&json!({ "subjects": [""] })))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    let parent_id = answered.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // The parent settles and exports while its child is still running…
    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    assert!(
        exported[0].body.get("detached").is_none(),
        "the first export is the parent's: {:#}",
        exported[0].body
    );
    let key = stub_record(&exported[0].body, "review")["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    let child_id = harness::child_execution_id(&parent_id, &key);
    // …and the child is mid-run on the empty subject: its model call recorded,
    // its stall started.
    harness::until(PATIENCE, || (stalls(&environment) == 1).then_some(()));
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())],
        "the child ran on the subject its dispatch bound, and its model call is on its record"
    );
    let asked = provider.requests().len();
    assert_eq!(asked, 1, "the child asked its model once");

    first.stop();
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT id, status, inputs FROM executions ORDER BY started_at, id"
        ),
        json!([
            { "id": parent_id, "status": "completed", "inputs": "{\"subjects\":[\"\"]}" },
            { "id": child_id, "status": "open", "inputs": "{\"subject\":\"\"}" },
        ]),
        "the crash left the parent settled and its child open, on the input its dispatch bound"
    );

    let mut resuming = environment.clone();
    resuming.push((harness::INTERACTIVE.to_string(), "0".to_string()));
    let refused = harness::resume_target(&project, &entrypoint, LOCAL, &parent_id, &resuming);
    let said = refused.failed();
    assert!(
        said.contains(&format!("`{parent_id}` has already completed")),
        "a settled parent is never resumed, so the child is the one thing left to resume: \
         {said}"
    );

    let resumed = harness::resume_target(&project, &entrypoint, LOCAL, &child_id, &resuming);
    resumed.succeeded();
    assert_eq!(
        provider.requests().len(),
        asked,
        "the resumed child's model call was replayed from its journal, not asked again"
    );
    assert_eq!(
        stalls(&environment),
        2,
        "the stalled step ran once in the generation that died and once in the resume"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            &format!("SELECT status, inputs FROM executions WHERE id = '{child_id}'")
        ),
        json!([{ "status": "completed", "inputs": "{\"subject\":\"\"}" }]),
        "the child completed on the input its row recorded"
    );

    let document = resumed.trace_document();
    assert_eq!(document["execution_id"], child_id, "{document:#}");
    assert_eq!(document["flow"], "flow.review_and_stall", "{document:#}");
    assert_eq!(document["status"], "completed", "{document:#}");
    assert_eq!(
        (
            &document["detached"],
            &document["parent_execution"],
            &document["idempotency_key"],
        ),
        (&json!(true), &json!(parent_id), &json!(key)),
        "the trace file of a child resumed by its own id is headed by its lineage: {document:#}"
    );
    let nodes: Vec<&str> = document["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(nodes, ["judge", "stall"], "{document:#}");

    let all = collector.wait_for_event("settled", 2, PATIENCE);
    let (_, children) = by_lineage(&all);
    assert_eq!(children.len(), 1, "{all:#?}");
    let child = &children[0].body;
    assert_eq!(child["execution_id"], child_id, "{child:#}");
    assert_eq!(child["parent_execution"], parent_id, "{child:#}");
    assert_eq!(child["idempotency_key"], key, "{child:#}");
    assert_eq!(child["status"], "completed", "{child:#}");
}

/// **A recovered parent that re-issues its detached dispatch resumes the child it
/// already started, rather than starting a second** (PRD resolved q65,
/// `docs/durability.md` §6.1).
///
/// The double-resume the ruling names. The parent detaches a review that stalls
/// and parks at a `human` node, and the process is killed with both open: the
/// child mid-run, its model call recorded. The restarted app finds both rows
/// open, and the child is resumed **by its parent**: recovery leaves it for the
/// parent's replay, which reaches the detached dispatch again, derives the same
/// id, finds the child's open row and resumes it behind that node's join — the
/// one direction that keeps the node's permit queue what the first generation's
/// was. One child results: one lifecycle row, one lineage, one effect per step,
/// the model asked once, and the stalled step run once per generation and never
/// twice in one. Then the parent is answered and settles, its export beside the
/// child's.
#[test]
fn a_recovered_parent_that_re_issues_its_dispatch_joins_the_child_it_started() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let shims = harness::Scratch::new("child-rejoin-shims");
    let environment = stalling(&provider, &shims);
    let Some((_composition, entrypoint, project, mut first)) =
        served("child-rejoined", &collector, None, &environment)
    else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/awaited-stalling-reviews");
    let parked = harness::settled(&app, &parent_id);
    assert_eq!(parked["status"], "interrupted", "{parked:#}");
    let key = format!("{parent_id}/review/0/0");
    let child_id = harness::child_execution_id(&parent_id, &key);
    harness::until(PATIENCE, || (stalls(&environment) == 1).then_some(()));
    let asked = provider.requests().len();
    assert_eq!(asked, 1);

    first.stop();
    let open = harness::journal_rows(
        &project,
        "SELECT id FROM executions WHERE status = 'open' ORDER BY started_at, id",
    );
    assert_eq!(
        open,
        json!([{ "id": parent_id }, { "id": child_id }]),
        "the crash left both the parked parent and its child open"
    );

    let Some(mut second) = harness::serve_target_into(&project, &entrypoint, LOCAL, &environment)
    else {
        return;
    };
    let shipped = collector.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(
        shipped[0].body["execution_id"], child_id,
        "the child settles and ships while its parent is still parked: {:#}",
        shipped[0].body
    );
    assert_eq!(shipped[0].body["status"], "completed");

    // **One generation, asked about directly.** Everything below it — one row,
    // one lineage, one effect set — is also what a second generation that
    // *failed at once* would leave, and a second arrival that was let in rather
    // than joined is exactly that: it puts a fresh entry for the child on this
    // app's board and its run trips the one-generation guard
    // (`docs/durability.md` §2.1), replacing the child the status route answers
    // with one that failed. So the route is asked, and it has to answer the one
    // generation that ran: the child `completed`, with both of its steps.
    let restarted = Client::new(&second.base_url).expect("a client for the restarted app");
    let reported = harness::settled(&restarted, &child_id);
    assert_eq!(
        reported["status"], "completed",
        "the status route answers the child with a generation that did not run it — a second \
         arrival started a second generation rather than joining the first: {reported:#}"
    );
    let steps: Vec<&str> = reported["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert_eq!(
        steps,
        ["judge", "stall"],
        "the child's report is its run's: {reported:#}"
    );

    // One child, whichever direction reached it first.
    assert_eq!(
        harness::journal_rows(&project, "SELECT execution, parent FROM lineage"),
        json!([{ "execution": child_id, "parent": parent_id }]),
        "one child execution for one dispatch"
    );
    assert_eq!(
        harness::journal_rows(&project, "SELECT COUNT(*) AS n FROM executions"),
        json!([{ "n": 2 }]),
        "no second child was begun"
    );
    assert_eq!(
        effect_keys(&project, &child_id),
        [
            ("judge/0#model/0".to_string(), "model".to_string()),
            ("stall/0#tool/0".to_string(), "tool".to_string()),
        ],
        "one effect set: the recorded call replayed, the stalled step recorded once"
    );
    assert_eq!(
        provider.requests().len(),
        asked,
        "the model was asked once across both generations"
    );
    assert_eq!(
        stalls(&environment),
        2,
        "the stalled step ran once per generation — a second generation of the child beside \
         the first would have run it a third time"
    );

    // The parent, answered, settles and exports beside its child.
    let app = Client::new(&second.base_url).expect("a client for the restarted app");
    harness::until(PATIENCE, || {
        let answered = app
            .post_json(
                &format!("/executions/{parent_id}/resume"),
                &json!({ "decision": "approve" }),
            )
            .expect("the resume route answers");
        (answered.status == 202).then_some(())
    });
    let all = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, children) = by_lineage(&all);
    assert_eq!((parents.len(), children.len()), (1, 1), "{all:#?}");
    assert_eq!(parents[0].body["execution_id"], parent_id);
    assert_eq!(parents[0].body["status"], "completed");
    assert_eq!(
        stub_record(&parents[0].body, "review")["idempotencyKey"],
        key.as_str()
    );

    // …and what the restarted app said about it, which is where a second
    // generation that failed at once is the only place it is said: the line the
    // child machinery writes for a child that did not complete. The recovery
    // line is the control — it is what shows this is the app's stderr at all —
    // and it says the child was left for its open parent to re-issue.
    let said = second.stop_and_read_stderr();
    assert!(
        said.contains(&format!(
            "recovered {child_id} (flow.review_and_stall), for its open parent {parent_id} to \
             re-issue"
        )),
        "the restarted app's stderr is not the one that recovered the child, or it resumed the \
         child itself rather than through its open parent: {said}"
    );
    assert!(
        !said.contains(&format!("the child execution `{child_id}`")),
        "a second generation of the child ran beside the first and did not complete: {said}"
    );
    assert!(
        !said.contains("is already running in this process"),
        "a second arrival reached the one-generation guard instead of joining: {said}"
    );
}

/// **A recovered parent that re-issues its dispatch to a child that already
/// finished starts nothing** (PRD resolved q65: a settled execution is never
/// resumed).
///
/// The parent detaches a review and parks; the review completes and ships. The
/// app is killed and started again over the same journal, and the recovered
/// parent replays to its dispatch and re-issues it — deriving the child's id and
/// finding its row closed. Nothing runs, nothing ships a second time, and the
/// model is not asked again. Answered, the parent settles and ships its own
/// export — beside one child envelope, not two.
#[test]
fn a_recovered_parent_finds_its_settled_child_and_starts_no_second() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let Some((_composition, entrypoint, project, mut first)) = served(
        "child-settled-once",
        &collector,
        None,
        &environment(&provider),
    ) else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let parent_id = started(&app, "/awaited-reviews");
    let shipped = collector.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(shipped[0].body["detached"], true, "{:#}", shipped[0].body);
    let child_id = shipped[0].body["execution_id"]
        .as_str()
        .expect("an id")
        .to_string();
    let parked = harness::settled(&app, &parent_id);
    assert_eq!(parked["status"], "interrupted", "{parked:#}");
    let asked = provider.requests().len();

    first.stop();
    let Some(second) =
        harness::serve_target_into(&project, &entrypoint, LOCAL, &environment(&provider))
    else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the restarted app");
    harness::until(PATIENCE, || {
        let answered = app
            .post_json(
                &format!("/executions/{parent_id}/resume"),
                &json!({ "decision": "approve" }),
            )
            .expect("the resume route answers");
        (answered.status == 202).then_some(())
    });
    let all = collector.wait_for_event("settled", 2, PATIENCE);
    let (parents, _) = by_lineage(&all);
    assert_eq!(parents.len(), 1, "{all:#?}");
    assert_eq!(parents[0].body["execution_id"], parent_id);
    assert_eq!(
        collector.distinct("settled"),
        [format!("{child_id}:0"), format!("{parent_id}:0")],
        "the child shipped once, before the crash, and the parent once, when it settled"
    );
    assert_eq!(
        provider.requests().len(),
        asked,
        "re-issuing the dispatch of a child that already ran asked its model nothing"
    );
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())]
    );
    assert_eq!(
        harness::journal_rows(&project, "SELECT COUNT(*) AS n FROM lineage"),
        json!([{ "n": 1 }])
    );
}

/// **A `run` waits for the child executions it started, and ships their
/// envelopes before it exits** (PRD resolved q50's "`run` included", q65).
///
/// `flow.dispatch_and_wait` detaches a review and parks, so the command reports
/// `3`. The command does not end under its child: it waits for it to settle, then
/// sends what it exported — the only POST, because the parked parent never
/// settles and exports nothing. The envelope is under the child's own id, on the
/// child's own ledger, and the command finished it.
#[test]
fn a_run_waits_for_its_child_executions_and_ships_their_envelopes() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "child-run",
        FIXTURE,
        LOCAL,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None),
    );
    let Some(project) = harness::scratch_project("child-run") else {
        return;
    };
    let run = harness::run_target(
        &project,
        &entrypoint,
        LOCAL,
        "flow.dispatch_and_wait",
        &[("subjects", r#"["the collector"]"#)],
        &environment(&provider),
    );
    let said = run.failed();
    assert_eq!(
        run.output.status.code(),
        Some(3),
        "a run with nobody to answer its `human` node parks and reports `3`: {said}"
    );
    let parent_id = said
        .lines()
        .find_map(|line| line.strip_prefix("execution: "))
        .unwrap_or_else(|| panic!("the run names its execution\nstderr: {said}"))
        .trim()
        .to_string();
    let parent = run.trace_document();
    assert_eq!(parent["status"], "interrupted", "{parent:#}");
    let stub = stub_record(&parent, "review");
    assert_eq!(stub["outcome"], "detached", "{stub:#}");
    let key = stub["idempotencyKey"].as_str().expect("a key");
    let child_id = harness::child_execution_id(&parent_id, key);

    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(exported.len(), 1, "{exported:#?}");
    let child = &exported[0].body;
    assert_eq!(child["trace_version"], 5, "{child:#}");
    assert_eq!(child["detached"], true, "{child:#}");
    assert_eq!(child["execution_id"], child_id, "{child:#}");
    assert_eq!(child["parent_execution"], parent_id, "{child:#}");
    assert_eq!(child["idempotency_key"], key, "{child:#}");
    assert_eq!(child["status"], "completed", "{child:#}");
    assert_eq!(
        exported[0].header("x-agentcompose-delivery"),
        Some(format!("{child_id}:0").as_str()),
        "the child's export is on the child's own ledger"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT execution, kind, event, status FROM deliveries ORDER BY execution, ordinal"
        ),
        json!([{
            "execution": child_id,
            "kind": "trace_sink",
            "event": "settled",
            "status": "delivered",
        }]),
        "one row, on the child's ledger, and the command sent it"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            &format!("SELECT status FROM executions WHERE id = '{child_id}'")
        ),
        json!([{ "status": "completed" }]),
        "the command did not exit under its child"
    );
}

/// **A `run` waits for a child still running after its own export has gone**
/// (PRD resolved q65: the process does not end under a child it started).
///
/// `flow.dispatch` detaches two reviews and settles at once. The first review is
/// answered at once; the second is answered `LATE_REVIEW` after it asks, and the
/// collector sits on the parent's export for `EXPORT_HOLD` — so the second child
/// is still running when the command has reported and sent its own export. A
/// command that exited then would have killed that child mid-effect; this one
/// waits, and sends three envelopes, each on its own ledger.
#[test]
fn a_run_waits_for_a_child_still_running_after_its_own_export_went() {
    const EXPORT_HOLD: Duration = Duration::from_millis(1_500);
    const LATE_REVIEW: Duration = Duration::from_millis(4_000);

    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })).after(LATE_REVIEW),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    collector.hold_answers(&[EXPORT_HOLD]);
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "child-run-late",
        FIXTURE,
        LOCAL,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None),
    );
    let Some(project) = harness::scratch_project("child-run-late") else {
        return;
    };
    let run = harness::run_target(
        &project,
        &entrypoint,
        LOCAL,
        "flow.dispatch",
        &[("subjects", r#"["the first","the second"]"#)],
        &environment(&provider),
    );
    run.succeeded();
    let said = run.stderr();
    let parent_id = said
        .lines()
        .find_map(|line| line.strip_prefix("execution: "))
        .unwrap_or_else(|| panic!("the run names its execution\nstderr: {said}"))
        .trim()
        .to_string();
    assert_eq!(provider.requests().len(), 2, "both children asked");

    let exported = collector.wait_for_event("settled", 3, PATIENCE);
    let (parents, children) = by_lineage(&exported);
    assert_eq!((parents.len(), children.len()), (1, 2), "{exported:#?}");
    let mut ids: Vec<String> = children
        .iter()
        .map(|delivered| {
            assert_eq!(
                delivered.body["status"], "completed",
                "{:#}",
                delivered.body
            );
            delivered.body["execution_id"]
                .as_str()
                .expect("an id")
                .to_string()
        })
        .collect();
    ids.sort_unstable();
    let mut expected = vec![
        harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/0")),
        harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/1")),
    ];
    expected.sort_unstable();
    assert_eq!(
        ids, expected,
        "one child execution per detached dispatch, the late one included"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT kind, status FROM deliveries ORDER BY execution, ordinal"
        ),
        json!([
            { "kind": "trace_sink", "status": "delivered" },
            { "kind": "trace_sink", "status": "delivered" },
            { "kind": "trace_sink", "status": "delivered" },
        ]),
        "three exports on three ledgers, all sent by the command before it exited"
    );
}

/// **A `run` waits for a child still queued behind its node's bound** (PRD
/// resolved q65: the process does not end under a child it started; Decision
/// D28).
///
/// `flow.dispatch` detaches two reviews under `max_concurrency: 1` and settles at
/// once, so the second child waits for the first's permit. The first review is
/// answered `LATE_REVIEW` after it asks, and **no sink** is declared, so nothing
/// the command sends after its answer gives the second child time to start on its
/// own. The command must wait out the first child and then the second — the one
/// that was never running while the first was — before it exits: both children
/// completed, both asked their model, one lineage row each.
#[test]
fn a_run_waits_for_a_child_still_queued_behind_its_nodes_bound() {
    const LATE_REVIEW: Duration = Duration::from_millis(2_000);

    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })).after(LATE_REVIEW),
    ));
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));
    let (_composition, entrypoint) =
        harness::staged_with_deploy("child-run-queued", FIXTURE, LOCAL, "version: \"0.1\"\n");
    let Some(project) = harness::scratch_project("child-run-queued") else {
        return;
    };
    let run = harness::run_target(
        &project,
        &entrypoint,
        LOCAL,
        "flow.dispatch",
        &[("subjects", r#"["the first","the second"]"#)],
        &environment(&provider),
    );
    run.succeeded();
    let said = run.stderr();
    let parent_id = said
        .lines()
        .find_map(|line| line.strip_prefix("execution: "))
        .unwrap_or_else(|| panic!("the run names its execution\nstderr: {said}"))
        .trim()
        .to_string();
    let first = harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/0"));
    let second = harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/1"));

    assert_eq!(
        provider.requests().len(),
        2,
        "the command exited before the queued child had asked its model: {said}"
    );
    let mut expected = vec![
        json!({ "id": parent_id, "status": "completed" }),
        json!({ "id": first, "status": "completed" }),
        json!({ "id": second, "status": "completed" }),
    ];
    expected.sort_by_key(|row| row["id"].as_str().unwrap_or_default().to_string());
    assert_eq!(
        harness::journal_rows(&project, "SELECT id, status FROM executions ORDER BY id"),
        Value::Array(expected),
        "the command did not end under a child — the queued one included"
    );
    assert_eq!(
        harness::journal_rows(&project, "SELECT COUNT(*) AS n FROM lineage"),
        json!([{ "n": 2 }]),
        "one child execution per detached dispatch"
    );
}

/// **A child still queued behind its node's bound when `serve` stopped is an
/// open row, recovered and run by the next start** (PRD resolved q65,
/// `docs/durability.md` §3.2, §6.1).
///
/// `flow.dispatch_stalling` detaches two reviews under `max_concurrency: 1` and
/// settles: the first child records its model call and stalls, holding the
/// permit, and the second waits for it. The process is killed there. Before the
/// fix the second dispatch existed only in memory — no row, nothing to recover —
/// so the work its parent counted as `detached` never ran. Now it was begun the
/// moment it was issued: the crash leaves the parent settled and **both** children
/// open, each with its lineage, and the restart recovers both — the first replays
/// its recorded model call, the second asks its own — and both settle and ship.
#[test]
fn a_child_still_queued_when_its_process_stopped_is_recovered_and_run() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let shims = harness::Scratch::new("child-queued-shims");
    let environment = stalling(&provider, &shims);
    let Some((_composition, entrypoint, project, mut first)) =
        served("child-queued", &collector, None, &environment)
    else {
        return;
    };
    let app = Client::new(&first.base_url).expect("a client for the generated app");
    let answered = app
        .send(Request::post("/stalling-reviews").json(&json!({ "subjects": ["one", "two"] })))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    let parent_id = answered.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let running = harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/0"));
    let queued = harness::child_execution_id(&parent_id, &format!("{parent_id}/review/0/1"));

    // The parent settles and exports; the first child is mid-run, holding the
    // node's one permit, and the second is waiting for it.
    let exported = collector.wait_for_event("settled", 1, PATIENCE);
    assert!(
        exported[0].body.get("detached").is_none(),
        "the first export is the parent's: {:#}",
        exported[0].body
    );
    harness::until(PATIENCE, || (stalls(&environment) == 1).then_some(()));
    assert_eq!(
        provider.requests().len(),
        1,
        "the queued child started beside the running one"
    );

    first.stop();
    let mut expected = vec![
        json!({ "id": parent_id, "status": "completed" }),
        json!({ "id": running, "status": "open" }),
        json!({ "id": queued, "status": "open" }),
    ];
    expected.sort_by_key(|row| row["id"].as_str().unwrap_or_default().to_string());
    assert_eq!(
        harness::journal_rows(&project, "SELECT id, status FROM executions ORDER BY id"),
        Value::Array(expected),
        "the crash left the queued child with no row — nothing will ever run it"
    );
    let mut lineage = vec![
        json!({ "execution": running, "parent": parent_id }),
        json!({ "execution": queued, "parent": parent_id }),
    ];
    lineage.sort_by_key(|row| row["execution"].as_str().unwrap_or_default().to_string());
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT execution, parent FROM lineage ORDER BY execution"
        ),
        Value::Array(lineage),
        "both children's causes are on the journal"
    );
    assert!(
        effect_keys(&project, &queued).is_empty(),
        "the queued child had done nothing"
    );

    let Some(_second) = harness::serve_target_into(&project, &entrypoint, LOCAL, &environment)
    else {
        return;
    };
    // Three distinct exports — the parent's and one per child. Counted by
    // delivery id rather than by POST, because the kill can land between the
    // parent's export arriving and its row being marked delivered, and the
    // restart then sends it again under the same id: at-least-once, which a
    // receiver dedupes (`docs/durability.md` §3.7).
    harness::until(PATIENCE, || {
        (collector.distinct("settled").len() >= 3).then_some(())
    });
    let all = collector.wait_for_event("settled", 3, PATIENCE);
    let (_, children) = by_lineage(&all);
    let mut shipped: Vec<(String, String)> = children
        .iter()
        .map(|delivered| {
            (
                delivered.body["execution_id"]
                    .as_str()
                    .expect("an id")
                    .to_string(),
                delivered.body["status"]
                    .as_str()
                    .expect("a status")
                    .to_string(),
            )
        })
        .collect();
    shipped.sort_unstable();
    shipped.dedup();
    let mut wanted = vec![
        (running.clone(), "completed".to_string()),
        (queued.clone(), "completed".to_string()),
    ];
    wanted.sort_unstable();
    assert_eq!(shipped, wanted, "{all:#?}");
    assert_eq!(
        provider.requests().len(),
        2,
        "the recovered child replayed its recorded call and the queued one asked its own — \
         once each"
    );
    assert_eq!(
        effect_keys(&project, &queued),
        [
            ("judge/0#model/0".to_string(), "model".to_string()),
            ("stall/0#tool/0".to_string(), "tool".to_string()),
        ],
        "the queued child ran, on its own record"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            "SELECT COUNT(*) AS n FROM executions WHERE status = 'completed'"
        ),
        json!([{ "n": 3 }])
    );
}

/// **A detached delivery an earlier build journaled under its parent is replayed
/// as the child execution this build runs it as** (`docs/durability.md` §11.2,
/// PRD resolved q65).
///
/// The upgrade path. Before the ruling a detached `flow.*` delivery ran under its
/// parent's id, so a parent an older build left **open** — parked here, at its
/// `human` node — holds the delivery's effects on its own record at the
/// dispatch's site, and no child row at all. This build re-issues that dispatch
/// when the parent is resumed, and beginning the child empty would ask its model
/// again. So the journal an older build would have left is made out of a real one
/// — the child's rows moved onto its parent's record under the dispatch's path,
/// its lifecycle and ledger rows removed, and the `lineage` table an older build
/// never created dropped — and the parent is resumed:
/// the child's records are carried over to it, it replays to its frontier, the
/// provider is asked nothing, and it settles and ships.
#[test]
fn a_delivery_an_earlier_build_journaled_under_its_parent_is_replayed_as_a_child() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        REVIEWER_MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    let collector = harness::Receiver::start().expect("a loopback collector");
    let (_composition, entrypoint) = harness::staged_with_deploy(
        "child-legacy",
        FIXTURE,
        LOCAL,
        &sink_target(&format!("{}/v1/traces", collector.base_url), None),
    );
    let Some(project) = harness::scratch_project("child-legacy") else {
        return;
    };
    let mut environment = environment(&provider);
    environment.push((harness::INTERACTIVE.to_string(), "0".to_string()));
    let run = harness::run_target(
        &project,
        &entrypoint,
        LOCAL,
        "flow.dispatch_and_wait",
        &[("subjects", r#"["the collector"]"#)],
        &environment,
    );
    let said = run.failed();
    assert_eq!(run.output.status.code(), Some(3), "{said}");
    let parent_id = said
        .lines()
        .find_map(|line| line.strip_prefix("execution: "))
        .unwrap_or_else(|| panic!("the run names its execution\nstderr: {said}"))
        .trim()
        .to_string();
    let key = format!("{parent_id}/review/0/0");
    let child_id = harness::child_execution_id(&parent_id, &key);
    collector.wait_for_event("settled", 1, PATIENCE);
    let asked = provider.requests().len();
    assert_eq!(asked, 1);

    // The journal an earlier build would have left: the delivery's effects on
    // its parent's record at the dispatch's site, no child at all — and no
    // `lineage` table, which this build adds to a file an older one wrote with
    // `CREATE TABLE IF NOT EXISTS` on first open (`docs/durability.md` §11.2).
    let surgery = harness::journal_sql(
        &project,
        &format!(
            "UPDATE effects SET execution = '{parent_id}', site = 'review/0/0/' || site, \
             \"key\" = 'review/0/0/' || \"key\" WHERE execution = '{child_id}';\n\
             DROP TABLE lineage;\n\
             DELETE FROM executions WHERE id = '{child_id}';\n\
             DELETE FROM deliveries WHERE execution = '{child_id}';\n"
        ),
    );
    assert!(surgery.status.success(), "{surgery:?}");
    assert_eq!(
        effect_keys(&project, &parent_id),
        [(
            "review/0/0/judge/0#model/0".to_string(),
            "model".to_string()
        )],
        "the delivery's model call is where an earlier build journaled it"
    );

    let resumed = harness::resume_target(&project, &entrypoint, LOCAL, &parent_id, &environment);
    assert_eq!(
        resumed.output.status.code(),
        Some(3),
        "the resumed parent re-parks at its `human` node: {}",
        resumed.stderr()
    );
    assert_eq!(
        provider.requests().len(),
        asked,
        "the delivery's model call was replayed from the record an earlier build wrote, not \
         asked again"
    );
    assert_eq!(
        effect_keys(&project, &child_id),
        [("judge/0#model/0".to_string(), "model".to_string())],
        "the record was carried over to the child, re-keyed to the child's own sites"
    );
    assert_eq!(
        harness::journal_rows(
            &project,
            &format!("SELECT status FROM executions WHERE id = '{child_id}'")
        ),
        json!([{ "status": "completed" }]),
        "the child settled"
    );
    assert_eq!(
        harness::journal_rows(&project, "SELECT execution, parent FROM lineage"),
        json!([{ "execution": child_id, "parent": parent_id }])
    );
    let reshipped = collector.wait_for_event("settled", 2, PATIENCE);
    let last = &reshipped[1].body;
    assert_eq!(last["execution_id"], child_id, "{last:#}");
    assert_eq!(last["status"], "completed", "{last:#}");
    assert_eq!(last["entries"][0]["node"], "judge", "{last:#}");
}
