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
//!    [`a_restart_with_a_debounce_pending_fires_it_once`]; mid-start, between
//!    a slot freeing and the queued arrival's lifecycle row —
//!    [`a_start_a_killed_serve_claimed_and_never_journaled_starts_once_as_the_lease_allows`];
//!    mid-debounce with a first look the journal cuts short —
//!    [`a_debounce_whose_fold_the_journal_refuses_at_a_restart_is_still_armed_and_fires`];
//!    and onto a build that removed the blocks —
//!    [`queued_and_pending_arrivals_a_deploy_left_without_a_block_start_with_no_bound`];
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
//!   [`deliveries_racing_for_one_slot_start_one_and_queue_or_drop_the_rest`],
//!   and with each lifecycle row landing late, so that only a start's wait for
//!   its row keeps the next decision from reading the slot free —
//!   [`a_start_holds_its_triggers_decisions_until_its_lifecycle_row_is_down`];
//! * an arrival that finds its debounce past the deadline —
//!   [`an_arrival_past_its_debounces_deadline_closes_it_and_opens_its_own`];
//! * `supersede` replacing its group's queued arrivals —
//!   [`a_superseding_arrival_drops_its_groups_queued_arrivals`] — and starting
//!   at its predecessor's cancel, while that execution is still unwinding —
//!   [`a_superseding_arrival_starts_while_the_execution_it_cancelled_unwinds`];
//! * what a burst behind a full group costs each arrival —
//!   [`arrivals_behind_a_full_group_never_read_its_queue`];
//! * an empty `session_key:` a flow needs, refused before a dedupe key is
//!   claimed as before a group is asked —
//!   [`an_empty_session_key_is_refused_before_a_dedupe_key_or_a_group_is_claimed`];
//! * a queued arrival a new build no longer binds, dropped while the queue
//!   moves on past it —
//!   [`a_queued_arrival_the_new_build_no_longer_binds_is_dropped_and_the_queue_moves_on`];
//! * a start the journal refused, owed and holding its slot, and the arrivals
//!   queued behind it —
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
/// one is the drop** — the execution it was dropped for, its own intake id and
/// `X-AgentCompose-Dropped: true` beside `X-AgentCompose-Deduped: true`
/// (grammar 13.3's answer table, Decision D163's Status) — a key beside a
/// group that queues, and one that drops.
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
        repeat.header("x-agentcompose-dropped"),
        Some("true"),
        "a repeat of a dropped arrival is the drop: {repeat:?}"
    );
    assert_eq!(
        repeat.json(),
        dropped.json(),
        "the first delivery's body, its intake id with it"
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
/// a repeat of a dropped one is the drop again — its headers, its body, at
/// once — and never the running execution's answer, which would be another
/// payload's outputs; a repeat of a started one waits on the first's execution and is answered
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
                    &json!({ "tag": "d1", "id": "d1", "linger": "3" }),
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
            &json!({ "tag": "d2", "id": "d2", "linger": "0" }),
        );
        assert_eq!(
            dropped.header("x-agentcompose-dropped"),
            Some("true"),
            "{dropped:?}"
        );
        assert_eq!(execution_of(&dropped), holder);
        let dropped_intake = intake_id_of(&dropped);
        assert_eq!(
            dropped.json(),
            json!({
                "execution_id": holder,
                "status_url": format!("/executions/{holder}"),
                "intake_id": dropped_intake,
            })
        );

        // A repeat of the dropped arrival, while the run it was dropped for
        // still lingers: the drop again, at once — never a wait on that run,
        // whose outputs are another payload's (grammar 13.3's table).
        let repeat = post(
            &app,
            "/sync-dropping",
            &json!({ "tag": "d2", "id": "d2", "linger": "0" }),
        );
        assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
        assert_eq!(
            repeat.header("x-agentcompose-dropped"),
            Some("true"),
            "a repeat of a dropped arrival is the drop: {repeat:?}"
        );
        assert_eq!(repeat.status, 202, "{}", repeat.text());
        assert_eq!(repeat.json(), dropped.json());
        assert_eq!(
            executions_of(&project, "sync_dropping")
                .iter()
                .map(|row| row.status.as_str())
                .collect::<Vec<_>>(),
            ["open"],
            "the repeat was answered while the run it names was still open"
        );

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
        // …and once that run has settled, still the drop: its outputs are
        // `d1`'s, and `d2` never ran.
        let repeat = post(
            &app,
            "/sync-dropping",
            &json!({ "tag": "d2", "id": "d2", "linger": "0" }),
        );
        assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
        assert_eq!(
            repeat.header("x-agentcompose-dropped"),
            Some("true"),
            "{repeat:?}"
        );
        assert_eq!(repeat.status, 202, "{}", repeat.text());
        assert_eq!(repeat.json(), dropped.json());
        assert_eq!(executions_of(&project, "sync_dropping").len(), 1);

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

/// `count` deliveries to `route`, released together from as many threads —
/// `racer <n>`, each lingering `linger` seconds — and what each was answered.
fn race(app: &Client, route: &'static str, count: usize, linger: &'static str) -> Vec<Response> {
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

    let answers = race(&app, "/serial", 8, "1");
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

    let answers = race(&app, "/dropping", 6, "3");
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
/// **The owed start holds its slot** meanwhile: `next`, queued behind it, is
/// never started into the slot the owed arrival was answered for, however many
/// looks at the queue pass over it while the move is refused — it runs after
/// the owed one, so the group never runs the two at once.
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
    let next = intake_id_of(&post(
        &app,
        "/serial",
        &json!({ "tag": "next", "linger": "0" }),
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
    let rows = executions_of(&project, "serial");
    assert!(
        rows.iter().all(|row| row.inputs["tag"] != "next"),
        "the arrival queued behind the owed start never took its slot: {rows:#?}"
    );
    assert_eq!(
        get(&app, &format!("/intakes/{next}")).json()["state"],
        "queued"
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
        ["first", "owed", "next", "behind"],
        "the owed arrival ran before the ones queued behind it"
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

/// The execution an intake row names, read off the journal.
fn execution_named_by(project: &Path, intake: &str) -> String {
    let rows = harness::journal_rows(
        project,
        &format!("SELECT execution FROM intakes WHERE id = '{intake}'"),
    );
    rows[0]["execution"]
        .as_str()
        .unwrap_or_else(|| panic!("the row {intake} names an execution: {rows}"))
        .to_string()
}

/// **A start a killed `serve` claimed and never journaled starts once, under
/// the id its row recorded, as the lease allows** (PRD resolved q77 ruling f,
/// Decision D163's Status, `docs/durability.md` §3.15, §6.3).
///
/// Two groups of one, each holding one execution and one queued arrival, and
/// `serve` killed. The journal is then left as a `SIGKILL` between a slot
/// freeing and the queued arrival's start leaves it: the queued row says
/// `started`, naming an execution that has no lifecycle row.
///
/// * Under `held`, the execution that held the slot had settled first, so its
///   slot is free: the restart starts the claimed arrival at once, under the
///   execution id its row recorded, and once.
/// * Under `keyed`, the execution holding the group is still open — the
///   restart recovers it, and it holds the group's one slot. The claimed
///   arrival is **not** started beside it: it goes back on the queue at its
///   place and starts when the recovered execution settles, so the group never
///   runs two at once.
#[test]
fn a_start_a_killed_serve_claimed_and_never_journaled_starts_once_as_the_lease_allows() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-claimed-start");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-claimed-start") else {
        return;
    };
    let file = |name: &str| scratch.path().join(name).display().to_string();
    let (held_first, held_intake, keyed_first, keyed_intake) = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let held_first = execution_of(&post(
            &app,
            "/held",
            &json!({ "tag": "held first", "hold": file("held-first") }),
        ));
        let held_intake = intake_id_of(&post(
            &app,
            "/held",
            &json!({ "tag": "held claimed", "hold": file("held-claimed") }),
        ));
        let keyed_first = execution_of(&post(
            &app,
            "/keyed",
            &json!({ "tag": "keyed first", "group": "g", "hold": file("keyed-first") }),
        ));
        let keyed_intake = intake_id_of(&post(
            &app,
            "/keyed",
            &json!({ "tag": "keyed claimed", "group": "g", "hold": file("keyed-claimed") }),
        ));
        eventually(
            Duration::from_secs(10),
            "both held executions' rows",
            || {
                (executions_of(&project, "held").len() == 1
                    && executions_of(&project, "keyed").len() == 1)
                    .then_some(())
            },
        );
        (held_first, held_intake, keyed_first, keyed_intake)
        // Dropped here: `SIGKILL` to the whole group.
    };
    let held_claimed = execution_named_by(&project, &held_intake);
    let keyed_claimed = execution_named_by(&project, &keyed_intake);
    let now = "strftime('%Y-%m-%dT%H:%M:%fZ', 'now')";
    let surgery = harness::journal_sql(
        &project,
        &format!(
            "UPDATE executions SET status = 'completed', ended_at = {now} WHERE id = '{held_first}';\n\
             UPDATE intakes SET state = 'started', started_at = {now} \
               WHERE id IN ('{held_intake}', '{keyed_intake}');\n"
        ),
    );
    assert!(
        surgery.status.success(),
        "{}",
        String::from_utf8_lossy(&surgery.stderr)
    );
    let unbegun = |intake: &str| {
        let row = harness::journal_rows(
            &project,
            &format!(
                "SELECT intakes.state AS state, executions.id AS begun FROM intakes \
                 LEFT JOIN executions ON executions.id = intakes.execution \
                 WHERE intakes.id = '{intake}'"
            ),
        );
        row[0]["state"] == "started" && row[0]["begun"].is_null()
    };
    assert!(
        unbegun(&held_intake) && unbegun(&keyed_intake),
        "both rows claim a start no lifecycle row followed"
    );

    let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");

    // `held`'s slot was free: started at once, under the id the row recorded.
    let report = eventually(
        Duration::from_secs(20),
        "the claimed start of `held` to be made",
        || {
            let report = get(&app, &format!("/intakes/{held_intake}")).json();
            executions_of(&project, "held")
                .iter()
                .any(|row| row.id == held_claimed)
                .then_some(report)
        },
    );
    assert_eq!(report["state"], "started", "{report}");
    assert_eq!(report["execution_id"], held_claimed.as_str(), "{report}");

    // `keyed`'s one slot is held by the recovered execution: the claimed
    // arrival waits on the queue, and never runs beside it.
    eventually(
        Duration::from_secs(20),
        "the claimed start of `keyed` to go back on the queue",
        || {
            (get(&app, &format!("/intakes/{keyed_intake}")).json()["state"] == "queued")
                .then_some(())
        },
    );
    std::thread::sleep(Duration::from_secs(3));
    let rows = executions_of(&project, "keyed");
    assert_eq!(
        rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
        [keyed_first.as_str()],
        "the claimed arrival did not start beside the execution holding its group: {rows:#?}"
    );
    assert_eq!(rows[0].status, "open", "{rows:#?}");

    std::fs::write(scratch.path().join("keyed-first"), "").expect("the release file");
    let report = eventually(
        Duration::from_secs(30),
        "the claimed start of `keyed` to be made once its slot frees",
        || {
            let report = get(&app, &format!("/intakes/{keyed_intake}")).json();
            (report["state"] == "started").then_some(report)
        },
    );
    assert_eq!(report["execution_id"], keyed_claimed.as_str(), "{report}");
    for name in ["held-claimed", "keyed-claimed"] {
        std::fs::write(scratch.path().join(name), "").expect("the release file");
    }
    for (trigger, first, claimed) in [
        ("held", &held_first, &held_claimed),
        ("keyed", &keyed_first, &keyed_claimed),
    ] {
        let rows = eventually(
            Duration::from_secs(30),
            "both of a group's executions to complete",
            || {
                let rows = executions_of(&project, trigger);
                (rows.len() == 2 && rows.iter().all(|row| row.status == "completed"))
                    .then_some(rows)
            },
        );
        never_overlapping(&rows);
        assert_eq!(
            rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>(),
            [first.as_str(), claimed.as_str()],
            "the claimed arrival started once, under the id its row recorded"
        );
    }
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        executions_of(&project, "held").len(),
        2,
        "none started twice"
    );
    assert_eq!(
        executions_of(&project, "keyed").len(),
        2,
        "none started twice"
    );

    let said = served.stop_and_read_stderr();
    let repaired = format!(
        "intake `held`: {held_intake} started {held_claimed}, whose start a stopped serve had claimed and not journaled"
    );
    assert!(
        said.lines().any(|line| line == repaired),
        "`{repaired}`\n{said}"
    );
    let requeued = format!("intake `keyed`: {keyed_intake} is back on the queue: ");
    assert!(
        said.lines().any(|line| line.starts_with(&requeued)
            && line.contains("its group had no free slot")
            && line.contains(&keyed_first)),
        "`{requeued}…`\n{said}"
    );
}

/// **A pending debounce is armed after a restart even where the journal
/// refuses its first look's fold** (PRD resolved q77 ruling f, Decision D163's
/// Status, `docs/durability.md` §6.3).
///
/// `burst` counts two deliveries into one ten-second debounce; `serve` is
/// killed and the second's dedupe key put back on the id it was claimed under,
/// as a crash between the count and the fold leaves it. The journal then
/// refuses every update of a dedupe row — the fold the restart's look makes
/// first — until the test lifts the refusal. The look that could not fold
/// still arms the debounce: it closes once its deadline passes and the
/// journal takes the fold, starting one execution over both arrivals, and the
/// key the look could not move names that execution in the end.
#[test]
fn a_debounce_whose_fold_the_journal_refuses_at_a_restart_is_still_armed_and_fires() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-unfolded-restart");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-unfolded-restart") else {
        return;
    };
    let intake = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let intake = intake_id_of(&post(
            &app,
            "/burst",
            &json!({ "tag": "first", "id": "u1" }),
        ));
        let second = post(&app, "/burst", &json!({ "tag": "second", "id": "u2" }));
        assert_eq!(second.json()["coalesced"], 2, "{}", second.text());
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
    let surgery = harness::journal_sql(
        &project,
        &format!(
            "UPDATE dedupe SET execution = '{newest}' \
               WHERE trigger_name = 'burst' AND dedupe_key = '\"u2\"';\n\
             CREATE TRIGGER refuse_the_fold BEFORE UPDATE ON dedupe \
               BEGIN SELECT RAISE(ABORT, 'refused by the test'); END;\n"
        ),
    );
    assert!(
        surgery.status.success(),
        "{}",
        String::from_utf8_lossy(&surgery.stderr)
    );

    let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    // The look has been made, and could not fold.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        dedupe_of(&project, "burst").get("u2"),
        Some(&newest),
        "the refusal held the key where the crash left it"
    );
    let lift = harness::journal_sql(&project, "DROP TRIGGER refuse_the_fold;\n");
    assert!(
        lift.status.success(),
        "{}",
        String::from_utf8_lossy(&lift.stderr)
    );

    let rows = eventually(
        Duration::from_secs(40),
        "the debounce to close and its execution to complete",
        || {
            let rows = executions_of(&project, "burst");
            (!rows.is_empty() && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    assert_eq!(rows.len(), 1, "one execution: {rows:#?}");
    assert_eq!(rows[0].id, own, "the row's own execution: {rows:#?}");
    assert_eq!(rows[0].inputs["pushes"], 2, "both arrivals: {rows:#?}");
    assert_eq!(
        rows[0].inputs["tag"], "second",
        "the newest payload: {rows:#?}"
    );
    assert_eq!(tally(&log), ["second start 2", "second end"]);
    let report = get(&app, &format!("/intakes/{intake}")).json();
    assert_eq!(report["state"], "started", "{report}");
    assert_eq!(
        dedupe_of(&project, "burst").get("u2"),
        Some(&own),
        "the close folded the key the look could not"
    );

    let said = served.stop_and_read_stderr();
    let unfolded = format!(
        "intake `burst`: {intake}'s newest arrival's gate could not yet be moved onto {own} ("
    );
    assert!(
        said.lines().any(|line| line.starts_with(&unfolded)),
        "`{unfolded}…`\n{said}"
    );
}

/// The composition [`queued_and_pending_arrivals_a_deploy_left_without_a_block_start_with_no_bound`]
/// serves: a group of one that queues and a debounce, each over a flow whose
/// execution ends when the file its `hold` names appears — with their
/// `concurrency:` blocks, or without them, as a deploy that removed them
/// leaves it.
fn unblocked(blocks: bool) -> String {
    let (serial, burst) = if blocks {
        (
            "\n    concurrency:\n      limit: 1",
            "\n    concurrency:\n      debounce: 4s",
        )
    } else {
        ("", "")
    };
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
    dedupe_key: "payload.body.id"{serial}

  burst:
    type: http
    path: /burst
    flow: flow.held
    input:
      tag: "payload.body.tag"
      hold: "payload.body.hold"{burst}

flow.held:
  description: One tagged piece of work that ends when a file appears.
  inputs:
    tag: {{ type: string }}
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

/// **Queued and pending arrivals a deploy left without a `concurrency:`
/// block start with no bound** (PRD resolved q77 ruling f, Decision D163's
/// Status, `docs/durability.md` §6.3).
///
/// `serial` holds `first` and queues `second` and `third`; `burst` has a
/// debounce open. `serve` is killed and started again under a build that
/// declares neither block. The arrivals were accepted and answered with their
/// intake ids, so they are owed a start; the serving build bounds the trigger
/// by nothing, so `second` and `third` start at once beside the recovered
/// `first`, and the debounce closes at the deadline its row records — each
/// under the id its row recorded, once — and a repeat of `second`'s key is
/// answered with the execution it started.
#[test]
fn queued_and_pending_arrivals_a_deploy_left_without_a_block_start_with_no_bound() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-unblocked");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let release = scratch.path().join("release-first");
    let released = scratch.path().join("released");
    std::fs::write(&released, "").expect("the scratch area is writable");
    let hold = |file: &Path| file.display().to_string();
    let before = scratch.path().join("before");
    let after = scratch.path().join("after");
    for (directory, blocks) in [(&before, true), (&after, false)] {
        std::fs::create_dir_all(directory).expect("the scratch area is writable");
        std::fs::write(directory.join("main.yml"), unblocked(blocks))
            .expect("the scratch area is writable");
    }
    let Some(project) = harness::scratch_project("intake-unblocked") else {
        return;
    };
    let (first, queued, pending) = {
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
        let pending = post(
            &app,
            "/burst",
            &json!({ "tag": "burst", "hold": hold(&released) }),
        );
        assert_eq!(pending.json()["status"], "pending", "{}", pending.text());
        (first, queued, intake_id_of(&pending))
        // Dropped here: `SIGKILL` to the whole group.
    };
    let claimed: Vec<String> = queued
        .iter()
        .map(|intake| execution_named_by(&project, intake))
        .collect();
    let burst = execution_named_by(&project, &pending);

    let Some(mut served) =
        harness::serve_entrypoint_into(&project, &after.join("main.yml"), &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let rows = eventually(
        Duration::from_secs(30),
        "the queued arrivals to complete beside the recovered first",
        || {
            let rows = executions_of(&project, "serial");
            (rows.len() == 3
                && rows
                    .iter()
                    .filter(|row| row.id != first)
                    .all(|row| row.status == "completed"))
            .then_some(rows)
        },
    );
    assert_eq!(
        rows.iter()
            .find(|row| row.id == first)
            .map(|row| row.status.as_str()),
        Some("open"),
        "the queued arrivals ran while first still held what was its group: {rows:#?}"
    );
    for (intake, execution) in queued.iter().zip(&claimed) {
        let report = get(&app, &format!("/intakes/{intake}")).json();
        assert_eq!(report["state"], "started", "{report}");
        assert_eq!(report["execution_id"], execution.as_str(), "{report}");
    }
    let repeat = post(
        &app,
        "/serial",
        &json!({ "id": "second", "tag": "second", "hold": hold(&released) }),
    );
    assert_eq!(deduped(&repeat), Some("true"), "{repeat:?}");
    assert_eq!(execution_of(&repeat), claimed[0]);

    let rows = eventually(
        Duration::from_secs(30),
        "the debounce to close and its execution to complete",
        || {
            let rows = executions_of(&project, "burst");
            (rows.len() == 1 && rows[0].status == "completed").then_some(rows)
        },
    );
    assert_eq!(rows[0].id, burst, "the id its row recorded: {rows:#?}");
    let report = get(&app, &format!("/intakes/{pending}")).json();
    assert_eq!(report["state"], "started", "{report}");

    std::fs::write(&release, "").expect("the release file");
    eventually(Duration::from_secs(30), "first to complete", || {
        executions_of(&project, "serial")
            .iter()
            .any(|row| row.id == first && row.status == "completed")
            .then_some(())
    });
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        executions_of(&project, "serial").len(),
        3,
        "none started twice"
    );
    assert_eq!(
        executions_of(&project, "burst").len(),
        1,
        "none started twice"
    );

    let said = served.stop_and_read_stderr();
    for trigger in ["serial", "burst"] {
        let line = format!(
            "intake `{trigger}`: this build declares no `concurrency:` on the trigger, so the arrivals an earlier build's intake accepted for it start with no bound"
        );
        assert_eq!(
            said.lines().filter(|said| *said == line).count(),
            1,
            "`{line}`\n{said}"
        );
    }
}

/// The host module [`a_start_holds_its_triggers_decisions_until_its_lifecycle_row_is_down`]
/// serves through: the journal the app is about to open, with each of
/// `serial`'s lifecycle rows begun `LIFECYCLE_DELAY_MS` late — the row joins
/// the journal's queue only after that wait, as a slower backend or run path
/// would land it — and then the project's own `serve`.
const LATE_LIFECYCLE_ROWS: &str = r#"import process from "node:process";
import { openJournal } from "./src/journal.ts";

await import("./src/index.ts");
const journal = await openJournal();
const begin = journal.begin.bind(journal);
const late = Number(process.env.LIFECYCLE_DELAY_MS ?? "0");
journal.begin = async (row) => {
  if (row.trigger === "serial") await new Promise((resolve) => setTimeout(resolve, late));
  return begin(row);
};
const { runMain } = await import("./src/cli.ts");
await runMain(process.argv.slice(2));
"#;

/// **A start holds its trigger's decisions until its lifecycle row is down**
/// (Decision D163's Status: each start waits for its lifecycle row before the
/// next decision reads the lease, so two arrivals never both read one slot
/// free).
///
/// The lease is the journal's rows, so a start whose lifecycle row has not yet
/// landed holds no slot the next decision could read. On SQLite the row lands
/// ahead of that read whether or not the start waits for it — the journal's one
/// queue orders the two — so here the app's journal begins each of `serial`'s
/// lifecycle rows a second and a half late. Four deliveries released together
/// still start exactly one: every other decision waited behind that start for
/// its row, read the one slot held, and queued; and the four ran one at a
/// time.
#[test]
fn a_start_holds_its_triggers_decisions_until_its_lifecycle_row_is_down() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-late-row");
    let (environment, _log) = environment(&provider, &scratch, &[("LIFECYCLE_DELAY_MS", "1500")]);
    let Some((project, built)) =
        harness::build_entrypoint(&harness::fixture(FIXTURE), "intake-late-row")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let Some(served) = harness::serve_built_hosted(&project, LATE_LIFECYCLE_ROWS, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");

    let answers = race(&app, "/serial", 4, "1");
    let started: Vec<&Response> = answers
        .iter()
        .filter(|answered| answered.json().get("intake_id").is_none())
        .collect();
    assert_eq!(
        started.len(),
        1,
        "one racer took the slot, however late its row landed: {answers:#?}"
    );
    assert!(
        answers
            .iter()
            .filter(|answered| answered.json().get("intake_id").is_some())
            .all(|answered| answered.status == 202 && answered.json()["status"] == "queued"),
        "every other racer read the slot held and queued: {answers:#?}"
    );
    let rows = eventually(
        Duration::from_secs(60),
        "four executions to complete",
        || {
            let rows = executions_of(&project, "serial");
            (rows.len() == 4 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    assert_eq!(rows[0].id, execution_of(started[0]), "the winner ran first");
    assert!(
        intakes_of(&project, "serial")
            .iter()
            .all(|row| row["state"] == "started"),
        "each racer started once"
    );
}

/// The host module [`an_arrival_past_its_debounces_deadline_closes_it_and_opens_its_own`]
/// serves through: the app's journal, refusing once the read of `debounced`'s
/// unbegun starts — the first step of the look a start takes at a trigger's
/// rows — as a journal a standby cannot yet read refuses it, so the look is
/// made again by the trigger's first arrival.
const FIRST_LOOK_REFUSED: &str = r#"import process from "node:process";
import { openJournal } from "./src/journal.ts";

await import("./src/index.ts");
const journal = await openJournal();
const unbegun = journal.unbegunIntakes.bind(journal);
let refused = false;
journal.unbegunIntakes = async (trigger) => {
  if (trigger === "debounced" && !refused) {
    refused = true;
    throw new Error("refused by the test");
  }
  return unbegun(trigger);
};
const { runMain } = await import("./src/cli.ts");
await runMain(process.argv.slice(2));
"#;

/// **An arrival that finds its debounce past the deadline closes it and opens
/// its own** (PRD resolved q77 rulings c and f, Decision D163's Status).
///
/// Two deliveries open `debounced`'s three-second debounce; `serve` is killed
/// and started again once the deadline has passed, with the start's look at the
/// trigger's rows refused once — so the row is still open, its close not yet
/// run, when a third delivery arrives and makes the look itself. The row starts
/// at the deadline it recorded, over the two arrivals it counted and with the
/// second's payload; the third is never counted into it nor moves its
/// deadline, and is answered as the first arrival of a debounce of its own — a
/// new intake, `coalesced: 1` — which starts three seconds on.
#[test]
fn an_arrival_past_its_debounces_deadline_closes_it_and_opens_its_own() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-overdue");
    let (environment, log) = environment(&provider, &scratch, &[]);
    let Some(project) = harness::scratch_project("intake-overdue") else {
        return;
    };
    let first = {
        let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client");
        let first = intake_id_of(&post(&app, "/debounced", &json!({ "tag": "first" })));
        let second = post(&app, "/debounced", &json!({ "tag": "second" }));
        assert_eq!(
            second.json()["intake_id"],
            first.as_str(),
            "{}",
            second.text()
        );
        assert_eq!(second.json()["coalesced"], 2, "{}", second.text());
        first
        // Dropped here, inside the debounce: `SIGKILL` to the whole group.
    };
    // Past the deadline the row records, with nothing running to close it.
    std::thread::sleep(Duration::from_secs(4));
    assert_eq!(pending_of(&project, "debounced")["id"], first.as_str());

    let Some(mut served) = harness::serve_built_hosted(&project, FIRST_LOOK_REFUSED, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let third = post(&app, "/debounced", &json!({ "tag": "third" }));
    assert_eq!(third.status, 202, "{}", third.text());
    assert_eq!(third.json()["status"], "pending", "{}", third.text());
    assert_eq!(
        third.json()["coalesced"],
        1,
        "the third opened a debounce of its own: {}",
        third.text()
    );
    let own = intake_id_of(&third);
    assert_ne!(own, first, "never counted into a row past its deadline");

    let rows = eventually(
        Duration::from_secs(40),
        "both debounces to start and complete",
        || {
            let rows = executions_of(&project, "debounced");
            (rows.len() == 2 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    assert_eq!(rows[0].inputs["tag"], "second", "{rows:#?}");
    assert_eq!(rows[0].inputs["pushes"], 2, "the two it counted: {rows:#?}");
    assert_eq!(rows[1].inputs["tag"], "third", "{rows:#?}");
    assert_eq!(rows[1].inputs["pushes"], 1, "{rows:#?}");
    for (id, row) in [(&first, &rows[0]), (&own, &rows[1])] {
        let report = get(&app, &format!("/intakes/{id}")).json();
        assert_eq!(report["state"], "started", "{report}");
        assert_eq!(report["execution_id"], row.id.as_str(), "{report}");
    }
    assert_eq!(
        tally(&log),
        ["second start 2", "second end", "third start 1", "third end"]
    );
    let said = served.stop_and_read_stderr();
    assert!(
        said.contains(
            "intake `debounced`: its intake rows could not be read (refused by the test)"
        ),
        "the start's look was the one refused: {said}"
    );
}

/// The composition [`a_superseding_arrival_drops_its_groups_queued_arrivals`]
/// serves: a group per body `group`, superseding, and the trigger's own
/// `max_running: 1` across them, over a flow whose execution ends when the
/// file its `hold` names appears.
const SUPERSEDING: &str = r#"version: "0.1"

triggers:
  latest:
    type: http
    path: /latest
    flow: flow.held
    input:
      tag: "payload.body.tag"
      hold: "payload.body.hold"
    concurrency:
      key: "payload.body.group"
      on_conflict: supersede
      max_running: 1

flow.held:
  description: One tagged piece of work that ends when a file appears.
  inputs:
    tag: { type: string }
    hold: { type: string }
  outputs: {}
  nodes:
    wait:
      exec:
        command: "${TALLY_BIN}/hold"
      input:
        tag: "input.tag"
        hold: "input.hold"
  edges:
    - { from: start, to: wait }
    - { from: wait, to: end }
"#;

/// **A superseding arrival drops its group's queued arrivals** (PRD resolved
/// q77 ruling b, Decision D163's Status): only the newest arrival of a group
/// is owed a run, so an older one still waiting is never started ahead of it.
///
/// `latest` supersedes within a group and runs one execution across all of
/// them. `a1` holds the trigger's one slot; `b1` arrives for another group and
/// queues behind `max_running:`, which a supersede never reaches past its own
/// group; `b2` arrives for `b1`'s group and drops `b1` — `GET /intakes/:id`
/// says so, naming the execution `b2` will be, and the serve log does too —
/// and queues in its place. Once `a1` ends, `b2` runs; `b1` never does.
#[test]
fn a_superseding_arrival_drops_its_groups_queued_arrivals() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-superseded-queue");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let release = scratch.path().join("release-a1");
    let released = scratch.path().join("released");
    std::fs::write(&released, "").expect("the scratch area is writable");
    let hold = |file: &Path| file.display().to_string();
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(&entrypoint, SUPERSEDING).expect("the scratch area is writable");
    let Some(project) = harness::scratch_project("intake-superseded-queue") else {
        return;
    };
    let Some(mut served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let a1 = execution_of(&post(
        &app,
        "/latest",
        &json!({ "group": "a", "tag": "a1", "hold": hold(&release) }),
    ));
    let b1 = post(
        &app,
        "/latest",
        &json!({ "group": "b", "tag": "b1", "hold": hold(&released) }),
    );
    assert_eq!(b1.json()["status"], "queued", "{}", b1.text());
    let b1 = intake_id_of(&b1);
    let b2 = post(
        &app,
        "/latest",
        &json!({ "group": "b", "tag": "b2", "hold": hold(&released) }),
    );
    assert_eq!(b2.json()["status"], "queued", "{}", b2.text());
    let b2 = intake_id_of(&b2);
    let successor = execution_named_by(&project, &b2);

    let detail =
        format!("a newer arrival of its group superseded it before it started, for {successor}");
    let report = get(&app, &format!("/intakes/{b1}")).json();
    assert_eq!(report["state"], "dropped", "{report}");
    assert_eq!(report["detail"], detail.as_str(), "{report}");
    assert!(
        report.get("execution_id").is_none() && report.get("running_execution_id").is_none(),
        "a replaced arrival names no execution of its own or running: {report}"
    );
    assert_eq!(
        get(&app, &format!("/intakes/{b2}")).json()["state"],
        "queued",
        "the newest waits in its place"
    );

    std::fs::write(&release, "").expect("the release file");
    let rows = eventually(Duration::from_secs(60), "a1 and b2 to complete", || {
        let rows = executions_of(&project, "latest");
        (rows.len() == 2 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
    });
    never_overlapping(&rows);
    let order: Vec<&str> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag"))
        .collect();
    assert_eq!(order, ["a1", "b2"], "the older arrival of b never ran");
    assert_eq!(rows[0].id, a1);
    assert_eq!(rows[1].id, successor);
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        executions_of(&project, "latest").len(),
        2,
        "nothing more started"
    );

    let said = served.stop_and_read_stderr();
    let dropped = format!("intake `latest`: {b1} dropped: {detail}");
    assert_eq!(
        said.lines().filter(|line| *line == dropped).count(),
        1,
        "`{dropped}`\n{said}"
    );
}

/// The composition
/// [`a_superseding_arrival_starts_while_the_execution_it_cancelled_unwinds`]
/// serves: one key-less group of one slot, superseding, over a flow whose work
/// ends when the file its `hold` names appears and whose `finally:` node ends
/// when the file its `unwind` names does — so a superseded execution stays
/// open, unwinding, for exactly as long as the test leaves that file absent.
const UNWINDING: &str = r#"version: "0.1"

triggers:
  newest_now:
    type: http
    path: /newest-now
    flow: flow.unwinding
    input:
      tag: "payload.body.tag"
      hold: "payload.body.hold"
      unwind: "payload.body.unwind"
    concurrency:
      on_conflict: supersede

flow.unwinding:
  description: Work that ends when a file appears, and a finally node that ends when another does.
  inputs:
    tag: { type: string }
    hold: { type: string }
    unwind: { type: string }
  outputs: {}
  nodes:
    wait:
      exec:
        command: "${TALLY_BIN}/hold"
      input:
        tag: "input.tag"
        hold: "input.hold"
    unwinding:
      exec:
        command: "${TALLY_BIN}/hold"
      input:
        tag: "input.tag"
        hold: "input.unwind"
  edges:
    - { from: start, to: wait }
    - { from: wait, to: end }
  finally: unwinding
"#;

/// **A superseding arrival starts when its predecessor's cancel is recorded,
/// not when that execution settles** (PRD resolved q77 ruling b, grammar 13.1
/// "starts the arrival in the slot that frees", Decision D163's Status: "The
/// lease frees at the cancel, not the settle").
///
/// `newest_now` holds one slot and supersedes. `first` takes it, its work
/// waiting on a file that never appears and its `finally:` node on a file the
/// test writes last. `second` arrives: it cancels `first` and is answered as an
/// arrival that **started** — an `execution_id`, no `intake_id`, which is what
/// a queued arrival would have been given instead. While `first` is still
/// open, its cancel recorded and its finally node holding it there, `second`
/// runs to completion beside it: the lease no longer counted `first` once its
/// cancel was in the journal. Only then is `first`'s finally node released,
/// and `first` settles `cancelled` — after `second` both started and ended.
///
/// The test is deterministic rather than timed: a supersede that waited for
/// the settle would leave `second` queued behind an execution that cannot
/// settle until `second` has been seen to finish.
#[test]
fn a_superseding_arrival_starts_while_the_execution_it_cancelled_unwinds() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-supersede-unwinding");
    let (environment, _log) = environment(&provider, &scratch, &[]);
    let never = scratch.path().join("never");
    let unwound = scratch.path().join("unwound");
    let present = scratch.path().join("present");
    std::fs::write(&present, "").expect("the scratch area is writable");
    let hold = |file: &Path| file.display().to_string();
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(&entrypoint, UNWINDING).expect("the scratch area is writable");
    let Some(project) = harness::scratch_project("intake-supersede-unwinding") else {
        return;
    };
    let Some(mut served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
    else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");

    let answered = post(
        &app,
        "/newest-now",
        &json!({ "tag": "first", "hold": hold(&never), "unwind": hold(&unwound) }),
    );
    let first = execution_of(&answered);
    assert!(
        answered.json().get("intake_id").is_none(),
        "an arrival its free slot starts at once has no intake id: {}",
        answered.text()
    );
    eventually(Duration::from_secs(30), "first's lifecycle row", || {
        executions_of(&project, "newest_now")
            .iter()
            .any(|row| row.id == first && row.status == "open")
            .then_some(())
    });

    let answered = post(
        &app,
        "/newest-now",
        &json!({ "tag": "second", "hold": hold(&present), "unwind": hold(&present) }),
    );
    let second = execution_of(&answered);
    assert!(
        answered.json().get("intake_id").is_none() && answered.json()["status"] == "running",
        "the superseding arrival started at once, into the slot the cancel freed, \
         rather than queueing behind the execution it cancelled: {}",
        answered.text()
    );
    assert_ne!(second, first);

    // `second` runs whole while `first` is still open, unwinding through the
    // finally node that waits on a file nothing has written yet.
    let rows = eventually(
        Duration::from_secs(60),
        "second to complete while first unwinds",
        || {
            let rows = executions_of(&project, "newest_now");
            rows.iter()
                .any(|row| row.id == second && row.status == "completed")
                .then_some(rows)
        },
    );
    let unwinding = rows
        .iter()
        .find(|row| row.id == first)
        .expect("first's lifecycle row");
    assert_eq!(
        (unwinding.status.as_str(), unwinding.ended_at.as_deref()),
        ("open", None),
        "first had not settled when second finished: its finally node still holds it"
    );
    let cancels = harness::journal_rows(
        &project,
        &format!("SELECT COUNT(*) AS n FROM cancellations WHERE execution = '{first}'"),
    );
    assert_eq!(
        cancels[0]["n"], 1,
        "first's cancel was recorded before second began: {cancels}"
    );
    let started: Vec<Value> = intakes_of(&project, "newest_now")
        .into_iter()
        .filter(|row| row["execution"] == second.as_str())
        .collect();
    assert_eq!(started.len(), 1, "{started:?}");
    assert_eq!(started[0]["state"], "started", "{started:?}");
    assert_eq!(started[0]["waited"], 0, "second never waited: {started:?}");

    std::fs::write(&unwound, "").expect("the unwind file");
    let rows = eventually(Duration::from_secs(60), "first to settle", || {
        let rows = executions_of(&project, "newest_now");
        rows.iter()
            .any(|row| row.id == first && row.status != "open")
            .then_some(rows)
    });
    let first_row = rows.iter().find(|row| row.id == first).expect("first");
    let second_row = rows.iter().find(|row| row.id == second).expect("second");
    assert_eq!(first_row.status, "cancelled", "{rows:#?}");
    let first_ended = first_row.ended_at.as_deref().expect("first ended");
    let second_ended = second_row.ended_at.as_deref().expect("second ended");
    assert!(
        second_row.started_at.as_str() < first_ended && second_ended < first_ended,
        "second began at {} and ended at {second_ended}, both before first settled at \
         {first_ended}: the slot freed at the cancel, not the settle",
        second_row.started_at
    );

    let said = served.stop_and_read_stderr();
    let superseded = format!("intake `newest_now`: superseded {first} for {second}");
    assert!(
        said.lines().any(|line| line == superseded),
        "the serve log says `{superseded}`:\n{said}"
    );
}

/// The host module [`arrivals_behind_a_full_group_never_read_its_queue`] serves
/// through: the journal the app is about to open, with every read of a
/// trigger's intake rows in one state, and of its lease, written as a line to
/// `READS_LOG` — `rows <trigger> <state>` and `lease <trigger>` — before it is
/// made.
const READS_COUNTED: &str = r#"import process from "node:process";
import { appendFileSync } from "node:fs";
import { openJournal } from "./src/journal.ts";

await import("./src/index.ts");
const journal = await openJournal();
const log = process.env.READS_LOG;
const intakesOf = journal.intakesOf.bind(journal);
journal.intakesOf = (trigger, state) => {
  appendFileSync(log, `rows ${trigger} ${state}\n`);
  return intakesOf(trigger, state);
};
const leasesOf = journal.leasesOf.bind(journal);
journal.leasesOf = (trigger) => {
  appendFileSync(log, `lease ${trigger}\n`);
  return leasesOf(trigger);
};
const { runMain } = await import("./src/cli.ts");
await runMain(process.argv.slice(2));
"#;

/// **Arrivals behind a full group never read its queue** (Decision D163's
/// Status: what a look costs is what it can start).
///
/// Every arrival looks at its trigger's queue before it is placed, so a look
/// that read the whole queue — or the lease once per queued row — would make
/// each arrival of a burst cost more than the one before it, and filling a
/// queue cost the square of its depth. `held`'s one slot is taken by an
/// execution waiting for a file, and twelve more deliveries queue behind it:
/// no look in that time reads the queue's rows, since none of its groups had
/// a slot to start one into, and the lease is read a bounded number of times a
/// delivery — not once more for every row ahead of it. Once the file appears
/// the thirteen run, one at a time, in the order they arrived.
#[test]
fn arrivals_behind_a_full_group_never_read_its_queue() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("intake-burst-reads");
    let reads = scratch.path().join("reads.log");
    let (environment, _log) = environment(
        &provider,
        &scratch,
        &[("READS_LOG", reads.to_str().expect("a UTF-8 path"))],
    );
    let release = scratch.path().join("release");
    let hold = release.display().to_string();
    let Some((project, built)) =
        harness::build_entrypoint(&harness::fixture(FIXTURE), "intake-burst-reads")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let Some(served) = harness::serve_built_hosted(&project, READS_COUNTED, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client");
    let first = execution_of(&post(
        &app,
        "/held",
        &json!({ "tag": "first", "hold": hold }),
    ));
    for n in 0..12 {
        let answered = post(
            &app,
            "/held",
            &json!({ "tag": format!("queued {n}"), "hold": hold }),
        );
        assert_eq!(answered.json()["status"], "queued", "{}", answered.text());
    }
    let lines: Vec<String> = std::fs::read_to_string(&reads)
        .expect("the reads were logged")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        lines
            .iter()
            .filter(|line| *line == "rows held queued")
            .count(),
        0,
        "no look read the queue's rows while its one group was full: {lines:#?}"
    );
    let leases = lines.iter().filter(|line| *line == "lease held").count();
    assert!(
        leases <= 3 * 13,
        "the lease was read {leases} times for 13 deliveries — more than a bounded few each: {lines:#?}"
    );

    std::fs::write(&release, "").expect("the release file");
    let rows = eventually(
        Duration::from_secs(60),
        "the thirteen executions to complete",
        || {
            let rows = executions_of(&project, "held");
            (rows.len() == 13 && rows.iter().all(|row| row.status == "completed")).then_some(rows)
        },
    );
    never_overlapping(&rows);
    assert_eq!(rows[0].id, first);
    let order: Vec<String> = rows
        .iter()
        .map(|row| row.inputs["tag"].as_str().expect("a tag").to_string())
        .collect();
    let expected: Vec<String> = std::iter::once("first".to_string())
        .chain((0..12).map(|n| format!("queued {n}")))
        .collect();
    assert_eq!(order, expected, "first in, first out");
}
