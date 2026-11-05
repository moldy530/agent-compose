//! A trigger's intake, end to end against a real `serve`: deduplicating,
//! bounding and debouncing the executions a trigger starts (PRD resolved q77,
//! Decision D163, grammar 13.1, 13.3, 13.5, `docs/durability.md` §3.12,
//! §3.15, §6.3, `docs/distributed.md` §8).
//!
//! Every execution here is `${TALLY_BIN}/work` — a shim that appends
//! `<tag> start <pushes>` to `$TALLY_LOG`, sleeps `$LINGER` seconds and appends
//! `<tag> end` — or `${TALLY_BIN}/hold`, which waits for the file `$HOLD` names.
//! What ran, when and how often is read off the journal's lifecycle and intake
//! rows, which is where the lease the ruling asks about lives.
//!
//! What the ruling's verification asks, and where each is:
//!
//! 1. three identical deliveries start one execution across a restart, and a
//!    fourth after the window starts a new one —
//!    [`identical_deliveries_inside_the_window_start_one_execution_across_a_restart`];
//! 2. twenty events for one key under `limit: 1` never run two at once and
//!    all run, in order —
//!    [`twenty_events_for_one_key_under_limit_one_run_one_at_a_time_in_order`];
//! 3. `supersede` gives the newest event a full run, its predecessor and that
//!    one's child cancelled —
//!    [`a_superseding_event_cancels_the_running_execution_and_its_child_and_runs_whole`];
//! 4. five deliveries under a debounce start one run with the fifth payload and
//!    the count —
//!    [`deliveries_under_a_debounce_start_one_run_with_the_newest_payload_and_the_count`],
//!    and the request a pending row holds keeps no credential —
//!    [`a_pending_row_holds_the_request_without_the_credential_its_auth_read`];
//! 5. a restart mid-queue and mid-debounce —
//!    [`a_restart_with_three_queued_arrivals_starts_each_once_in_order`] and
//!    [`a_restart_with_a_debounce_pending_fires_it_once`];
//! 6. a refused `run` — [`a_run_of_a_flow_whose_group_is_full_is_refused_and_journals_nothing`];
//! 7. `drop` — [`an_arrival_its_full_group_drops_is_answered_with_the_running_execution`]
//!    and the event half in (2);
//! 8. `GET /intakes/:id` — [`the_intake_route_reports_each_state_under_the_triggers_auth`];
//! 11. `max_running` across groups —
//!     [`the_triggers_max_running_bounds_its_groups_together`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider, Request, Response};
use serde_json::{Value, json};

/// The fixture the `http` tests serve.
const FIXTURE: &str = "intake";

/// The token the fixture's `guarded` trigger verifies.
const TOKEN: &str = "intake-token-of-the-test";

/// The shims the fixtures name, written into `directory`.
fn shims(directory: &Path) {
    harness::shim(
        directory,
        "work",
        "printf '%s start %s\\n' \"$TAG\" \"$PUSHES\" >> \"$TALLY_LOG\"\n\
         sleep \"$LINGER\"\n\
         printf '%s end\\n' \"$TAG\" >> \"$TALLY_LOG\"\n",
    );
    harness::shim(
        directory,
        "hold",
        "while [ ! -e \"$HOLD\" ]; do sleep 0.05; done\n",
    );
}

/// The environment a fixture is served with, its tally at the returned path.
fn environment(
    provider: &MockProvider,
    scratch: &harness::Scratch,
    knobs: &[(&str, &str)],
) -> (Vec<(String, String)>, PathBuf) {
    shims(scratch.path());
    let log = scratch.path().join("tally.log");
    let mut held = harness::environment(provider);
    held.push((
        harness::TALLY_BIN.to_string(),
        scratch.path().display().to_string(),
    ));
    held.push((harness::TALLY_LOG.to_string(), log.display().to_string()));
    held.push(("INTAKE_TOKEN".to_string(), TOKEN.to_string()));
    held.push((
        "FEED_PROGRAM".to_string(),
        harness::projects()
            .join("command-source")
            .join("feed.mjs")
            .display()
            .to_string(),
    ));
    held.push((
        "FEED_LOG".to_string(),
        scratch.path().join("emitted.log").display().to_string(),
    ));
    for (name, value) in knobs {
        held.push(((*name).to_string(), (*value).to_string()));
    }
    (held, log)
}

/// One `POST` with a JSON body, and what the app answered.
fn post(app: &Client, route: &str, body: &Value) -> Response {
    app.send(Request::post(route).json(body))
        .expect("the trigger's route answers")
}

/// One `GET`, and what the app answered.
fn get(app: &Client, route: &str) -> Response {
    app.send(Request::get(route)).expect("the route answers")
}

/// Poll `probe` until it answers, or fail naming what never happened.
fn eventually<T>(budget: Duration, what: &str, mut probe: impl FnMut() -> Option<T>) -> T {
    let deadline = Instant::now() + budget;
    loop {
        if let Some(answer) = probe() {
            return answer;
        }
        assert!(
            Instant::now() < deadline,
            "{what} did not happen within {budget:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// One lifecycle row, as the journal holds it.
#[derive(Debug, Clone)]
struct Lifecycle {
    id: String,
    inputs: Value,
    status: String,
    started_at: String,
    ended_at: Option<String>,
}

/// Every lifecycle row a trigger's executions left, in the order they began.
fn executions_of(project: &Path, trigger: &str) -> Vec<Lifecycle> {
    harness::journal_rows(
        project,
        &format!(
            "SELECT id, inputs, status, started_at, ended_at FROM executions \
             WHERE trigger_kind = '{trigger}' ORDER BY started_at, rowid"
        ),
    )
    .as_array()
    .expect("rows")
    .iter()
    .map(|row| Lifecycle {
        id: row["id"].as_str().expect("an id").to_string(),
        inputs: serde_json::from_str(row["inputs"].as_str().expect("inputs")).expect("JSON"),
        status: row["status"].as_str().expect("a status").to_string(),
        started_at: row["started_at"].as_str().expect("a start").to_string(),
        ended_at: row["ended_at"].as_str().map(str::to_string),
    })
    .collect()
}

/// A trigger's intake rows, as the journal holds them, in insertion order.
fn intakes_of(project: &Path, trigger: &str) -> Vec<Value> {
    harness::journal_rows(
        project,
        &format!(
            "SELECT id, state, execution, waited, coalesced, held_by FROM intakes \
             WHERE trigger_name = '{trigger}' ORDER BY rowid"
        ),
    )
    .as_array()
    .expect("rows")
    .clone()
}

/// The executions of a trigger that are not those of a child (a lineage row).
fn roots_of(project: &Path, trigger: &str) -> Vec<Lifecycle> {
    let children: Vec<String> = harness::journal_rows(project, "SELECT execution FROM lineage")
        .as_array()
        .expect("rows")
        .iter()
        .map(|row| row["execution"].as_str().expect("an id").to_string())
        .collect();
    executions_of(project, trigger)
        .into_iter()
        .filter(|row| !children.contains(&row.id))
        .collect()
}

/// No two of these lifecycle rows were ever open at the same instant: each
/// began at or after the one before it ended.
fn never_overlapping(rows: &[Lifecycle]) {
    for pair in rows.windows(2) {
        let ended = pair[0]
            .ended_at
            .as_deref()
            .unwrap_or_else(|| panic!("{} never ended: {pair:#?}", pair[0].id));
        assert!(
            pair[1].started_at.as_str() >= ended,
            "{} began at {} while {} was still open (it ended at {ended}): two of one \
             group's executions ran at once",
            pair[1].id,
            pair[1].started_at,
            pair[0].id
        );
    }
}

/// The tally's lines.
fn tally(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// An execution's id off a `202`.
fn execution_of(answered: &Response) -> String {
    assert_eq!(answered.status, 202, "{}", answered.text());
    answered.json()["execution_id"]
        .as_str()
        .unwrap_or_else(|| panic!("an execution id: {}", answered.text()))
        .to_string()
}

/// An intake's id off a `202`.
fn intake_id_of(answered: &Response) -> String {
    assert_eq!(answered.status, 202, "{}", answered.text());
    answered.json()["intake_id"]
        .as_str()
        .unwrap_or_else(|| panic!("an intake id: {}", answered.text()))
        .to_string()
}

/// **(1) Three identical deliveries start one execution, across a restart; a
/// fourth after the window starts another** (PRD resolved q77 ruling a).
///
/// `deduped`'s key is the body's `id`, held 20 seconds. The first delivery
/// starts an execution; `serve` is killed (`SIGKILL`, nothing flushed) and
/// started again over the same journal; the second and third deliveries are
/// answered with the first's id and `X-AgentCompose-Deduped: true`, the same
/// shape a fresh start has, and start nothing — the dedupe row outlived the
/// process. Once the window has passed the same delivery starts a new
/// execution, and the row names it.
#[test]
fn identical_deliveries_inside_the_window_start_one_execution_across_a_restart() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-dedupe");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-dedupe") else {
        return;
    };
    let delivery = json!({ "tag": "push", "id": "delivery-7" });
    let first_at = Instant::now();
    let first = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let answered = post(&app, "/deduped", &delivery);
        assert_eq!(
            answered.header("x-agentcompose-deduped"),
            None,
            "{answered:?}"
        );
        execution_of(&answered)
        // Dropped here: `SIGKILL` to the whole group.
    };

    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    for repeat in ["second", "third"] {
        let answered = post(&app, "/deduped", &delivery);
        assert_eq!(
            answered.header("x-agentcompose-deduped"),
            Some("true"),
            "the {repeat} delivery is a repeat: {answered:?}"
        );
        assert_eq!(answered.status, 202, "{}", answered.text());
        let body = answered.json();
        assert_eq!(body["execution_id"], first, "{body}");
        assert_eq!(body["status_url"], format!("/executions/{first}"), "{body}");
    }
    assert!(
        first_at.elapsed() < Duration::from_secs(20),
        "the repeats landed inside the window, which is what they test"
    );
    assert_eq!(
        executions_of(&project, "deduped").len(),
        1,
        "three identical deliveries started one execution"
    );

    // After the window: a new execution, and the row is replaced.
    let wait = Duration::from_secs(21).saturating_sub(first_at.elapsed());
    std::thread::sleep(wait);
    let answered = post(&app, "/deduped", &delivery);
    assert_eq!(
        answered.header("x-agentcompose-deduped"),
        None,
        "{answered:?}"
    );
    let fourth = execution_of(&answered);
    assert_ne!(
        fourth, first,
        "a key seen again after its window starts again"
    );
    let rows = executions_of(&project, "deduped");
    assert_eq!(rows.len(), 2, "{rows:#?}");
    let held = harness::journal_rows(
        &project,
        "SELECT execution FROM dedupe WHERE trigger_name = 'deduped'",
    );
    assert_eq!(
        held,
        json!([{ "execution": fourth }]),
        "the row names the new execution"
    );
}

/// **(2) Twenty events for one key under `limit: 1` never run two at once,
/// and all of them run, first in first out** (PRD resolved q77 ruling b) —
/// and **(7)'s event half**: an event the `dropping` trigger's full group
/// cannot take leaves a `dropped` intake row naming the execution running it.
///
/// The program prints twenty events as fast as the intake takes them; each is
/// offered to both triggers. `serial` queues all but the first behind its one
/// slot: from the lifecycle rows, no execution of the group began before the
/// one before it ended, all twenty completed, and they began in the order the
/// events arrived — which is the order of their intake rows too.
#[test]
fn twenty_events_for_one_key_under_limit_one_run_one_at_a_time_in_order() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-twenty");
    let (environment, _log) = environment(&provider, &scratch, &[("FEED_TOTAL", "20")]);
    let Some(project) = harness::scratch_project("intake-twenty") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "intake-events", &environment) else {
        return;
    };
    let _app = Client::new(&served.base_url).expect("a client");

    // While they run: never more than one of the group open at an instant.
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut most_open = 0;
    loop {
        let rows = executions_of(&project, "serial");
        let open = rows.iter().filter(|row| row.status == "open").count();
        most_open = most_open.max(open);
        if rows.len() == 20 && rows.iter().all(|row| row.status == "completed") {
            break;
        }
        assert!(
            Instant::now() < deadline,
            "twenty executions did not complete: {rows:#?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(
        most_open <= 1,
        "{most_open} of the group's executions were open at once"
    );

    let rows = executions_of(&project, "serial");
    never_overlapping(&rows);
    let order: Vec<String> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag").to_string())
        .collect();
    let expected: Vec<String> = (1..=20).map(|seq| seq.to_string()).collect();
    assert_eq!(order, expected, "the queue drained first in first out");

    let intakes = intakes_of(&project, "serial");
    assert_eq!(intakes.len(), 20, "{intakes:#?}");
    assert!(
        intakes.iter().all(|row| row["state"] == "started"),
        "every queued arrival started: {intakes:#?}"
    );
    let started: Vec<&str> = intakes
        .iter()
        .map(|row| row["execution"].as_str().expect("an execution"))
        .collect();
    let begun: Vec<&str> = rows.iter().map(|row| row.id.as_str()).collect();
    assert_eq!(
        started, begun,
        "each intake row started the execution it names, in order"
    );
    assert!(
        intakes.iter().skip(1).all(|row| row["waited"] == 1),
        "all but the first arrival waited on the queue: {intakes:#?}"
    );

    // The drop half: what arrived while `dropping`'s one execution ran.
    let dropped: Vec<Value> = intakes_of(&project, "dropping")
        .into_iter()
        .filter(|row| row["state"] == "dropped")
        .collect();
    assert!(
        !dropped.is_empty(),
        "an event arriving at a full group was dropped"
    );
    let running: Vec<String> = executions_of(&project, "dropping")
        .iter()
        .map(|row| row.id.clone())
        .collect();
    for row in &dropped {
        let held = row["held_by"].as_str().expect("a running execution");
        assert!(
            running.iter().any(|id| id == held),
            "{row} names a running execution"
        );
    }
    assert_eq!(
        running.len() + dropped.len(),
        20,
        "every event was either started or dropped by `dropping`"
    );
}

/// **(3) `supersede` gives the newest event a full run** (PRD resolved q77
/// ruling b, q75).
///
/// The first event starts `flow.parent`, which detaches a ten-minute
/// `flow.child` and lingers ten minutes itself. The second event, paced to
/// arrive while that runs, supersedes it: the first parent settles
/// `cancelled`, and so does its child — the cancel cascades — while the second
/// parent and its child run to completion.
#[test]
fn a_superseding_event_cancels_the_running_execution_and_its_child_and_runs_whole() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-supersede");
    let (environment, log) = environment(
        &provider,
        &scratch,
        &[("FEED_TOTAL", "2"), ("FEED_PACE_MS", "4000")],
    );
    let Some(project) = harness::scratch_project("intake-supersede") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "intake-supersede", &environment) else {
        return;
    };
    let _app = Client::new(&served.base_url).expect("a client");

    eventually(
        Duration::from_secs(90),
        "both events' executions to settle",
        || {
            let rows = executions_of(&project, "newest");
            (rows.len() == 4 && rows.iter().all(|row| row.status != "open")).then_some(())
        },
    );
    let roots = roots_of(&project, "newest");
    assert_eq!(roots.len(), 2, "{roots:#?}");
    let first = roots
        .iter()
        .find(|row| row.inputs["tag"] == "1")
        .expect("the first event's execution");
    let second = roots
        .iter()
        .find(|row| row.inputs["tag"] == "2")
        .expect("the second event's execution");
    assert_eq!(
        first.status, "cancelled",
        "the running execution was superseded"
    );
    assert_eq!(second.status, "completed", "the newest event ran whole");
    let children: Vec<(String, String)> = harness::journal_rows(
        &project,
        "SELECT lineage.parent AS parent, executions.status AS status FROM lineage \
         JOIN executions ON executions.id = lineage.execution",
    )
    .as_array()
    .expect("rows")
    .iter()
    .map(|row| {
        (
            row["parent"].as_str().expect("a parent").to_string(),
            row["status"].as_str().expect("a status").to_string(),
        )
    })
    .collect();
    assert!(
        children.contains(&(first.id.clone(), "cancelled".to_string())),
        "the superseded execution's child was cancelled through the cascade: {children:?}"
    );
    assert!(
        children.contains(&(second.id.clone(), "completed".to_string())),
        "the newest execution's child ran whole: {children:?}"
    );
    let lines = tally(&log);
    assert!(
        lines.iter().any(|line| line == "2 end"),
        "the newest event's work reached its end: {lines:?}"
    );
    assert!(
        !lines.iter().any(|line| line == "1 end"),
        "the superseded event's work never did: {lines:?}"
    );
}

/// **(4) Deliveries under a debounce start one run, with the newest payload and
/// the count** (PRD resolved q77 ruling c).
///
/// The ruling's five deliveries ten seconds apart under `debounce: 30s`,
/// scaled by ten in real time: five a second apart under `debounce: 3s` (the
/// ratio is the ruling's, and the clock is the real one). Every delivery is
/// answered `202` with the same `intake_id`, pending; the one execution starts
/// once three seconds pass with no arrival, with the fifth payload and
/// `payload.coalesced == 5`.
#[test]
fn deliveries_under_a_debounce_start_one_run_with_the_newest_payload_and_the_count() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-debounce");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-debounce") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let mut intakes = Vec::new();
    for n in 1..=5 {
        let answered = post(&app, "/debounced", &json!({ "tag": format!("push {n}") }));
        let body = answered.json();
        assert_eq!(body["status"], "pending", "{body}");
        assert_eq!(body["coalesced"], n, "{body}");
        intakes.push(intake_id_of(&answered));
        assert!(
            executions_of(&project, "debounced").is_empty(),
            "nothing starts inside it"
        );
        if n < 5 {
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    assert!(
        intakes.iter().all(|id| *id == intakes[0]),
        "every arrival joined one debounce: {intakes:?}"
    );
    let rows = eventually(Duration::from_secs(30), "the debounced execution", || {
        let rows = executions_of(&project, "debounced");
        rows.iter()
            .all(|row| row.status == "completed")
            .then_some(rows)
            .filter(|r| !r.is_empty())
    });
    assert_eq!(rows.len(), 1, "one execution: {rows:#?}");
    assert_eq!(
        rows[0].inputs["tag"], "push 5",
        "the newest payload: {:?}",
        rows[0]
    );
    assert_eq!(
        rows[0].inputs["pushes"], 5,
        "`payload.coalesced` counts the five"
    );
    assert_eq!(tally(&log), ["push 5 start 5", "push 5 end"], "one run");
    let report = get(&app, &format!("/intakes/{}", intakes[0])).json();
    assert_eq!(report["state"], "started", "{report}");
    assert_eq!(report["execution_id"], rows[0].id.as_str(), "{report}");
    assert_eq!(report["coalesced"], 5, "{report}");
}

/// **A pending row holds the request, and never the credential the trigger's
/// `auth:` read** (`docs/durability.md` §3.15, §8, Decision D163's Status).
///
/// `guarded_burst` debounces behind a bearer token. The row the arrival opens
/// holds its payload — the debounce binds it again when it closes — and that
/// payload carries every header the request did except `authorization`, so the
/// token is written nowhere in the journal; the execution the debounce starts
/// still binds the other headers it reads.
#[test]
fn a_pending_row_holds_the_request_without_the_credential_its_auth_read() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-credential");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-credential") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let answered = app
        .send(
            Request::post("/guarded-burst")
                .header("Authorization", format!("Bearer {TOKEN}"))
                .header("X-Note", "kept")
                .json(&json!({ "tag": "burst" })),
        )
        .expect("the route answers");
    let intake = intake_id_of(&answered);
    assert_eq!(answered.json()["status"], "pending", "{}", answered.text());

    let held = harness::journal_rows(
        &project,
        &format!("SELECT payload FROM intakes WHERE id = '{intake}'"),
    );
    let payload: Value = serde_json::from_str(
        held[0]["payload"]
            .as_str()
            .unwrap_or_else(|| panic!("the pending row holds its payload: {held}")),
    )
    .expect("a JSON payload");
    let headers = payload["headers"]
        .as_object()
        .expect("the request's headers");
    assert_eq!(headers.get("x-note"), Some(&json!("kept")), "{payload}");
    assert!(
        !headers
            .keys()
            .any(|name| name.eq_ignore_ascii_case("authorization")),
        "the credential the trigger's `auth:` read is not journaled: {payload}"
    );
    let journal = std::fs::read_dir(project.join(".agent-compose"))
        .expect("the project's data directory")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().contains("journal"))
        .map(|entry| std::fs::read(entry.path()).unwrap_or_default())
        .collect::<Vec<_>>();
    assert!(
        journal.iter().all(|bytes| !bytes
            .windows(TOKEN.len())
            .any(|window| window == TOKEN.as_bytes())),
        "the token appears nowhere in the journal's files"
    );

    let rows = eventually(Duration::from_secs(30), "the debounced execution", || {
        let rows = executions_of(&project, "guarded_burst");
        rows.iter()
            .all(|row| row.status == "completed")
            .then_some(rows)
            .filter(|r| !r.is_empty())
    });
    assert_eq!(rows.len(), 1, "one execution: {rows:#?}");
    assert_eq!(rows[0].inputs["tag"], "kept burst", "{:?}", rows[0]);
    assert_eq!(
        tally(&log),
        ["kept burst start 1", "kept burst end"],
        "one run"
    );
}

/// **(5) A restart mid-queue starts every queued arrival once, in order**
/// (PRD resolved q77 ruling f).
///
/// `serial`'s first delivery starts a three-second execution; three more
/// queue behind it. `serve` is killed with the three still queued and started
/// again over the journal: the first is recovered and finishes, and the three
/// start one at a time in the order they were admitted — none lost, none
/// started twice, and never two at once.
#[test]
fn a_restart_with_three_queued_arrivals_starts_each_once_in_order() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-restart-queue");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-restart-queue") else {
        return;
    };
    let queued = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        execution_of(&post(
            &app,
            "/serial",
            &json!({ "tag": "first", "linger": "3" }),
        ));
        let queued: Vec<String> = ["second", "third", "fourth"]
            .iter()
            .map(|tag| {
                let answered = post(&app, "/serial", &json!({ "tag": tag, "linger": "1" }));
                assert_eq!(answered.json()["status"], "queued", "{}", answered.text());
                intake_id_of(&answered)
            })
            .collect();
        let rows = intakes_of(&project, "serial");
        assert_eq!(
            rows.iter().filter(|row| row["state"] == "queued").count(),
            3,
            "three arrivals are queued when serve dies: {rows:#?}"
        );
        queued
    };

    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let rows = eventually(
        Duration::from_secs(60),
        "four executions to complete",
        || {
            let rows = executions_of(&project, "serial");
            (rows.len() == 4 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag"))
        .collect();
    assert_eq!(
        order,
        ["first", "second", "third", "fourth"],
        "in admission order"
    );
    for (id, expected) in queued.iter().zip(&rows[1..]) {
        let report = get(&app, &format!("/intakes/{id}")).json();
        assert_eq!(report["state"], "started", "{report}");
        assert_eq!(report["execution_id"], expected.id.as_str(), "{report}");
    }
    // Settled for good: nothing more starts.
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        executions_of(&project, "serial").len(),
        4,
        "none started twice"
    );
}

/// **(5) A restart mid-debounce fires the pending debounce once** (PRD
/// resolved q77 ruling f).
///
/// Two deliveries open one debounce; `serve` is killed before it closes and
/// started again more than its three seconds later, so the deadline the row
/// holds is already past: it fires at once, once, with both arrivals counted.
#[test]
fn a_restart_with_a_debounce_pending_fires_it_once() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-restart-debounce");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-restart-debounce") else {
        return;
    };
    let pending = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let id = intake_id_of(&post(&app, "/debounced", &json!({ "tag": "one" })));
        let again = intake_id_of(&post(&app, "/debounced", &json!({ "tag": "two" })));
        assert_eq!(id, again);
        id
    };
    assert_eq!(
        intakes_of(&project, "debounced")[0]["state"],
        "pending",
        "the debounce is open when serve dies"
    );
    std::thread::sleep(Duration::from_secs(4));

    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let rows = eventually(Duration::from_secs(30), "the debounced execution", || {
        let rows = executions_of(&project, "debounced");
        rows.iter()
            .all(|row| row.status == "completed")
            .then_some(rows)
            .filter(|r| !r.is_empty())
    });
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        executions_of(&project, "debounced").len(),
        1,
        "once: {rows:#?}"
    );
    assert_eq!(rows[0].inputs["tag"], "two", "{rows:#?}");
    assert_eq!(rows[0].inputs["pushes"], 2, "{rows:#?}");
    assert_eq!(tally(&log), ["two start 2", "two end"]);
    let report = get(&app, &format!("/intakes/{pending}")).json();
    assert_eq!(report["state"], "started", "{report}");
}

/// **(6) A `run` of a flow a trigger's key-less group covers is refused while
/// the group is full** (PRD resolved q77 ruling e).
///
/// `held`'s one slot is taken by a served execution waiting for a file. A
/// `run` of the same flow, over the same journal, exits `2` naming that
/// execution and starts and journals nothing; once the file appears and the
/// served execution settles, the same `run` succeeds.
#[test]
fn a_run_of_a_flow_whose_group_is_full_is_refused_and_journals_nothing() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-run-refused");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-run-refused") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let hold = scratch.path().join("release");
    let hold_text = hold.display().to_string();
    let running = execution_of(&post(
        &app,
        "/held",
        &json!({ "tag": "served", "hold": hold_text }),
    ));
    eventually(Duration::from_secs(10), "the held execution's row", || {
        (executions_of(&project, "held").len() == 1).then_some(())
    });

    let inputs = [("tag", "by hand"), ("hold", hold_text.as_str())];
    let refused = harness::run_into(&project, FIXTURE, "flow.held", &inputs, None, &environment);
    assert_eq!(
        refused.output.status.code(),
        Some(2),
        "{}",
        refused.stderr()
    );
    assert!(
        refused.stderr().contains(&format!(
            "error: `flow.held` is covered by the `concurrency:` of the trigger `held`, and its group has no free slot: {running} holds it (1 of 1 running). A run is refused rather than queued — a terminal is not a queue — so nothing was started or journaled: run it again once that execution settles (grammar 13.2, PRD resolved q77)"
        )),
        "the refusal names the running execution: {}",
        refused.stderr()
    );
    assert!(
        executions_of(&project, "manual").is_empty(),
        "a refused run journals nothing"
    );

    std::fs::write(&hold, "").expect("the release file");
    eventually(
        Duration::from_secs(20),
        "the served execution to settle",
        || (executions_of(&project, "held")[0].status == "completed").then_some(()),
    );
    let ran = harness::run_into(&project, FIXTURE, "flow.held", &inputs, None, &environment);
    assert_eq!(ran.output.status.code(), Some(0), "{}", ran.stderr());
    assert_eq!(
        executions_of(&project, "manual").len(),
        1,
        "the run, once the slot freed"
    );
}

/// **(7) An arrival its full group drops is answered with the running
/// execution** (PRD resolved q77 ruling b).
///
/// `dropping`'s one slot is held by a five-second execution; a second delivery
/// is answered `202` with that execution's id, `X-AgentCompose-Dropped: true`
/// and the intake id of the `dropped` row it leaves, and starts nothing.
#[test]
fn an_arrival_its_full_group_drops_is_answered_with_the_running_execution() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-drop");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-drop") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let running = execution_of(&post(
        &app,
        "/dropping",
        &json!({ "tag": "a", "linger": "5" }),
    ));
    let answered = post(&app, "/dropping", &json!({ "tag": "b", "linger": "5" }));
    assert_eq!(
        answered.header("x-agentcompose-dropped"),
        Some("true"),
        "{answered:?}"
    );
    let body = answered.json();
    assert_eq!(body["execution_id"], running.as_str(), "{body}");
    assert_eq!(
        body["status_url"],
        format!("/executions/{running}"),
        "{body}"
    );
    let intake = body["intake_id"]
        .as_str()
        .expect("an intake id")
        .to_string();
    let report = get(&app, &format!("/intakes/{intake}")).json();
    assert_eq!(report["state"], "dropped", "{report}");
    assert_eq!(report["running_execution_id"], running.as_str(), "{report}");
    assert_eq!(
        executions_of(&project, "dropping").len(),
        1,
        "nothing else started"
    );
}

/// **(8) `GET /intakes/:id`** (PRD resolved q77 ruling d): `404` for an id the
/// journal does not hold; each state's shape; the trigger's `auth:` enforced
/// exactly as an execution's routes enforce it; and a started intake answers
/// after its execution has settled.
#[test]
fn the_intake_route_reports_each_state_under_the_triggers_auth() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-route");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-route") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let bearer = format!("Bearer {TOKEN}");

    let unknown = get(&app, "/intakes/int_00000000-0000-0000-0000-000000000000");
    assert_eq!(unknown.status, 404, "{}", unknown.text());
    assert_eq!(
        unknown.json()["error"],
        "no intake `int_00000000-0000-0000-0000-000000000000` is held in this project's journal"
    );

    // Queued, behind the bearer token.
    let guarded = |body: &Value| {
        app.send(
            Request::post("/guarded")
                .header("Authorization", &bearer)
                .json(body),
        )
        .expect("the route answers")
    };
    let running = execution_of(&guarded(&json!({ "tag": "a", "linger": "2" })));
    let queued_answer = guarded(&json!({ "tag": "b", "linger": "0" }));
    let queued = intake_id_of(&queued_answer);
    assert_eq!(
        queued_answer.json(),
        json!({ "intake_id": queued, "status": "queued", "status_url": format!("/intakes/{queued}") })
    );
    let refused = get(&app, &format!("/intakes/{queued}"));
    assert_eq!(
        refused.status,
        401,
        "an unauthenticated caller is refused: {}",
        refused.text()
    );
    assert_eq!(refused.json()["trigger"], "guarded");
    let wrong = app
        .send(Request::get(format!("/intakes/{queued}")).header("Authorization", "Bearer nope"))
        .expect("the route answers");
    assert_eq!(wrong.status, 401, "{}", wrong.text());
    let report = app
        .send(Request::get(format!("/intakes/{queued}")).header("Authorization", &bearer))
        .expect("the route answers")
        .json();
    assert_eq!(report["intake_id"], queued.as_str(), "{report}");
    assert_eq!(report["trigger"], "guarded", "{report}");
    assert_eq!(report["state"], "queued", "{report}");
    assert_eq!(report["coalesced"], 1, "{report}");
    assert!(report["admitted_at"].is_string(), "{report}");
    assert!(
        report.get("execution_id").is_none(),
        "nothing has started: {report}"
    );

    // Pending.
    let pending = intake_id_of(&post(&app, "/debounced", &json!({ "tag": "p" })));
    let report = get(&app, &format!("/intakes/{pending}")).json();
    assert_eq!(report["state"], "pending", "{report}");
    assert_eq!(report["coalesced"], 1, "{report}");
    assert!(
        report["deadline"].is_string(),
        "a pending intake names its deadline: {report}"
    );

    // Started, and still answering once its execution has settled.
    let started = eventually(
        Duration::from_secs(30),
        "the queued intake to start",
        || {
            let report = app
                .send(Request::get(format!("/intakes/{queued}")).header("Authorization", &bearer))
                .expect("the route answers")
                .json();
            (report["state"] == "started").then_some(report)
        },
    );
    let execution = started["execution_id"]
        .as_str()
        .expect("an execution")
        .to_string();
    assert_ne!(execution, running);
    assert_eq!(
        started["status_url"],
        format!("/executions/{execution}"),
        "{started}"
    );
    assert!(started["started_at"].is_string(), "{started}");
    eventually(Duration::from_secs(30), "its execution to settle", || {
        executions_of(&project, "guarded")
            .iter()
            .any(|row| row.id == execution && row.status == "completed")
            .then_some(())
    });
    let after = app
        .send(Request::get(format!("/intakes/{queued}")).header("Authorization", &bearer))
        .expect("the route answers")
        .json();
    assert_eq!(after["state"], "started", "{after}");
    assert_eq!(after["execution_id"], execution.as_str(), "{after}");
}

/// **(11) A trigger's `max_running:` bounds its groups together** (PRD
/// resolved q77 ruling b).
///
/// `capped` gives each body `group` a slot of one, and the trigger
/// `max_running: 1` across them: two deliveries to two groups start one at a
/// time — the second queued until the first settles — never two at once.
#[test]
fn the_triggers_max_running_bounds_its_groups_together() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-max-running");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-max-running") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    execution_of(&post(
        &app,
        "/capped",
        &json!({ "tag": "a", "group": "a", "linger": "2" }),
    ));
    let second = post(
        &app,
        "/capped",
        &json!({ "tag": "b", "group": "b", "linger": "1" }),
    );
    assert_eq!(second.json()["status"], "queued", "{}", second.text());
    let rows = eventually(
        Duration::from_secs(30),
        "both executions to complete",
        || {
            let rows = executions_of(&project, "capped");
            (rows.len() == 2 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    let order: BTreeMap<&str, &str> = rows
        .iter()
        .map(|row| (row.id.as_str(), row.inputs["tag"].as_str().expect("a tag")))
        .collect();
    assert_eq!(order.len(), 2);
    assert_eq!(rows[0].inputs["tag"], "a");
    assert_eq!(rows[1].inputs["tag"], "b");
}
