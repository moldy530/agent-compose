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
//!     [`the_triggers_max_running_bounds_its_groups_together`], and a `run`
//!     refused by it — [`a_run_is_refused_by_its_triggers_max_running_below_its_groups_limit`].
//!
//! And what the surfaces combine into:
//!
//! * keyed groups are leases of their own —
//!   [`two_keyed_groups_run_side_by_side_while_a_busy_one_queues`];
//! * a `dedupe_key:` beside a debounce —
//!   [`a_repeat_of_an_arrival_a_debounce_counted_is_answered_and_not_counted_again`],
//!   and across a crash inside an arrival —
//!   [`a_crash_inside_a_debounced_arrival_neither_loses_it_nor_counts_it_twice`];
//! * beside a group that queues or drops —
//!   [`a_repeat_of_a_queued_or_dropped_arrival_is_answered_with_what_the_first_got`];
//! * a key that answers `''` —
//!   [`an_arrival_whose_dedupe_key_answers_the_empty_string_is_refused`];
//! * `respond: sync` on every intake path, and a repeat after a restart —
//!   [`under_respond_sync_an_intake_answers_at_once_and_a_repeat_with_the_first`];
//! * deliveries racing for the last slot —
//!   [`deliveries_racing_for_one_slot_start_one_and_queue_or_drop_the_rest`];
//! * an empty `session_key:` a flow needs, refused before a dedupe key is
//!   claimed as before a group is asked —
//!   [`an_empty_session_key_is_refused_before_a_dedupe_key_or_a_group_is_claimed`];
//! * a queued arrival a new build no longer binds, dropped while the queue
//!   moves on past it —
//!   [`a_queued_arrival_the_new_build_no_longer_binds_is_dropped_and_the_queue_moves_on`];
//! * a start the journal refused, owed, and the arrival queued behind it —
//!   [`a_start_the_journal_refused_twice_is_owed_and_started_once_it_recovers`];
//! * the serve log's lines — [`the_serve_log_names_each_intake_decision`], and
//!   the supersede line in (3).

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
    let Some(mut served) = harness::serve_into(&project, "intake-supersede", &environment) else {
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
    let said = served.stop_and_read_stderr();
    let superseded = format!("intake `newest`: superseded {} for {}", first.id, second.id);
    assert!(
        said.lines().any(|line| line == superseded),
        "the serve log says `{superseded}`:\n{said}"
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

/// The `X-AgentCompose-Deduped` header an answer carries, if any.
fn deduped(answered: &Response) -> Option<&str> {
    answered.header("x-agentcompose-deduped")
}

/// One pending row of a trigger, as the journal holds it — its id, the count,
/// its payload and the id its newest counted arrival's gate was claimed under.
fn pending_of(project: &Path, trigger: &str) -> Value {
    let rows = harness::journal_rows(
        project,
        &format!(
            "SELECT id, execution, coalesced, payload, joined_execution FROM intakes \
             WHERE trigger_name = '{trigger}' AND state = 'pending'"
        ),
    );
    let rows = rows.as_array().expect("rows");
    assert_eq!(rows.len(), 1, "one open debounce: {rows:#?}");
    rows[0].clone()
}

/// The execution each of a trigger's dedupe keys names, by key.
fn dedupe_of(project: &Path, trigger: &str) -> BTreeMap<String, String> {
    harness::journal_rows(
        project,
        &format!("SELECT dedupe_key, execution FROM dedupe WHERE trigger_name = '{trigger}'"),
    )
    .as_array()
    .expect("rows")
    .iter()
    .map(|row| {
        let key: String =
            serde_json::from_str(row["dedupe_key"].as_str().expect("a key")).expect("JSON");
        (
            key,
            row["execution"].as_str().expect("an execution").to_string(),
        )
    })
    .collect()
}

/// **A keyed group is its own lease** (PRD resolved q77 ruling b): two groups
/// run side by side, and an arrival for a busy group queues behind it alone.
///
/// `keyed` gives each body `group` one slot and declares no `max_running:`.
/// An arrival for `a` and one for `b` both start at once — both lifecycle rows
/// open together — and a second arrival for `a` queues; it starts only once
/// `a`'s first execution settles, while `b`'s is still running.
#[test]
fn two_keyed_groups_run_side_by_side_while_a_busy_one_queues() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-keyed");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-keyed") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let hold = |name: &str| scratch.path().join(name).display().to_string();
    let first_a = execution_of(&post(
        &app,
        "/keyed",
        &json!({ "tag": "a1", "group": "a", "hold": hold("a1") }),
    ));
    let answered_b = post(
        &app,
        "/keyed",
        &json!({ "tag": "b1", "group": "b", "hold": hold("b1") }),
    );
    assert!(
        answered_b.json().get("intake_id").is_none(),
        "another group's arrival starts rather than waiting: {}",
        answered_b.text()
    );
    let first_b = execution_of(&answered_b);
    eventually(
        Duration::from_secs(10),
        "both groups' executions to be open at once",
        || {
            let rows = executions_of(&project, "keyed");
            (rows.len() == 2 && rows.iter().all(|row| row.status == "open")).then_some(())
        },
    );

    let second_a = post(
        &app,
        "/keyed",
        &json!({ "tag": "a2", "group": "a", "hold": hold("a2") }),
    );
    assert_eq!(
        second_a.json()["status"],
        "queued",
        "a busy group's arrival queues: {}",
        second_a.text()
    );
    let queued = intake_id_of(&second_a);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        executions_of(&project, "keyed").len(),
        2,
        "nothing more starts while `a` is busy"
    );

    std::fs::write(hold("a1"), "").expect("the release file");
    let started = eventually(
        Duration::from_secs(20),
        "the queued `a` arrival to start",
        || {
            let report = get(&app, &format!("/intakes/{queued}")).json();
            (report["state"] == "started").then_some(report)
        },
    );
    let rows = executions_of(&project, "keyed");
    let b = rows
        .iter()
        .find(|row| row.id == first_b)
        .expect("`b`'s execution");
    assert_eq!(
        b.status, "open",
        "`b` still runs while `a`'s queue moves: {rows:#?}"
    );
    let a_rows: Vec<Lifecycle> = rows
        .iter()
        .filter(|row| row.inputs["tag"] == "a1" || row.inputs["tag"] == "a2")
        .cloned()
        .collect();
    assert_eq!(a_rows.len(), 2, "{rows:#?}");
    assert_eq!(a_rows[0].id, first_a);
    assert_eq!(started["execution_id"], a_rows[1].id.as_str(), "{started}");
    never_overlapping(&a_rows);

    std::fs::write(hold("a2"), "").expect("the release file");
    std::fs::write(hold("b1"), "").expect("the release file");
    eventually(Duration::from_secs(20), "all three to complete", || {
        let rows = executions_of(&project, "keyed");
        (rows.len() == 3 && rows.iter().all(|row| row.status == "completed")).then_some(())
    });
}

/// **A repeat of an arrival a debounce counted is a repeat** (PRD resolved q77
/// ruling a, c, grammar 13.3's answer table).
///
/// `burst` keys on the body's `id` beside a ten-second debounce. Three
/// identical deliveries open one debounce and count one arrival: the first is
/// answered `pending` with `coalesced: 1`, and the two after it carry
/// `X-AgentCompose-Deduped: true` and the same intake, still `coalesced: 1`. A
/// different key is the second arrival. The one execution reads `pushes: 2` and
/// the newest payload; each key's row names that execution, and a repeat after
/// it started is answered with it.
#[test]
fn a_repeat_of_an_arrival_a_debounce_counted_is_answered_and_not_counted_again() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-burst");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-burst") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let opened_at = Instant::now();
    let same = json!({ "tag": "same", "id": "delivery-1" });
    let first = post(&app, "/burst", &same);
    assert_eq!(deduped(&first), None, "{first:?}");
    let intake = intake_id_of(&first);
    assert_eq!(first.json()["coalesced"], 1, "{}", first.text());
    for repeat in ["second", "third"] {
        let answered = post(&app, "/burst", &same);
        assert_eq!(
            deduped(&answered),
            Some("true"),
            "the {repeat} identical delivery is a repeat: {answered:?}"
        );
        let body = answered.json();
        assert_eq!(body["intake_id"], intake.as_str(), "{body}");
        assert_eq!(body["status"], "pending", "{body}");
        assert_eq!(
            body["coalesced"], 1,
            "a repeat is not counted again: {body}"
        );
        assert_eq!(body["status_url"], format!("/intakes/{intake}"), "{body}");
    }
    let other = post(
        &app,
        "/burst",
        &json!({ "tag": "other", "id": "delivery-2" }),
    );
    assert_eq!(deduped(&other), None, "{other:?}");
    assert_eq!(intake_id_of(&other), intake);
    assert_eq!(other.json()["coalesced"], 2, "{}", other.text());
    let again = post(
        &app,
        "/burst",
        &json!({ "tag": "other", "id": "delivery-2" }),
    );
    assert_eq!(deduped(&again), Some("true"), "{again:?}");
    assert_eq!(again.json()["coalesced"], 2, "{}", again.text());
    assert!(
        opened_at.elapsed() < Duration::from_secs(10),
        "every delivery landed inside the debounce, which is what this tests"
    );

    let rows = eventually(Duration::from_secs(30), "the debounced execution", || {
        let rows = executions_of(&project, "burst");
        rows.iter()
            .all(|row| row.status == "completed")
            .then_some(rows)
            .filter(|r| !r.is_empty())
    });
    assert_eq!(rows.len(), 1, "one execution: {rows:#?}");
    assert_eq!(
        rows[0].inputs["tag"], "other",
        "the newest payload: {rows:#?}"
    );
    assert_eq!(
        rows[0].inputs["pushes"], 2,
        "two arrivals were counted, not five: {rows:#?}"
    );
    assert_eq!(tally(&log), ["other start 2", "other end"], "one run");
    let keys = dedupe_of(&project, "burst");
    assert_eq!(keys.len(), 2, "{keys:?}");
    assert!(
        keys.values().all(|execution| *execution == rows[0].id),
        "every counted arrival's key names the execution the debounce started: {keys:?}"
    );

    let late = post(&app, "/burst", &same);
    assert_eq!(deduped(&late), Some("true"), "{late:?}");
    assert_eq!(execution_of(&late), rows[0].id, "{}", late.text());
    assert_eq!(
        executions_of(&project, "burst").len(),
        1,
        "nothing more started"
    );
}

/// **A crash between a debounced arrival's claim and its count neither loses
/// it nor counts it twice** (Decision D163's Status, `docs/durability.md`
/// §3.15, PRD resolved q77 ruling f).
///
/// Three arrivals open and extend one `burst` debounce; `serve` is killed, and
/// the journal is put in the two states a crash inside an arrival can leave:
///
///  * **claimed, not counted** — the second key's row names an id nothing
///    records, and the pending row does not count it;
///  * **counted, not folded** — the third key's row still names the id its
///    arrival claimed it under, which the pending row records as its newest.
///
/// Restarted, the third and first are repeats — answered with the intake and
/// counted once — while the second, delivered again, is counted: one execution,
/// three arrivals, with the second's payload, the newest to arrive.
#[test]
fn a_crash_inside_a_debounced_arrival_neither_loses_it_nor_counts_it_twice() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-burst-crash");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-burst-crash") else {
        return;
    };
    let delivery = |n: u32, tag: &str| json!({ "tag": tag, "id": format!("d{n}") });
    let intake = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let intake = intake_id_of(&post(&app, "/burst", &delivery(1, "first")));
        post(&app, "/burst", &delivery(2, "second"));
        let third = post(&app, "/burst", &delivery(3, "third"));
        assert_eq!(third.json()["coalesced"], 3, "{}", third.text());
        intake
        // Dropped here: `SIGKILL` to the whole group.
    };
    let row = pending_of(&project, "burst");
    let own = row["execution"]
        .as_str()
        .expect("the row's execution")
        .to_string();
    let newest = row["joined_execution"]
        .as_str()
        .expect("the row records its newest arrival's id")
        .to_string();
    assert_ne!(
        newest, own,
        "a joining arrival claims under an id of its own"
    );
    let surgery = harness::journal_sql(
        &project,
        &format!(
            "UPDATE dedupe SET execution = 'exec_claimed_and_never_counted' \
               WHERE trigger_name = 'burst' AND dedupe_key = '\"d2\"';\n\
             UPDATE dedupe SET execution = '{newest}' \
               WHERE trigger_name = 'burst' AND dedupe_key = '\"d3\"';\n\
             UPDATE intakes SET coalesced = 2 WHERE id = '{intake}';\n"
        ),
    );
    assert!(
        surgery.status.success(),
        "{}",
        String::from_utf8_lossy(&surgery.stderr)
    );

    let restarted_at = Instant::now();
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    eventually(
        Duration::from_secs(10),
        "the restart to fold the counted arrival's key onto the row's execution",
        || (dedupe_of(&project, "burst").get("d3") == Some(&own)).then_some(()),
    );
    for n in [3, 1] {
        let answered = post(&app, "/burst", &delivery(n, "again"));
        assert_eq!(
            deduped(&answered),
            Some("true"),
            "d{n} was counted before the crash: {answered:?}"
        );
        assert_eq!(answered.json()["intake_id"], intake.as_str());
        assert_eq!(answered.json()["coalesced"], 2, "{}", answered.text());
    }
    let recounted = post(&app, "/burst", &delivery(2, "second"));
    assert_eq!(
        deduped(&recounted),
        None,
        "d2's claim was never counted, so its delivery is not a repeat: {recounted:?}"
    );
    assert_eq!(recounted.json()["intake_id"], intake.as_str());
    assert_eq!(recounted.json()["coalesced"], 3, "{}", recounted.text());
    assert!(
        restarted_at.elapsed() < Duration::from_secs(10),
        "the deliveries landed inside the debounce the row still held"
    );

    let rows = eventually(Duration::from_secs(30), "the debounced execution", || {
        let rows = executions_of(&project, "burst");
        rows.iter()
            .all(|row| row.status == "completed")
            .then_some(rows)
            .filter(|r| !r.is_empty())
    });
    assert_eq!(rows.len(), 1, "one execution: {rows:#?}");
    assert_eq!(rows[0].id, own, "the row's own execution: {rows:#?}");
    assert_eq!(rows[0].inputs["pushes"], 3, "three arrivals: {rows:#?}");
    assert_eq!(
        rows[0].inputs["tag"], "second",
        "the newest payload: {rows:#?}"
    );
    assert_eq!(tally(&log), ["second start 3", "second end"]);
    let keys = dedupe_of(&project, "burst");
    assert!(
        ["d1", "d2", "d3"]
            .iter()
            .all(|key| keys.get(*key) == Some(&own)),
        "every key names the one execution: {keys:?}"
    );
}

/// **A repeat of a queued arrival is that arrival, and a repeat of a dropped
/// one is the execution it was dropped for** (grammar 13.3's answer table,
/// Decision D163's Status) — a key beside a group that queues, and one that
/// drops.
#[test]
fn a_repeat_of_a_queued_or_dropped_arrival_is_answered_with_what_the_first_got() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-dedupe-groups");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-dedupe-groups") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");

    // Queued.
    let running = execution_of(&post(
        &app,
        "/deduped-serial",
        &json!({ "tag": "a", "id": "a", "linger": "3" }),
    ));
    let queued_answer = post(
        &app,
        "/deduped-serial",
        &json!({ "tag": "b", "id": "b", "linger": "0" }),
    );
    let queued = intake_id_of(&queued_answer);
    let repeat = post(
        &app,
        "/deduped-serial",
        &json!({ "tag": "b", "id": "b", "linger": "0" }),
    );
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(
        repeat.json(),
        json!({ "intake_id": queued, "status": "queued", "status_url": format!("/intakes/{queued}") }),
        "a repeat of a queued arrival is answered with that arrival"
    );
    let repeat = post(
        &app,
        "/deduped-serial",
        &json!({ "tag": "a", "id": "a", "linger": "3" }),
    );
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(execution_of(&repeat), running);
    assert_eq!(
        intakes_of(&project, "deduped_serial").len(),
        2,
        "the repeats left no rows"
    );

    // Dropped.
    let holder = execution_of(&post(
        &app,
        "/deduped-dropping",
        &json!({ "tag": "x", "id": "x", "linger": "3" }),
    ));
    let dropped = post(
        &app,
        "/deduped-dropping",
        &json!({ "tag": "y", "id": "y", "linger": "0" }),
    );
    assert_eq!(
        dropped.header("x-agentcompose-dropped"),
        Some("true"),
        "{dropped:?}"
    );
    assert_eq!(execution_of(&dropped), holder);
    let repeat = post(
        &app,
        "/deduped-dropping",
        &json!({ "tag": "y", "id": "y", "linger": "0" }),
    );
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(
        execution_of(&repeat),
        holder,
        "a repeat of a dropped arrival names the execution it was dropped for"
    );
    assert_eq!(
        intakes_of(&project, "deduped_dropping").len(),
        2,
        "one started row, one dropped row, and nothing for the repeat"
    );

    let rows = eventually(Duration::from_secs(30), "both queued executions", || {
        let rows = executions_of(&project, "deduped_serial");
        (rows.len() == 2 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
    });
    assert_eq!(rows[0].id, running);
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(executions_of(&project, "deduped_serial").len(), 2);
    assert_eq!(executions_of(&project, "deduped_dropping").len(), 1);
}

/// **An arrival whose `dedupe_key:` answers the empty string is refused**
/// (Decision D163's Status): there is no key to tell a repeat of it by, so it
/// is a `400` that starts nothing and claims nothing, rather than one delivery
/// that every other empty-keyed request is deduplicated against.
#[test]
fn an_arrival_whose_dedupe_key_answers_the_empty_string_is_refused() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-empty-key");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-empty-key") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    for tag in ["e1", "e2"] {
        let answered = post(&app, "/deduped", &json!({ "tag": tag, "id": "" }));
        assert_eq!(answered.status, 400, "{}", answered.text());
        assert_eq!(
            answered.json(),
            json!({
                "error": "the trigger `deduped` could not read this request: its `dedupe_key:` answered the empty string, so there is no key to tell a repeat of it by (grammar 13.3)"
            })
        );
        assert_eq!(deduped(&answered), None, "{answered:?}");
    }
    assert!(
        executions_of(&project, "deduped").is_empty(),
        "nothing started"
    );
    assert!(
        dedupe_of(&project, "deduped").is_empty(),
        "nothing was claimed"
    );
}

/// **Under `respond: sync`** (grammar 13.3, Decision D163's Status): a queued
/// arrival and a dropped one are answered as they are under `async`, at once;
/// a repeat of a started one waits on the first's execution and is answered
/// with its outputs; and a repeat after a restart, whose execution settled
/// before it, is answered from its lifecycle row — the id, the status, the
/// status URL — since a settled row holds no outputs.
#[test]
fn under_respond_sync_an_intake_answers_at_once_and_a_repeat_with_the_first() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-sync");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-sync") else {
        return;
    };
    let (first, keyless) = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");

        // Queued under sync, while the first waits on its run.
        let waiting = {
            let app = app.clone();
            std::thread::spawn(move || {
                post(
                    &app,
                    "/sync-serial",
                    &json!({ "tag": "s1", "id": "s1", "linger": "3" }),
                )
            })
        };
        eventually(
            Duration::from_secs(10),
            "the first sync arrival's row",
            || (executions_of(&project, "sync_serial").len() == 1).then_some(()),
        );
        let queued_answer = post(
            &app,
            "/sync-serial",
            &json!({ "tag": "s2", "id": "s2", "linger": "0" }),
        );
        let queued = intake_id_of(&queued_answer);
        assert_eq!(
            queued_answer.json(),
            json!({ "intake_id": queued, "status": "queued", "status_url": format!("/intakes/{queued}") }),
            "a queued arrival is answered at once, sync or not"
        );
        let repeat = post(
            &app,
            "/sync-serial",
            &json!({ "tag": "s2", "id": "s2", "linger": "0" }),
        );
        assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
        assert_eq!(repeat.json()["intake_id"], queued.as_str());

        // Dropped under sync, while its group's one run lingers.
        let lingering = {
            let app = app.clone();
            std::thread::spawn(move || {
                post(
                    &app,
                    "/sync-dropping",
                    &json!({ "tag": "d1", "linger": "3" }),
                )
            })
        };
        let holder = eventually(Duration::from_secs(10), "the dropping run's row", || {
            executions_of(&project, "sync_dropping")
                .first()
                .map(|row| row.id.clone())
        });
        let dropped = post(
            &app,
            "/sync-dropping",
            &json!({ "tag": "d2", "linger": "0" }),
        );
        assert_eq!(
            dropped.header("x-agentcompose-dropped"),
            Some("true"),
            "{dropped:?}"
        );
        assert_eq!(execution_of(&dropped), holder);

        let first = waiting.join().expect("the first arrival's thread");
        assert_eq!(first.status, 200, "{}", first.text());
        let first_id = first.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        assert_eq!(
            first.json(),
            json!({ "execution_id": first_id, "status": "completed", "outputs": {} })
        );
        let lingered = lingering.join().expect("the dropping run's thread");
        assert_eq!(lingered.status, 200, "{}", lingered.text());

        // A repeat of the started one: the first's answer, outputs and all.
        let repeat = post(
            &app,
            "/sync-serial",
            &json!({ "tag": "s1", "id": "s1", "linger": "3" }),
        );
        assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
        assert_eq!(repeat.status, 200, "{}", repeat.text());
        assert_eq!(repeat.json(), first.json());

        // …and a sync key with no block, the same way.
        let keyless = post(&app, "/deduped-sync", &json!({ "tag": "one", "id": "k" }));
        assert_eq!(keyless.status, 200, "{}", keyless.text());
        let again = post(&app, "/deduped-sync", &json!({ "tag": "one", "id": "k" }));
        assert_eq!(deduped(&again), Some("true"), "{again:?}");
        assert_eq!(again.status, 200, "{}", again.text());
        assert_eq!(again.json(), keyless.json());
        let keyless_id = keyless.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        eventually(
            Duration::from_secs(20),
            "the queued sync arrival to settle",
            || {
                let rows = executions_of(&project, "sync_serial");
                (rows.len() == 2 && rows.iter().all(|row| row.status == "completed")).then_some(())
            },
        );
        (first_id, keyless_id)
        // Dropped here: `SIGKILL` to the whole group.
    };

    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    for (route, body, execution) in [
        (
            "/sync-serial",
            json!({ "tag": "s1", "id": "s1", "linger": "3" }),
            &first,
        ),
        (
            "/deduped-sync",
            json!({ "tag": "one", "id": "k" }),
            &keyless,
        ),
    ] {
        let answered = post(&app, route, &body);
        assert_eq!(deduped(&answered), Some("true"), "{answered:?}");
        assert_eq!(answered.status, 202, "{}", answered.text());
        assert_eq!(
            answered.json(),
            json!({
                "execution_id": execution,
                "status": "completed",
                "status_url": format!("/executions/{execution}"),
            }),
            "a repeat of an execution that settled before this process is answered from its row"
        );
    }
    assert_eq!(executions_of(&project, "sync_serial").len(), 2);
    assert_eq!(executions_of(&project, "deduped_sync").len(), 1);
}

/// **Deliveries racing for one slot**: one starts, and every other is queued
/// or dropped — never two started (PRD resolved q77 ruling b).
///
/// Eight deliveries to `serial` and six to `dropping` are released together
/// from as many threads. Exactly one of each is answered with a started
/// execution; the other seven are queued and run one at a time in some order,
/// and the other five are dropped naming the one that started.
#[test]
fn deliveries_racing_for_one_slot_start_one_and_queue_or_drop_the_rest() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-race");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-race") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let race = |route: &'static str, count: usize, linger: &'static str| -> Vec<Response> {
        let gate = std::sync::Arc::new(std::sync::Barrier::new(count));
        let threads: Vec<_> = (0..count)
            .map(|n| {
                let app = app.clone();
                let gate = std::sync::Arc::clone(&gate);
                std::thread::spawn(move || {
                    gate.wait();
                    post(
                        &app,
                        route,
                        &json!({ "tag": format!("racer {n}"), "linger": linger }),
                    )
                })
            })
            .collect();
        threads
            .into_iter()
            .map(|thread| thread.join().expect("a racer"))
            .collect()
    };

    let answers = race("/serial", 8, "1");
    let started: Vec<&Response> = answers
        .iter()
        .filter(|answered| answered.json().get("intake_id").is_none())
        .collect();
    assert_eq!(started.len(), 1, "one racer took the slot: {answers:#?}");
    assert!(
        answers
            .iter()
            .filter(|answered| answered.json().get("intake_id").is_some())
            .all(|answered| answered.status == 202 && answered.json()["status"] == "queued"),
        "every other racer queued: {answers:#?}"
    );
    let rows = eventually(
        Duration::from_secs(60),
        "eight executions to complete",
        || {
            let rows = executions_of(&project, "serial");
            (rows.len() == 8 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    assert_eq!(rows[0].id, execution_of(started[0]), "the winner ran first");
    let intakes = intakes_of(&project, "serial");
    assert_eq!(intakes.len(), 8, "{intakes:#?}");
    assert!(
        intakes.iter().all(|row| row["state"] == "started"),
        "each racer started once: {intakes:#?}"
    );

    let answers = race("/dropping", 6, "3");
    let started: Vec<String> = answers
        .iter()
        .filter(|answered| answered.header("x-agentcompose-dropped").is_none())
        .map(execution_of)
        .collect();
    assert_eq!(started.len(), 1, "one racer took the slot: {answers:#?}");
    for answered in answers
        .iter()
        .filter(|answered| answered.header("x-agentcompose-dropped").is_some())
    {
        assert_eq!(
            execution_of(answered),
            started[0],
            "a dropped racer names the one that started"
        );
    }
    assert_eq!(executions_of(&project, "dropping").len(), 1);
}

/// **The serve log names each intake decision** in the lines the emitted
/// `README.md` tabulates (Decision D163's Status): a queued arrival and the
/// execution it will start, the queued arrival starting, a drop and why, and a
/// debounce closing over its count.
#[test]
fn the_serve_log_names_each_intake_decision() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-log");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-log") else {
        return;
    };
    let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let running = execution_of(&post(
        &app,
        "/serial",
        &json!({ "tag": "first", "linger": "2" }),
    ));
    let queued = intake_id_of(&post(
        &app,
        "/serial",
        &json!({ "tag": "second", "linger": "0" }),
    ));
    let holder = execution_of(&post(
        &app,
        "/dropping",
        &json!({ "tag": "held", "linger": "2" }),
    ));
    let dropped = post(&app, "/dropping", &json!({ "tag": "late", "linger": "0" }));
    let dropped = dropped.json()["intake_id"]
        .as_str()
        .expect("the dropped row's id")
        .to_string();
    let pending = intake_id_of(&post(&app, "/debounced", &json!({ "tag": "one" })));
    post(&app, "/debounced", &json!({ "tag": "two" }));

    let next = eventually(
        Duration::from_secs(30),
        "the queued arrival to start",
        || {
            let report = get(&app, &format!("/intakes/{queued}")).json();
            (report["state"] == "started").then(|| {
                report["execution_id"]
                    .as_str()
                    .expect("an execution")
                    .to_string()
            })
        },
    );
    let closed = eventually(Duration::from_secs(30), "the debounce to close", || {
        let report = get(&app, &format!("/intakes/{pending}")).json();
        (report["state"] == "started").then(|| {
            report["execution_id"]
                .as_str()
                .expect("an execution")
                .to_string()
        })
    });
    let said = served.stop_and_read_stderr();
    for line in [
        format!("intake `serial`: {queued} queued for {next}: {running} holds the trigger's group"),
        format!("intake `serial`: {queued} started {next}"),
        format!(
            "intake `dropping`: {dropped} dropped: its group had no free slot: {holder} is running it"
        ),
        format!("intake `debounced`: {pending} closed over 2 arrivals and started {closed}"),
    ] {
        assert!(
            said.lines().any(|said| said == line),
            "the serve log says `{line}`:\n{said}"
        );
    }
}

/// **A `run` is refused by its trigger's `max_running:` where that is below
/// its group's `limit:`** (PRD resolved q77 ruling e, Decision D163's Status:
/// a key-less group covers a run, counting the trigger's total).
///
/// `held_capped`'s group has room for three, and the trigger for one. One
/// served execution fills the trigger, and a `run` of its flow is refused
/// naming it, `1 of 1 running`.
#[test]
fn a_run_is_refused_by_its_triggers_max_running_below_its_groups_limit() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-run-max-running");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-run-max-running") else {
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
        "/held-capped",
        &json!({ "tag": "served", "hold": hold_text }),
    ));
    eventually(Duration::from_secs(10), "the held execution's row", || {
        (executions_of(&project, "held_capped").len() == 1).then_some(())
    });
    let inputs = [("tag", "by hand"), ("hold", hold_text.as_str())];
    let refused = harness::run_into(
        &project,
        FIXTURE,
        "flow.held_too",
        &inputs,
        None,
        &environment,
    );
    assert_eq!(
        refused.output.status.code(),
        Some(2),
        "{}",
        refused.stderr()
    );
    assert!(
        refused.stderr().contains(&format!(
            "error: `flow.held_too` is covered by the `concurrency:` of the trigger `held_capped`, and its group has no free slot: {running} holds it (1 of 1 running)."
        )),
        "the refusal counts the trigger's total, not the group's limit: {}",
        refused.stderr()
    );
    assert!(
        executions_of(&project, "manual").is_empty(),
        "a refused run journals nothing"
    );
    std::fs::write(&hold, "").expect("the release file");
}

/// **A start owed and refused twice by the journal is started once it
/// recovers** (Decision D163's Status, `docs/durability.md` §3.15).
///
/// `serial`'s queued arrival is started as its slot frees, while the journal
/// refuses both that execution's lifecycle row and the move that would put the
/// row back on the queue — two SQLite triggers the test installs. The row is
/// left `started`, naming an execution that never began, which no drain of the
/// queue reads. The process remembers it owes the start: once the refusals are
/// lifted the row goes back on the queue and starts, in this same `serve`.
///
/// The refusals are lifted one at a time. With the move onto the queue taken
/// again and the lifecycle row still refused, each look at the queue puts the
/// owed row back and fails its start once more — and an arrival then finds
/// the group's slot free and is queued **behind** it all the same: the slot is
/// the older arrival's (FIFO per group, PRD resolved q77 ruling b), so the
/// newcomer starts only once the owed one has run.
#[test]
fn a_start_the_journal_refused_twice_is_owed_and_started_once_it_recovers() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-owed");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-owed") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    execution_of(&post(
        &app,
        "/serial",
        &json!({ "tag": "first", "linger": "2" }),
    ));
    let owed = intake_id_of(&post(
        &app,
        "/serial",
        &json!({ "tag": "owed", "linger": "0" }),
    ));
    let refuse = harness::journal_sql(
        &project,
        "CREATE TRIGGER refuse_the_owed_begin BEFORE INSERT ON executions \
           WHEN NEW.inputs LIKE '%owed%' BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;\n\
         CREATE TRIGGER refuse_the_requeue BEFORE UPDATE OF state ON intakes \
           WHEN NEW.state = 'queued' BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;\n",
    );
    assert!(
        refuse.status.success(),
        "{}",
        String::from_utf8_lossy(&refuse.stderr)
    );
    let stranded = || {
        let row = harness::journal_rows(
            &project,
            &format!(
                "SELECT intakes.state AS state, executions.id AS begun FROM intakes \
                 LEFT JOIN executions ON executions.id = intakes.execution \
                 WHERE intakes.id = '{owed}'"
            ),
        );
        row[0]["state"] == "started" && row[0]["begun"].is_null()
    };
    eventually(
        Duration::from_secs(20),
        "the owed row to be left started with no execution",
        || stranded().then_some(()),
    );
    // Held there across more than one re-drain: nothing reads it back while
    // the journal still refuses the move.
    std::thread::sleep(Duration::from_secs(3));
    assert!(
        stranded(),
        "the refusals hold the row where the failed start left it"
    );
    let half = harness::journal_sql(&project, "DROP TRIGGER refuse_the_requeue;\n");
    assert!(
        half.status.success(),
        "{}",
        String::from_utf8_lossy(&half.stderr)
    );
    let behind = post(&app, "/serial", &json!({ "tag": "behind", "linger": "0" }));
    assert_eq!(
        behind.json()["status"],
        "queued",
        "a free slot is the older arrival's, never a newcomer's: {}",
        behind.text()
    );
    let behind = intake_id_of(&behind);

    let lift = harness::journal_sql(&project, "DROP TRIGGER refuse_the_owed_begin;\n");
    assert!(
        lift.status.success(),
        "{}",
        String::from_utf8_lossy(&lift.stderr)
    );
    let report = eventually(
        Duration::from_secs(30),
        "the owed arrival to start once the journal takes it",
        || {
            let report = get(&app, &format!("/intakes/{owed}")).json();
            let started = report["execution_id"].as_str().map(str::to_string)?;
            executions_of(&project, "serial")
                .iter()
                .any(|row| row.id == started && row.status == "completed")
                .then_some(report)
        },
    );
    assert_eq!(report["state"], "started", "{report}");
    let rows = eventually(
        Duration::from_secs(30),
        "the arrival queued behind it to start and complete",
        || {
            let report = get(&app, &format!("/intakes/{behind}")).json();
            let started = report["execution_id"].as_str()?.to_string();
            let rows = executions_of(&project, "serial");
            rows.iter()
                .any(|row| row.id == started && row.status == "completed")
                .then_some(rows)
        },
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.inputs["tag"] == "owed")
            .count(),
        1,
        "started once: {rows:#?}"
    );
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag"))
        .collect();
    assert_eq!(
        order,
        ["first", "owed", "behind"],
        "the owed arrival ran before the one queued behind it"
    );
    never_overlapping(&rows);
    assert!(
        tally(&log).iter().any(|line| line == "owed end"),
        "{:?}",
        tally(&log)
    );
}

/// The composition [`an_empty_session_key_is_refused_before_a_dedupe_key_or_a_group_is_claimed`]
/// serves: one flow that writes a `scope: session` store, so no run of it
/// starts without a session identity, behind two `http` triggers whose
/// `session_key:` is the body's `project` — `deduped` with a `dedupe_key:` and
/// no block, and `grouped` with a `concurrency:` block and no key.
const SESSION_SCOPED: &str = r#"version: "0.1"

store.memory:
  kind: kv
  scope: session
  description: What each project was last started for.
  value_schema:
    id: { type: string }

flow.ingest:
  inputs:
    id: { type: string }
  outputs: {}
  nodes:
    note:
      store: store.memory
      op: set
      key: "execution.session_key"
      value:
        id: "input.id"
  edges:
    - { from: start, to: note }
    - { from: note, to: end }

triggers:
  deduped:
    type: http
    path: /deduped
    flow: flow.ingest
    session_key: "payload.body.project"
    input:
      id: "payload.body.id"
    dedupe_key: "payload.body.id"
  grouped:
    type: http
    path: /grouped
    flow: flow.ingest
    session_key: "payload.body.project"
    input:
      id: "payload.body.id"
    concurrency:
      limit: 1
"#;

/// **An empty `session_key:` a flow needs is refused before anything is
/// claimed — beside a `dedupe_key:` as beside a `concurrency:` block**
/// (grammar 11.3, 13.3, Decision D163's Status).
///
/// The run would refuse it before its lifecycle row, and an admission a gate
/// keeps is answered only once that row is down — so, asked no earlier, it
/// reads as the journal's failure: a `503` saying the request may be sent
/// again, and a key every resend takes over and fails the same way. So it is
/// the trigger's binding that failed, and both triggers answer `400`, twice
/// alike, naming the store — no dedupe row claimed, no intake row written,
/// nothing started. A request whose key answers starts, and its repeat is a
/// repeat.
#[test]
fn an_empty_session_key_is_refused_before_a_dedupe_key_or_a_group_is_claimed() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-empty-session");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(&entrypoint, SESSION_SCOPED).expect("the scratch area is writable");
    let Some(project) = harness::scratch_project("intake-empty-session") else {
        return;
    };
    let Some(served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let refusal = "`flow.ingest` reaches `store.memory`, which is `scope: session`, so this run \
                   needs a session identity and arrived with none: pass `--session <key>` to \
                   `agent-compose run`, or declare `session_key:` on the trigger that starts it \
                   — and where one is declared, it answered the empty string for this \
                   invocation (grammar 11.3, 13.2)";
    for (trigger, route) in [("deduped", "/deduped"), ("grouped", "/grouped")] {
        for attempt in 0..2 {
            let answered = post(&app, route, &json!({ "id": "d1", "project": "" }));
            assert_eq!(
                answered.status,
                400,
                "attempt {attempt} at `{trigger}`: {}",
                answered.text()
            );
            assert_eq!(
                answered.json(),
                json!({
                    "error": format!(
                        "the trigger `{trigger}` could not admit this request: {refusal}"
                    )
                }),
                "attempt {attempt} at `{trigger}`"
            );
            assert_eq!(deduped(&answered), None, "{answered:?}");
        }
    }
    assert!(
        dedupe_of(&project, "deduped").is_empty(),
        "no key was claimed"
    );
    assert!(
        intakes_of(&project, "grouped").is_empty(),
        "no intake row was written"
    );
    assert!(
        executions_of(&project, "deduped").is_empty()
            && executions_of(&project, "grouped").is_empty(),
        "nothing started"
    );

    let first = execution_of(&post(
        &app,
        "/deduped",
        &json!({ "id": "d1", "project": "alpha" }),
    ));
    eventually(
        Duration::from_secs(30),
        "the keyed request to complete",
        || {
            executions_of(&project, "deduped")
                .iter()
                .any(|row| row.id == first && row.status == "completed")
                .then_some(())
        },
    );
    let repeat = post(&app, "/deduped", &json!({ "id": "d1", "project": "alpha" }));
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(execution_of(&repeat), first);
    assert_eq!(
        dedupe_of(&project, "deduped"),
        BTreeMap::from([("d1".to_string(), first.clone())]),
        "the key names the one execution"
    );
    assert_eq!(executions_of(&project, "deduped").len(), 1);
}

/// The composition [`a_queued_arrival_the_new_build_no_longer_binds_is_dropped_and_the_queue_moves_on`]
/// serves, under the schema of the flow's `tag` it is built with: one keyed
/// group of one over a flow whose execution ends when the file its `hold`
/// names appears.
fn rebuilt(tag: &str) -> String {
    format!(
        r#"version: "0.1"

triggers:
  serial:
    type: http
    path: /serial
    flow: flow.held
    input:
      tag: "payload.body.tag"
      hold: "payload.body.hold"
    dedupe_key: "payload.body.id"
    concurrency:
      limit: 1

flow.held:
  description: One tagged piece of work that ends when a file appears.
  inputs:
    tag: {tag}
    hold: {{ type: string }}
  outputs: {{}}
  nodes:
    wait:
      exec:
        command: "${{TALLY_BIN}}/hold"
      input:
        tag: "input.tag"
        hold: "input.hold"
  edges:
    - {{ from: start, to: wait }}
    - {{ from: wait, to: end }}
"#
    )
}

/// **A queued arrival the new build no longer binds is dropped, naming why,
/// and the queue moves on past it** (PRD resolved q77 ruling f, Decision
/// D163's Status, `docs/durability.md` §3.15).
///
/// A queued row holds the admission the build that took it bound, and is
/// started by whichever build is serving when its slot frees. Here `serial`
/// holds `first` and queues `second` and `third`; `serve` is killed and
/// started again over the same journal under a build whose flow narrows `tag`
/// to `first`, `third` or `fourth`. `fourth` arrives and queues behind them.
/// When `first` ends, `second`'s admission no longer fits the flow: it is
/// dropped — `GET /intakes/:id` says so, with the reason `serve` gives the same
/// request sent now — and logged, never tried again as a start the journal
/// would not begin; `third` starts in the slot, and `fourth` after it. A repeat
/// of `second`'s key is answered with its intake, which started nothing.
#[test]
fn a_queued_arrival_the_new_build_no_longer_binds_is_dropped_and_the_queue_moves_on() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-rebuilt");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let release = scratch.path().join("release-first");
    let released = scratch.path().join("released");
    std::fs::write(&released, "").expect("the scratch area is writable");
    let hold = |file: &Path| file.display().to_string();
    let before = scratch.path().join("before");
    let after = scratch.path().join("after");
    for (directory, tag) in [
        (&before, "{ type: string }".to_string()),
        (
            &after,
            r#"{ type: string, pattern: "^(first|third|fourth)$" }"#.to_string(),
        ),
    ] {
        std::fs::create_dir_all(directory).expect("the scratch area is writable");
        std::fs::write(directory.join("main.yml"), rebuilt(&tag))
            .expect("the scratch area is writable");
    }
    let Some(project) = harness::scratch_project("intake-rebuilt") else {
        return;
    };
    let (first, second, third) = {
        let Some(served) =
            harness::serve_entrypoint_into(&project, &before.join("main.yml"), &environment)
        else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let first = execution_of(&post(
            &app,
            "/serial",
            &json!({ "id": "first", "tag": "first", "hold": hold(&release) }),
        ));
        let queued: Vec<String> = ["second", "third"]
            .iter()
            .map(|tag| {
                let answered = post(
                    &app,
                    "/serial",
                    &json!({ "id": tag, "tag": tag, "hold": hold(&released) }),
                );
                assert_eq!(answered.json()["status"], "queued", "{}", answered.text());
                intake_id_of(&answered)
            })
            .collect();
        (first, queued[0].clone(), queued[1].clone())
    };

    let Some(mut served) =
        harness::serve_entrypoint_into(&project, &after.join("main.yml"), &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    // What this build answers `second`'s request with, sent now.
    let refused = post(
        &app,
        "/serial",
        &json!({ "id": "second-again", "tag": "second", "hold": hold(&released) }),
    );
    assert_eq!(refused.status, 400, "{}", refused.text());
    let reason = refused.json()["error"]
        .as_str()
        .and_then(|error| error.strip_prefix("the request does not fit "))
        .unwrap_or_else(|| panic!("a refusal naming the flow's inputs: {}", refused.text()))
        .to_string();
    let fourth = post(
        &app,
        "/serial",
        &json!({ "id": "fourth", "tag": "fourth", "hold": hold(&released) }),
    );
    assert_eq!(fourth.json()["status"], "queued", "{}", fourth.text());
    let fourth = intake_id_of(&fourth);

    std::fs::write(&release, "").expect("the release file");
    let rows = eventually(
        Duration::from_secs(60),
        "first, third and fourth to complete",
        || {
            let rows = executions_of(&project, "serial");
            (rows.len() == 3 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag"))
        .collect();
    assert_eq!(
        order,
        ["first", "third", "fourth"],
        "the queue moved on past the row it dropped, in its order"
    );
    assert_eq!(rows[0].id, first);
    for (id, expected) in [(&third, &rows[1]), (&fourth, &rows[2])] {
        let report = get(&app, &format!("/intakes/{id}")).json();
        assert_eq!(report["state"], "started", "{report}");
        assert_eq!(report["execution_id"], expected.id.as_str(), "{report}");
    }
    let detail = format!("its admission no longer binds under this build: {reason}");
    let report = get(&app, &format!("/intakes/{second}")).json();
    assert_eq!(report["state"], "dropped", "{report}");
    assert_eq!(report["detail"], detail.as_str(), "{report}");
    assert!(
        report.get("execution_id").is_none() && report.get("running_execution_id").is_none(),
        "a row dropped for its binding names no execution: {report}"
    );

    // A repeat of the dropped arrival's key, in a request this build binds,
    // is a repeat: answered with the intake that started nothing.
    let repeat = post(
        &app,
        "/serial",
        &json!({ "id": "second", "tag": "fourth", "hold": hold(&released) }),
    );
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(repeat.status, 202, "{}", repeat.text());
    assert_eq!(
        repeat.json(),
        json!({
            "intake_id": second,
            "status": "dropped",
            "status_url": format!("/intakes/{second}"),
        })
    );
    std::thread::sleep(Duration::from_secs(3));
    assert_eq!(
        executions_of(&project, "serial").len(),
        3,
        "nothing more started"
    );

    let said = served.stop_and_read_stderr();
    let dropped = format!("intake `serial`: {second} dropped: {detail}");
    assert_eq!(
        said.lines().filter(|line| *line == dropped).count(),
        1,
        "the drop is logged once: `{dropped}`\n{said}"
    );
    assert!(
        !said.contains("could not be journaled"),
        "the row was never tried as a start the journal would not begin: {said}"
    );
}
