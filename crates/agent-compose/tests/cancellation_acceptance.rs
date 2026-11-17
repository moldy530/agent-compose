//! Cancelling an execution, end to end: `POST /executions/:id/cancel` and
//! `agent-compose cancel` against a real `serve` (PRD resolved q75, Decision
//! D160, grammar §7.9 and §13.3, `docs/durability.md` §6.1, `docs/trace.md` §2).
//!
//! Every test here serves the `cancellation` fixture, whose long-running nodes
//! are `${TALLY_BIN}/linger` — a shim that appends its own pid to
//! `$TALLY_LOG.pids` and then becomes a `sleep` — and whose finally nodes are
//! `${TALLY_BIN}/ended`, which appends `<execution.outcome>|<execution.error>`
//! to `$TALLY_LOG`. So "the cancel stopped the process" is a question asked of a
//! pid, and "the finally node ran, once, at `cancelled`" is a line count.
//!
//! What the ruling asks to be shown, and where each is:
//!
//! * a cancel of a running node answers `202`, settles the row `cancelled`
//!   within ten seconds, leaves no process alive, runs the finally node at
//!   `cancelled`, and the settle webhook says so; a second cancel is `202`, a
//!   cancel after the end is `409`, and an unknown id is `404` —
//!   [`a_cancel_stops_a_running_command_and_settles_the_execution_cancelled`];
//! * the cascade over a lineage, through a child that has already completed,
//!   and a child cancelled alone —
//!   [`a_cancel_of_a_parent_cascades_to_the_child_it_started`],
//!   [`a_cancel_of_a_root_reaches_a_grandchild_its_completed_child_started`] and
//!   [`a_child_cancelled_alone_leaves_its_parent_running_to_its_own_end`];
//! * a `human` wait released, live and across a restart —
//!   [`a_cancel_releases_a_human_pause_and_settles_the_execution_cancelled`] and
//!   [`a_restart_in_the_middle_of_a_cancel_at_a_human_pause_completes_it`] (the
//!   cancel route's auth is pinned beside the status and resume routes', in
//!   `compiled_graph_acceptance.rs`);
//! * `resume` refuses a cancelled execution, at the route and at the command
//!   line, naming the status —
//!   [`a_cancelled_execution_is_refused_by_resume_at_the_route_and_the_command_line`];
//! * a restart between the `202` and the settled row completes the cancel and
//!   issues nothing new —
//!   [`a_restart_in_the_middle_of_a_cancel_completes_it_and_issues_nothing_new`]
//!   — and one between the parent's request and its child's completes the
//!   cascade too —
//!   [`a_restart_between_a_parents_cancel_and_its_childs_cancels_both`];
//! * "no child process alive" asked of every process in the tree, not of the
//!   command's own pid: a command that forks, and one that ignores `SIGTERM` —
//!   [`a_cancel_stops_every_process_a_forking_command_started`] and
//!   [`a_command_that_ignores_sigterm_is_killed_once_its_grace_has_passed`];
//! * a finally node's detached delivery is part of how its instance ends, and
//!   runs on a cancelled execution — and so does the child execution a
//!   detached `flow.*` of it starts —
//!   [`a_finally_nodes_detached_delivery_runs_on_a_cancelled_execution`] and
//!   [`a_finally_nodes_detached_child_execution_runs_on_a_cancelled_execution`];
//! * `agent-compose cancel`'s exit codes, and the `hub.public_url` it sends to
//!   when no `--url` is given —
//!   [`agent_compose_cancel_exits_by_what_the_app_answered`] and
//!   [`agent_compose_cancel_finds_the_app_at_the_targets_public_url`];
//! * and `run`'s posture, which a cancel does not change: an interrupted run's
//!   row stays open for `resume` (PRD resolved q28) —
//!   [`an_interrupted_run_is_left_open_for_resume_rather_than_cancelled`].
//!
//! And what PRD resolved q81 adds: a cancel **closes** an execution the serving
//! build's recovery could not take — its row ended `cancelled` with no finally
//! node run, said in `finally_error` — together with the open children it left
//! that this process is not running, while one it *is* running goes through
//! the cascade; and `agent-compose cancel` prints why —
//! [`a_cancel_closes_an_execution_this_build_cannot_run_and_the_open_child_it_left`],
//! [`a_cancel_of_an_execution_whose_recovery_diverged_cancels_the_child_it_runs_through_the_cascade`]
//! — and one it was running whose own generation then settled with its row
//! still open is closed too, before its parent —
//! [`a_child_the_cascade_reached_waiting_for_its_permit_whose_replay_then_diverged_is_closed_before_its_parent`],
//! and [`agent_compose_cancel_prints_why_it_closed_an_execution_this_build_cannot_run`]
//! (the divergence and the renamed flow under `pushed`'s settle webhook and a
//! trace sink are in `compiled_graph_acceptance.rs`) — while an open row a
//! `run` beside the `serve` is running is still not `serve`'s to cancel —
//! [`serves_cancel_of_an_execution_a_run_beside_it_is_running_is_404_and_leaves_its_row_open`].
//! An execution this build *can* run is never closed so: one a recovery the
//! process owes has not reached yet has its cancel recorded, and that recovery
//! completes it, finally node and all —
//! [`a_cancel_of_an_execution_the_owed_recovery_has_not_reached_is_completed_by_that_recovery`]
//! — and a standby's cancel takes the recovery it owes first, so it cancels a
//! running execution —
//! [`a_standby_recovers_an_execution_before_cancelling_it_and_its_finally_node_runs`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider, Request};
use serde_json::{Value, json};

/// The fixture every test here serves.
const FIXTURE: &str = "cancellation";

/// How long a cancel may take to settle the row (the field report's bound).
const SETTLE: Duration = Duration::from_secs(10);

/// The two shims the fixture names, written into `shims`.
fn shims(shims: &Path) {
    harness::shim(
        shims,
        "linger",
        "echo $$ >> \"$TALLY_LOG.pids\"\nexec sleep \"$LINGER\"\n",
    );
    harness::shim(
        shims,
        "ended",
        "if [ \"$1\" = \"--slowly\" ]; then echo $$ >> \"$TALLY_LOG.slow\"; sleep 4; fi\n\
         printf '%s|%s\\n' \"$OUTCOME\" \"$ERROR\" >> \"$TALLY_LOG\"\n",
    );
    harness::shim(
        shims,
        "ping",
        "printf 'pinged %s\\n' \"$LINE\" >> \"$TALLY_LOG.pings\"\necho done\n",
    );
}

/// Whether a process is gone within `budget` of being asked about.
fn gone_within(pid: i32, budget: Duration) -> bool {
    let deadline = Instant::now() + budget;
    loop {
        // SAFETY: signal 0 delivers nothing; it asks whether `pid` exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return true;
        }
        if Instant::now() > deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Kill every pid a shim wrote to `file`, for a test whose crash leaves them
/// behind: an `exec:` command runs in a process group of its own, so the
/// harness's `SIGKILL` to the serving process's group does not reach it.
fn kill_listed(file: &Path) {
    for pid in std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
    {
        // SAFETY: a pid this test's shim wrote; a gone one answers ESRCH.
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
}

/// The environment a served fixture runs under, with its tally at `log`.
fn environment(
    provider: &MockProvider,
    scratch: &harness::Scratch,
) -> (Vec<(String, String)>, PathBuf) {
    shims(scratch.path());
    let log = scratch.path().join("ended.log");
    let mut held = harness::environment(provider);
    held.push((harness::CALLBACK_RETRY.to_string(), "0s,0s,0s".to_string()));
    held.push((
        harness::TALLY_BIN.to_string(),
        scratch.path().display().to_string(),
    ));
    held.push((harness::TALLY_LOG.to_string(), log.display().to_string()));
    (held, log)
}

/// The pids the linger shim wrote, once there are at least `count` of them.
fn pids(log: &Path, count: usize) -> Vec<i32> {
    let file = PathBuf::from(format!("{}.pids", log.display()));
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let held: Vec<i32> = std::fs::read_to_string(&file)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse().ok())
            .collect();
        if held.len() >= count {
            return held;
        }
        assert!(
            Instant::now() < deadline,
            "the linger shim wrote {} pid(s), not {count}",
            held.len()
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether a process is still there, for a moment after it was asked to stop.
fn alive(pid: i32) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        // SAFETY: signal 0 delivers nothing; it asks whether `pid` exists.
        if unsafe { libc::kill(pid, 0) } != 0 {
            return false;
        }
        if Instant::now() > deadline {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// What the finally nodes recorded, line by line.
fn ended(log: &Path) -> Vec<String> {
    std::fs::read_to_string(log)
        .unwrap_or_default()
        .lines()
        .map(str::to_string)
        .collect()
}

/// Start one execution and answer its id.
fn started(app: &Client, route: &str, body: &Value) -> String {
    let answered = app
        .send(Request::post(route).json(body))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    answered.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string()
}

/// `POST /executions/:id/cancel`, answering the status and the body.
fn cancel(app: &Client, execution: &str) -> (u16, Value) {
    let answered = app
        .send(Request::post(format!("/executions/{execution}/cancel")))
        .expect("the cancel route answers");
    let status = answered.status;
    (status, answered.json())
}

/// Poll the status route until it reports `status`, within `budget`.
fn reaches(app: &Client, execution: &str, status: &str, budget: Duration) -> Value {
    let deadline = Instant::now() + budget;
    loop {
        let report: Value = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        if report["status"] == status {
            return report;
        }
        assert!(
            Instant::now() < deadline,
            "execution `{execution}` did not reach `{status}` within {budget:?}: {report:#}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// The compiler binary, sealed to `environment`.
fn compiler(environment: &[(String, String)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
    harness::seal(&mut command, environment);
    command
}

/// **A cancel stops a running node and settles the execution `cancelled`**
/// (PRD resolved q75 rulings a, b, d; Decision D160).
///
/// `flow.long`'s `work` is a ten-minute command. The cancel is answered `202`
/// with the status route to watch; the row settles `cancelled` inside the
/// field report's ten seconds; the command's process is gone; the finally node
/// ran once and read `execution.outcome` as `cancelled` with no
/// `execution.error`; the report and the settle webhook both say `cancelled`
/// with no `error`; the stopped node's trace entry is `cancelled` with no
/// `error` and no `routing`, and the finally node's entry follows it. Around it,
/// the route's other answers: a second cancel while the first is settling is
/// `202` too — the request stands either way — a cancel after the end is `409`
/// naming the status, and an id the process does not know is `404`.
#[test]
fn a_cancel_stops_a_running_command_and_settles_the_execution_cancelled() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-running");
    let (environment, log) = environment(&provider, &scratch);
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("cancel-running") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(
        &app,
        "/long",
        &json!({ "linger": 600, "callback_url": format!("{}/hook", receiver.base_url) }),
    );
    let running = pids(&log, 1);

    let asked = Instant::now();
    let (status, body) = cancel(&app, &execution);
    assert_eq!(status, 202, "{body:#}");
    assert_eq!(body["execution_id"], execution, "{body:#}");
    assert_eq!(body["cancelling"], true, "{body:#}");
    assert_eq!(
        body["status_url"],
        format!("/executions/{execution}"),
        "{body:#}"
    );
    let (again, said) = cancel(&app, &execution);
    assert!(
        again == 202 || again == 409,
        "a second cancel is `202` while the first settles, `409` once it has: {said:#}"
    );

    let report = reaches(&app, &execution, "cancelled", SETTLE);
    assert!(
        asked.elapsed() < SETTLE,
        "the cancel took {:?} to settle the row",
        asked.elapsed()
    );
    assert!(
        report.get("error").is_none(),
        "a cancelled report carries no `error`: {report:#}"
    );
    assert_eq!(report["trace_version"], 7, "{report:#}");
    let trace = report["trace"]
        .as_array()
        .expect("a settled report carries its trace");
    let work = trace
        .iter()
        .find(|entry| entry["node"] == "work")
        .unwrap_or_else(|| panic!("the stopped node has an entry: {report:#}"));
    assert_eq!(work["outcome"], "cancelled", "{work:#}");
    assert!(work.get("error").is_none(), "{work:#}");
    assert!(work.get("routing").is_none(), "{work:#}");
    let last = trace.last().expect("entries");
    assert_eq!(
        last["node"], "record",
        "the finally node's entry is last: {report:#}"
    );
    assert_eq!(last["outcome"], "completed", "{last:#}");

    assert!(
        !alive(running[0]),
        "the command the cancelled node was running is still alive"
    );
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the finally node ran once, reading `cancelled` and no `execution.error`"
    );

    let settled = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    assert_eq!(
        settled[0].body["status"], "cancelled",
        "{:#}",
        settled[0].body
    );
    assert!(
        settled[0].body.get("error").is_none(),
        "the settle webhook carries no `error` for a cancel: {:#}",
        settled[0].body
    );
    assert_eq!(settled[0].body["execution_id"], execution);

    let (after, refused) = cancel(&app, &execution);
    assert_eq!(after, 409, "{refused:#}");
    assert_eq!(
        refused,
        json!({
            "execution_id": execution,
            "status": "cancelled",
            "error": "this execution has already been cancelled, so there is nothing to cancel",
        })
    );
    let (unknown, said) = cancel(&app, "exec_nobody_started_this");
    assert_eq!(unknown, 404, "{said:#}");
    assert_eq!(
        said["error"],
        "no execution `exec_nobody_started_this` was started by this process"
    );
}

/// **A cancel of a parent cascades to the child it started** (PRD resolved
/// q75 ruling c, q65; Decision D160).
///
/// `flow.parent` dispatches one detached `flow.long` child — a child execution
/// with a row of its own — and then runs a ten-minute command of its own. A
/// cancel of the parent stops both commands, settles both rows `cancelled`, and
/// runs both finally nodes at `cancelled`.
#[test]
fn a_cancel_of_a_parent_cascades_to_the_child_it_started() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-cascade");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-cascade") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let parent = started(
        &app,
        "/parent",
        &json!({ "child_lingers": [600], "linger": 600 }),
    );
    let running = pids(&log, 2);
    let (status, body) = cancel(&app, &parent);
    assert_eq!(status, 202, "{body:#}");
    reaches(&app, &parent, "cancelled", SETTLE);

    for pid in &running {
        assert!(!alive(*pid), "process {pid} outlived the cascade");
    }
    let deadline = Instant::now() + SETTLE;
    while ended(&log).len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        ended(&log),
        ["cancelled|none", "cancelled|none"],
        "the child's finally node and the parent's both ran, at `cancelled`"
    );
}

/// **A child cancelled alone leaves its parent running** (Decision D160, D94).
///
/// The parent's detached dispatch resolved when it was issued, so nothing of
/// the parent waits on the child: the child's command stops and its finally
/// node reads `cancelled`, while the parent's own command keeps running and the
/// parent reaches its own end — `completed`, its finally node reading so.
#[test]
fn a_child_cancelled_alone_leaves_its_parent_running_to_its_own_end() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-child");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-child") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // The parent's own command lingers four seconds: long enough for the child
    // to be cancelled while it runs, short enough to wait out.
    let parent = started(
        &app,
        "/parent",
        &json!({ "child_lingers": [600], "linger": 4 }),
    );
    pids(&log, 2);
    let report: Value = app
        .get(&format!("/executions/{parent}"))
        .expect("the status route answers")
        .json();
    let child = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "spawn")
        .and_then(|entry| {
            entry["dispatches"][0]["executionId"]
                .as_str()
                .map(str::to_string)
        })
        .or_else(|| child_of(&project, &parent));
    let child =
        child.unwrap_or_else(|| panic!("the parent's child is named somewhere: {report:#}"));

    let (status, body) = cancel(&app, &child);
    assert_eq!(status, 202, "{body:#}");
    reaches(&app, &child, "cancelled", SETTLE);
    let finished = reaches(&app, &parent, "completed", Duration::from_secs(30));
    assert!(finished.get("error").is_none(), "{finished:#}");
    let mut lines = ended(&log);
    lines.sort();
    assert_eq!(
        lines,
        ["cancelled|none", "completed|none"],
        "the child ended `cancelled` and the parent ran to its own end"
    );
}

/// The child execution `parent` started, read off the journal file's bytes: a
/// child's id is its parent's with a suffix of its own (`docs/durability.md`
/// §3.2), and the journal is plain text in its pages.
fn child_of(project: &Path, parent: &str) -> Option<String> {
    let bytes = std::fs::read(project.join(".agent-compose").join("journal.sqlite")).ok()?;
    let text = String::from_utf8_lossy(&bytes);
    let mut found: Vec<String> = Vec::new();
    // An id is `exec_` and a UUID's thirty-six characters, and a page packs
    // the bytes of the next column straight after it, so it is read by width.
    for (at, _) in text.match_indices("exec_") {
        let Some(id) = text.get(at..at + 41) else {
            continue;
        };
        let shaped = id[5..].char_indices().all(|(index, c)| {
            if [8, 13, 18, 23].contains(&index) {
                c == '-'
            } else {
                c.is_ascii_hexdigit()
            }
        });
        if shaped && id != parent && !found.iter().any(|held| held == id) {
            found.push(id.to_string());
        }
    }
    found.into_iter().next()
}

/// **A cancelled execution is never resumed**, at the route or at the command
/// line, and the refusal names the status (PRD resolved q75 ruling d).
#[test]
fn a_cancelled_execution_is_refused_by_resume_at_the_route_and_the_command_line() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-resume");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-resume") else {
        return;
    };
    let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    pids(&log, 1);
    assert_eq!(cancel(&app, &execution).0, 202);
    reaches(&app, &execution, "cancelled", SETTLE);

    let answered = app
        .send(Request::post(format!("/executions/{execution}/resume")).json(&json!({})))
        .expect("the resume route answers");
    assert_eq!(answered.status, 409, "{}", answered.text());
    assert_eq!(
        answered.json(),
        json!({
            "execution_id": execution,
            "status": "cancelled",
            "error": "this execution was cancelled, so nothing is waiting for an answer: a \
                      cancelled execution is ended for good and is never resumed",
        })
    );

    // The command line reads the same journal once the app has let it go.
    served.stop();
    let output = compiler(&environment)
        .arg("resume")
        .arg(harness::fixture(FIXTURE))
        .arg(&execution)
        .arg("--out")
        .arg(&project)
        .output()
        .expect("the compiler runs");
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let said = String::from_utf8_lossy(&output.stderr);
    assert!(
        said.contains(&format!("`{execution}` was cancelled (")),
        "the refusal names the status and when: {said}"
    );
    assert!(
        said.contains(
            "), so there is nothing to resume: a cancelled execution is ended for good, and \
             the journal keeps its record"
        ),
        "{said}"
    );
}

/// **A restart in the middle of a cancel completes it** and issues nothing new
/// (PRD resolved q75 ruling e, `docs/durability.md` §6.1).
///
/// `flow.lingering_end`'s finally node takes four seconds before it records
/// anything. The cancel is accepted, the command is stopped, the finally node's
/// boundary reaches the journal — and the serving process is killed with
/// `SIGKILL` while that node is still sleeping, so the row is open, carrying the
/// request. A second `serve` over the same journal recovers it: the row settles
/// `cancelled`, the stopped command is **not** started again (one pid, ever),
/// and the finally node's line is written exactly once.
#[test]
fn a_restart_in_the_middle_of_a_cancel_completes_it_and_issues_nothing_new() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-restart");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-restart") else {
        return;
    };
    let execution = {
        let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client for the generated app");
        let execution = started(&app, "/lingering-end", &json!({ "linger": 600 }));
        let running = pids(&log, 1);
        assert_eq!(cancel(&app, &execution).0, 202);
        let deadline = Instant::now() + SETTLE;
        while !harness::journal_holds(&project, &["slow_record/0#finally/0"]) {
            assert!(
                Instant::now() < deadline,
                "the finally node's boundary never reached the journal"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(
            !alive(running[0]),
            "the cancel stopped the command before the kill"
        );
        assert!(
            ended(&log).is_empty(),
            "the slow finally node had not recorded yet"
        );
        served.stop();
        // The machine went with it: the finally node's command runs in a
        // process group of its own (grammar 5.5, Decision D160), so the kill
        // above does not reach it, and a crash that left it alive to finish
        // would not be the crash this test is about.
        kill_listed(&PathBuf::from(format!("{}.slow", log.display())));
        execution
    };

    let Some(recovered) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&recovered.base_url).expect("a client for the recovered app");
    let report = reaches(&app, &execution, "cancelled", Duration::from_secs(30));
    assert!(report.get("error").is_none(), "{report:#}");
    assert_eq!(
        pids(&log, 1).len(),
        1,
        "the recovered generation started the stopped command again"
    );
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the finally node recorded once, at `cancelled`"
    );
}

/// **A cancel stops every process a forking command started** (PRD resolved
/// q75 ruling b's "no child process alive", Decision D160).
///
/// The linger shim here does not `exec` its `sleep`: it forks it, records both
/// pids — its own and the `sleep`'s — and waits. A signal aimed at the
/// command's own pid would end the shell and leave the `sleep` running,
/// reparented; the cancel stops the command's whole process group, so both are
/// gone, and the row settles `cancelled` inside the field report's ten seconds.
#[test]
fn a_cancel_stops_every_process_a_forking_command_started() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-forking");
    let (environment, log) = environment(&provider, &scratch);
    harness::shim(
        scratch.path(),
        "linger",
        "echo $$ >> \"$TALLY_LOG.pids\"\nsleep \"$LINGER\" &\necho $! >> \"$TALLY_LOG.pids\"\nwait\n",
    );
    let Some(project) = harness::scratch_project("cancel-forking") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    let tree = pids(&log, 2);
    let asked = Instant::now();
    assert_eq!(cancel(&app, &execution).0, 202);
    reaches(&app, &execution, "cancelled", SETTLE);
    for pid in &tree {
        assert!(
            gone_within(*pid, SETTLE.saturating_sub(asked.elapsed())),
            "process {pid} of the cancelled command's tree {tree:?} is still alive"
        );
    }
    assert_eq!(ended(&log), ["cancelled|none"]);
}

/// **A command that ignores `SIGTERM` is killed once its grace has passed**
/// (PRD resolved q75 ruling b, Decision D160).
///
/// The shim turns `SIGTERM` off — for itself and, since an ignored signal is
/// inherited, for the `sleep` it forks — so the polite half of the stop
/// reaches nothing. The node does not wait for it: the row settles `cancelled`
/// and the finally node runs at once. The kill that follows the grace takes
/// the whole group, so both processes are gone inside the ten seconds.
#[test]
fn a_command_that_ignores_sigterm_is_killed_once_its_grace_has_passed() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-stubborn");
    let (environment, log) = environment(&provider, &scratch);
    harness::shim(
        scratch.path(),
        "linger",
        "trap '' TERM\necho $$ >> \"$TALLY_LOG.pids\"\nsleep \"$LINGER\" &\n\
         echo $! >> \"$TALLY_LOG.pids\"\nwait\n",
    );
    let Some(project) = harness::scratch_project("cancel-stubborn") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    let tree = pids(&log, 2);
    let asked = Instant::now();
    assert_eq!(cancel(&app, &execution).0, 202);
    reaches(&app, &execution, "cancelled", SETTLE);
    for pid in &tree {
        assert!(
            gone_within(*pid, SETTLE.saturating_sub(asked.elapsed())),
            "process {pid} of a command that ignores SIGTERM outlived the cancel's ten \
             seconds; its tree was {tree:?}"
        );
    }
    assert_eq!(ended(&log), ["cancelled|none"]);
}

/// **A finally node's detached delivery runs on a cancelled execution**
/// (grammar 7.9, Decision D160: "a finally node and all it begins").
///
/// `flow.notifying`'s finally node is a detached `map` onto `tool.ping`, handed
/// `execution.outcome`. A cancel stops the command, and the finally node runs
/// *because* of it — so the delivery it issues chains to nothing, rather than
/// to the execution's cancel, which has already fired: the sink runs and
/// records `pinged cancelled`. Read through the execution's signal, the
/// delivery would be stopped before its command began.
#[test]
fn a_finally_nodes_detached_delivery_runs_on_a_cancelled_execution() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-finally-delivery");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-finally-delivery") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(
        &app,
        "/notifying",
        &json!({ "names": ["ops"], "linger": 600 }),
    );
    let running = pids(&log, 1);
    assert_eq!(cancel(&app, &execution).0, 202);
    let report = reaches(&app, &execution, "cancelled", SETTLE);
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        !alive(running[0]),
        "the command the cancelled node was running is still alive"
    );

    let pings = PathBuf::from(format!("{}.pings", log.display()));
    let deadline = Instant::now() + SETTLE;
    while std::fs::read_to_string(&pings)
        .unwrap_or_default()
        .is_empty()
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        std::fs::read_to_string(&pings).unwrap_or_default(),
        "pinged cancelled\n",
        "the finally node's detached delivery did not run on the cancelled execution: {report:#}"
    );
}

/// **A finally node's detached child execution runs on a cancelled execution**
/// (grammar 7.9, Decision D160: "a finally node and all it begins"; PRD
/// resolved q65).
///
/// `flow.notifying_child`'s finally node is a detached `map` onto
/// `flow.pinger`, so what it begins is a child execution with a row of its
/// own. The cancel stops the command and the finally node runs *because* of it:
/// the child it issues is begun on a parent whose cancel has fired — and is
/// still not the cancel's, so it carries no cancel request of its own, runs its
/// node, records `pinged cancelled`, and settles `completed`. Read as a child
/// a cancelled parent was issuing as the cancel came, it would be begun
/// cancelled and its node never run.
#[test]
fn a_finally_nodes_detached_child_execution_runs_on_a_cancelled_execution() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-finally-child");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-finally-child") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(
        &app,
        "/notifying-child",
        &json!({ "names": ["ops"], "linger": 600 }),
    );
    let running = pids(&log, 1);
    assert_eq!(cancel(&app, &execution).0, 202);
    let report = reaches(&app, &execution, "cancelled", SETTLE);
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        !alive(running[0]),
        "the command the cancelled node was running is still alive"
    );

    // The child's row: begun by the finally node, and run to its own end.
    let child_rows = || {
        harness::journal_rows(
            &project,
            &format!(
                "SELECT executions.id AS id, status, \
                 (SELECT COUNT(*) FROM cancellations WHERE cancellations.execution = executions.id) \
                 AS cancels FROM executions WHERE id != '{execution}'"
            ),
        )
    };
    let deadline = Instant::now() + SETTLE;
    let mut rows = child_rows();
    while rows[0]["status"] != "completed"
        && rows[0]["status"] != "cancelled"
        && Instant::now() < deadline
    {
        std::thread::sleep(Duration::from_millis(20));
        rows = child_rows();
    }
    let rows = rows.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "one child execution: {rows:#?}");
    assert_eq!(
        (rows[0]["status"].clone(), rows[0]["cancels"].clone()),
        (json!("completed"), json!(0)),
        "the child the finally node began was cancelled with its parent: {rows:#?}"
    );
    let pings = PathBuf::from(format!("{}.pings", log.display()));
    assert_eq!(
        std::fs::read_to_string(&pings).unwrap_or_default(),
        "pinged cancelled\n",
        "the finally node's child execution did not run its node: {report:#}"
    );
}

/// **A restart between a parent's cancel and its child's completes both**
/// (PRD resolved q75 rulings c and e, `docs/durability.md` §6.1).
///
/// The cancel route records the parent's request and then walks its open
/// children, and a process can die between the two. Made here as the journal
/// that crash leaves: a parent and its child both running, the serving process
/// killed, and the parent's request — and only the parent's — written into the
/// `cancellations` table. The generation that recovers the parent walks the
/// cascade again before its replay re-issues the dispatch, so the child
/// resumes carrying a request of its own: both rows settle `cancelled`, both
/// finally nodes run at `cancelled`, and neither command is started again.
#[test]
fn a_restart_between_a_parents_cancel_and_its_childs_cancels_both() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-restart-cascade");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-restart-cascade") else {
        return;
    };
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let parent = {
        let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client for the generated app");
        let parent = started(
            &app,
            "/parent",
            &json!({ "child_lingers": [600], "linger": 600 }),
        );
        pids(&log, 2);
        served.stop();
        // The commands run in process groups of their own, so the kill above
        // does not reach them; the machine went with it.
        kill_listed(&pid_file);
        parent
    };
    let child = child_of(&project, &parent).expect("the parent's child has a row");
    let wrote = harness::journal_sql(
        &project,
        &format!(
            "INSERT INTO cancellations (execution, requested_at) \
             VALUES ('{parent}', '2026-11-04T00:00:00.000Z');"
        ),
    );
    assert!(
        wrote.status.success(),
        "{}",
        String::from_utf8_lossy(&wrote.stderr)
    );

    let Some(recovered) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&recovered.base_url).expect("a client for the recovered app");
    let report = reaches(&app, &parent, "cancelled", Duration::from_secs(30));
    assert!(report.get("error").is_none(), "{report:#}");
    let deadline = Instant::now() + Duration::from_secs(30);
    let status = loop {
        let rows = harness::journal_rows(
            &project,
            &format!("SELECT status FROM executions WHERE id = '{child}'"),
        );
        let status = rows[0]["status"].as_str().unwrap_or_default().to_string();
        if status != "open" || Instant::now() > deadline {
            break status;
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    assert_eq!(
        status, "cancelled",
        "the child the dying process never reached was not cancelled by the recovery"
    );
    assert_eq!(
        pids(&log, 2).len(),
        2,
        "the recovered generation started a stopped command again"
    );
    assert_eq!(
        ended(&log),
        ["cancelled|none", "cancelled|none"],
        "both finally nodes ran once, at `cancelled`"
    );
}

/// The status of every lifecycle row of `flow`, read off the journal.
fn statuses_of(project: &Path, flow: &str) -> Vec<String> {
    harness::journal_rows(
        project,
        &format!("SELECT status FROM executions WHERE flow = '{flow}'"),
    )
    .as_array()
    .into_iter()
    .flatten()
    .filter_map(|row| row["status"].as_str().map(str::to_string))
    .collect()
}

/// Wait until every lifecycle row of `flow` reads `status`, within `budget`.
fn rows_reach(project: &Path, flow: &str, status: &str, budget: Duration) {
    let deadline = Instant::now() + budget;
    loop {
        let held = statuses_of(project, flow);
        if !held.is_empty() && held.iter().all(|one| one == status) {
            return;
        }
        assert!(
            Instant::now() < deadline,
            "the `{flow}` rows did not reach `{status}` within {budget:?}: {held:?}"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// **A cancel of a root reaches a grandchild its completed child started**
/// (PRD resolved q75 ruling c: "every open child execution in its lineage,
/// recursively"; Decision D160, D94).
///
/// `flow.grandparent` detaches one `flow.relay`, which detaches one ten-minute
/// `flow.long` and **ends at once** — a detached dispatch resolves when it is
/// issued — while the grandparent runs a ten-minute command of its own. So when
/// the cancel comes the lineage is root (open) → relay (`completed`) →
/// grandchild (open). The cascade walks through the settled relay: both
/// commands stop, the grandchild's row settles `cancelled`, the relay's stays
/// `completed`, and both finally nodes ran once at `cancelled`.
#[test]
fn a_cancel_of_a_root_reaches_a_grandchild_its_completed_child_started() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-grandchild");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-grandchild") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let root = started(
        &app,
        "/grandparent",
        &json!({ "relays": [{ "lingers": [600] }], "linger": 600 }),
    );
    let running = pids(&log, 2);
    rows_reach(&project, "flow.relay", "completed", SETTLE);
    assert_eq!(
        statuses_of(&project, "flow.long"),
        ["open"],
        "the grandchild is running when the cancel comes"
    );

    let (status, body) = cancel(&app, &root);
    assert_eq!(status, 202, "{body:#}");
    reaches(&app, &root, "cancelled", SETTLE);
    rows_reach(&project, "flow.long", "cancelled", SETTLE);
    assert_eq!(
        statuses_of(&project, "flow.relay"),
        ["completed"],
        "the settled child between them keeps its own outcome"
    );
    for pid in &running {
        assert!(
            !alive(*pid),
            "process {pid} outlived a cancel of the root its execution descends from"
        );
    }
    let deadline = Instant::now() + SETTLE;
    while ended(&log).len() < 2 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        ended(&log),
        ["cancelled|none", "cancelled|none"],
        "the grandchild's finally node and the root's both ran once, at `cancelled`"
    );
}

/// What the resume route answers for an execution somebody cancelled
/// (`src/serve.ts`'s `CANCELLED_RESUME`).
const CANCELLED_RESUME: &str = "this execution was cancelled, so nothing is waiting for an \
                                answer: a cancelled execution is ended for good and is never \
                                resumed";

/// Wait until `execution` is parked at its pause, and answer the pause's
/// `resume_url`.
fn parked_at(app: &Client, execution: &str) -> String {
    let report = reaches(app, execution, "interrupted", Duration::from_secs(30));
    report["interrupts"][0]["resume_url"]
        .as_str()
        .unwrap_or_else(|| panic!("the pause publishes a resume url: {report:#}"))
        .to_string()
}

/// Assert that a cancelled report publishes no wait, and that an answer sent to
/// the wait it held is refused naming the status.
fn released_and_refused(app: &Client, execution: &str, resume_url: &str, report: &Value) {
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        report["interrupts"].as_array().is_none_or(Vec::is_empty),
        "a cancelled execution still publishes the question it was waiting on: {report:#}"
    );
    let refused = app
        .send(Request::post(resume_url).json(&json!({ "decision": "approve" })))
        .expect("the resume route answers");
    assert_eq!(refused.status, 409, "{}", refused.text());
    assert_eq!(
        refused.json(),
        json!({
            "execution_id": execution,
            "status": "cancelled",
            "error": CANCELLED_RESUME,
        })
    );
}

/// What the resume route answers while an accepted cancel is still settling
/// (`src/serve.ts`'s `CANCELLING_RESUME`).
const CANCELLING_RESUME: &str = "this execution is being cancelled, so every wait it held has \
                                 been released and nothing is waiting for an answer";

/// **A cancel the journal would not record leaves the execution answerable**
/// (PRD resolved q75 rulings d and e, `docs/durability.md` §6.1).
///
/// `flow.asking` parks at its question, and the journal is then made to refuse
/// every write to its `cancellations` table — a trigger raising on insert, the
/// shape a dialled journal's dropped connection takes at the moment the
/// request arrives. The cancel route answers `500`: nothing was recorded, so
/// nothing was fired, and the execution is still running at its pause, which
/// the report still publishes. So an answer to it is **not** refused as one to
/// an execution being cancelled: it is taken, `202`, and the execution runs on
/// to its end — `completed`, its finally node reading so.
#[test]
fn a_cancel_the_journal_would_not_record_leaves_the_execution_answerable() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-unrecorded");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-unrecorded") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(&app, "/asking", &json!({ "path": "release.dmg" }));
    let resume_url = parked_at(&app, &execution);
    let refused = harness::journal_sql(
        &project,
        "CREATE TRIGGER refuse_cancellations BEFORE INSERT ON cancellations \
         BEGIN SELECT RAISE(ABORT, 'the journal is unreachable'); END;",
    );
    assert!(
        refused.status.success(),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let (status, body) = cancel(&app, &execution);
    assert_eq!(
        status, 500,
        "a cancel the journal refused was answered as accepted: {body:#}"
    );
    let still: Value = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert_eq!(still["status"], "interrupted", "{still:#}");
    assert!(
        still["interrupts"]
            .as_array()
            .is_some_and(|interrupts| interrupts.len() == 1),
        "the pause a cancel never fired on is gone from the report: {still:#}"
    );

    let answered = app
        .send(Request::post(&resume_url).json(&json!({ "decision": "approve" })))
        .expect("the resume route answers");
    assert_eq!(
        answered.status,
        202,
        "an answer to an execution whose cancel was never recorded was refused: {}",
        answered.text()
    );
    let report = reaches(&app, &execution, "completed", Duration::from_secs(30));
    assert!(report.get("error").is_none(), "{report:#}");
    assert_eq!(
        ended(&log),
        ["completed|none"],
        "the finally node ran once, at the end the answered run reached"
    );
}

/// **A cancel releases a `human` pause** (PRD resolved q75 ruling e: "human and
/// placement waits are released"; Decision D160's *Waits*).
///
/// `flow.asking` parks at a question with no `timeout:`, so nothing but the
/// cancel can end it, and its finally node takes four seconds — the window in
/// which the cancel is accepted and the row not yet settled. The release is
/// asserted **inside** that window, where a wait still on the board would be
/// published: right after the `202` the report holds no question and an answer
/// is refused as one to an execution being cancelled. Then the row settles
/// `cancelled` inside the field report's ten seconds: the paused node's entry
/// is `cancelled`, the finally node ran once at `cancelled`, and the answer is
/// refused naming the status.
#[test]
fn a_cancel_releases_a_human_pause_and_settles_the_execution_cancelled() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-at-a-pause");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-at-a-pause") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let execution = started(&app, "/asking", &json!({ "path": "release.dmg" }));
    let resume_url = parked_at(&app, &execution);
    let asked = Instant::now();
    let (status, body) = cancel(&app, &execution);
    assert_eq!(status, 202, "{body:#}");

    // Inside the finally node's four seconds: the cancel is accepted and the
    // row is not settled, and the wait is already gone.
    let cancelling: Value = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert!(
        ended(&log).is_empty(),
        "the slow finally node had already recorded, so this is not the cancelling window"
    );
    assert_ne!(cancelling["status"], "cancelled", "{cancelling:#}");
    assert!(
        cancelling["interrupts"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "an execution being cancelled still publishes the question it released: {cancelling:#}"
    );
    let early = app
        .send(Request::post(&resume_url).json(&json!({ "decision": "approve" })))
        .expect("the resume route answers");
    assert_eq!(early.status, 409, "{}", early.text());
    assert_eq!(early.json()["cancelling"], true, "{}", early.text());
    assert_eq!(early.json()["error"], CANCELLING_RESUME, "{}", early.text());

    let report = reaches(&app, &execution, "cancelled", SETTLE);
    assert!(
        asked.elapsed() < SETTLE,
        "the cancel took {:?} to settle an execution parked at a pause",
        asked.elapsed()
    );
    released_and_refused(&app, &execution, &resume_url, &report);
    let ask = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "ask")
        .unwrap_or_else(|| panic!("the paused node has an entry: {report:#}"));
    assert_eq!(ask["outcome"], "cancelled", "{ask:#}");
    assert!(ask.get("error").is_none(), "{ask:#}");
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the finally node ran once, reading `cancelled` and no `execution.error`"
    );
}

/// **A restart in the middle of a cancel at a `human` pause completes it**
/// (PRD resolved q75 ruling e, `docs/durability.md` §6.1).
///
/// The journal is left as a crash right after the cancel route recorded its
/// request would leave it: the request on the `cancellations` table, the pause
/// unanswered — an abandoned pause writes nothing. The recovered replay reaches
/// the pause's claim, the first effect the record does not hold, and the
/// cancel fires there instead of the wait being planted again: the row settles
/// `cancelled`, no question is published, and the finally node runs once.
#[test]
fn a_restart_in_the_middle_of_a_cancel_at_a_human_pause_completes_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-restart-pause");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-restart-pause") else {
        return;
    };
    let (execution, resume_url) = {
        let Some(mut served) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&served.base_url).expect("a client for the generated app");
        let execution = started(&app, "/asking", &json!({ "path": "release.dmg" }));
        let resume_url = parked_at(&app, &execution);
        served.stop();
        (execution, resume_url)
    };
    let wrote = harness::journal_sql(
        &project,
        &format!(
            "INSERT INTO cancellations (execution, requested_at) \
             VALUES ('{execution}', '2026-11-04T00:00:00.000Z');"
        ),
    );
    assert!(
        wrote.status.success(),
        "{}",
        String::from_utf8_lossy(&wrote.stderr)
    );

    let Some(recovered) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&recovered.base_url).expect("a client for the recovered app");
    let report = reaches(&app, &execution, "cancelled", Duration::from_secs(30));
    released_and_refused(&app, &execution, &resume_url, &report);
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the finally node ran once, at `cancelled`"
    );
}

/// **`agent-compose cancel` exits by what the app answered** (Decision D160):
/// `0` on `202`, `1` on a refusal — here `409` for an execution that has
/// already ended — and `2` where it could not ask at all.
#[test]
fn agent_compose_cancel_exits_by_what_the_app_answered() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-cli");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-cli") else {
        return;
    };
    let Some(cli_out) = harness::scratch_project("cancel-cli-build") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    pids(&log, 1);

    let cancelling = |url: &str, format: &str| {
        compiler(&environment)
            .arg("cancel")
            .arg(harness::fixture(FIXTURE))
            .arg(&execution)
            .args(["--url", url, "--format", format])
            .arg("--out")
            .arg(&cli_out)
            .output()
            .expect("the compiler runs")
    };

    let accepted = cancelling(&served.base_url, "json");
    assert_eq!(
        accepted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let document: Value =
        serde_json::from_slice(&accepted.stdout).expect("`--format json` prints one document");
    assert_eq!(document["http_status"], 202, "{document:#}");
    assert_eq!(document["cancelling"], true, "{document:#}");
    reaches(&app, &execution, "cancelled", SETTLE);

    let refused = cancelling(&served.base_url, "human");
    assert_eq!(
        refused.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(
        String::from_utf8_lossy(&refused.stderr).contains("(409)"),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );

    let unreachable = cancelling("http://127.0.0.1:1", "human");
    assert_eq!(
        unreachable.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&unreachable.stderr)
    );
}

/// **`serve --no-build` and `cancel --no-build` launch a built tree as it
/// stands** (PRD resolved q87, Decision D170): the fixture is built once, and
/// then served and cancelled out of that tree with a spec path that names no
/// file at all — so neither command read a spec, and neither built one.
///
/// The cancel is the real thing rather than a refusal: the execution started
/// on the served tree reaches `cancelled`, so the `cancel` verb launched out of
/// the tree (the emitted project's own) sent the route its request, exit `0`
/// and the `202`. The tree is the one directory both commands launch out of,
/// and it is left as the build wrote it — `--no-build` writes nothing into it.
#[test]
fn serve_and_cancel_without_a_build_launch_the_built_tree_as_it_stands() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-no-build");
    let (environment, log) = environment(&provider, &scratch);
    let Some((project, built)) = harness::build_under_toolchain(FIXTURE, "cancel-no-build") else {
        return;
    };
    let _cleanup = harness::Scratch::at(project.clone());
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let stamp = std::fs::read(project.join("src/artifact.ts")).expect("the build wrote a stamp");
    let graph = std::fs::read(project.join("src/graph.ts")).expect("the build wrote a graph");

    let Some(served) = harness::serve_as_built(&project, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    pids(&log, 1);

    let nowhere = project.join("no-spec-is-read-here").join("main.yml");
    let accepted = compiler(&environment)
        .arg("cancel")
        .arg(&nowhere)
        .arg(&execution)
        .args(["--url", &served.base_url, "--format", "json", "--no-build"])
        .arg("--out")
        .arg(&project)
        .output()
        .expect("the compiler runs");
    assert_eq!(
        accepted.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&accepted.stderr)
    );
    let document: Value =
        serde_json::from_slice(&accepted.stdout).expect("`--format json` prints one document");
    assert_eq!(document["http_status"], 202, "{document:#}");
    reaches(&app, &execution, "cancelled", SETTLE);

    assert_eq!(
        std::fs::read(project.join("src/artifact.ts")).expect("readable"),
        stamp,
        "the tree's stamp is the build's"
    );
    assert_eq!(
        std::fs::read(project.join("src/graph.ts")).expect("readable"),
        graph,
        "…and so is everything it stamps"
    );
    assert!(
        !nowhere.exists(),
        "nothing was written where the spec path pointed"
    );
}

/// **`agent-compose cancel` sends the cancel to the target's `hub.public_url`
/// when no `--url` is given, and is a usage error where the target declares
/// none** (PRD resolved q75 ruling d: "`--url`, defaulting to
/// `hub.public_url`"; Decision D160).
///
/// A literal `public_url:` cannot name a port the operating system chose, so
/// the fixture is staged with a `deploy/local.yml` whose `hub:` names the
/// served app — `local` may declare `hub:` (grammar §14), and it is the one
/// target this fixture's detached dispatches compile under. Under it, a cancel
/// with no `--url` reaches that app: exit `0`, the `202`, and the execution
/// settles `cancelled`. An id the app does not know is the route's `404`, a
/// refusal: exit `1`, carrying the status. And the fixture as committed, whose
/// `local` declares no `hub:`, has nowhere to send it: exit `2`, with the
/// sentence that says to pass `--url`.
#[test]
fn agent_compose_cancel_finds_the_app_at_the_targets_public_url() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-cli-public-url");
    let (environment, log) = environment(&provider, &scratch);
    let Some(project) = harness::scratch_project("cancel-cli-public-url") else {
        return;
    };
    let Some(staged_out) = harness::scratch_project("cancel-cli-public-url-staged") else {
        return;
    };
    let Some(committed_out) = harness::scratch_project("cancel-cli-public-url-committed") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let execution = started(&app, "/long", &json!({ "linger": 600 }));
    pids(&log, 1);

    let (_staged, entrypoint) = harness::staged_with_deploy(
        "cancel-cli-public-url",
        FIXTURE,
        "local",
        &format!(
            "version: \"0.1\"\n\nhub:\n  public_url: \"{}\"\n",
            served.base_url
        ),
    );
    let cancelling = |entrypoint: &Path, out: &Path, id: &str| {
        compiler(&environment)
            .arg("cancel")
            .arg(entrypoint)
            .arg(id)
            .args(["--target", "local", "--format", "json"])
            .arg("--out")
            .arg(out)
            .output()
            .expect("the compiler runs")
    };

    let accepted = cancelling(&entrypoint, &staged_out, &execution);
    assert_eq!(
        accepted.status.code(),
        Some(0),
        "a cancel with no `--url` did not reach the app at the target's `hub.public_url`: {}{}",
        String::from_utf8_lossy(&accepted.stdout),
        String::from_utf8_lossy(&accepted.stderr)
    );
    let document: Value =
        serde_json::from_slice(&accepted.stdout).expect("`--format json` prints one document");
    assert_eq!(document["http_status"], 202, "{document:#}");
    reaches(&app, &execution, "cancelled", SETTLE);

    let unknown = cancelling(&entrypoint, &staged_out, "exec_nobody_started");
    assert_eq!(
        unknown.status.code(),
        Some(1),
        "an id the app does not know is a refusal: {}",
        String::from_utf8_lossy(&unknown.stderr)
    );
    let document: Value =
        serde_json::from_slice(&unknown.stdout).expect("`--format json` prints one document");
    assert_eq!(document["http_status"], 404, "{document:#}");

    let nowhere = cancelling(&harness::fixture(FIXTURE), &committed_out, &execution);
    assert_eq!(
        nowhere.status.code(),
        Some(2),
        "a target with no `hub.public_url` and no `--url` is a usage error: {}",
        String::from_utf8_lossy(&nowhere.stderr)
    );
    let said = String::from_utf8_lossy(&nowhere.stderr);
    assert!(
        said.contains("this target declares no `hub.public_url` to find it at: pass `--url"),
        "{said}"
    );
}

/// **An interrupted `run` is left open for `resume`, not cancelled** (PRD
/// resolved q28, read at its own words by Decision D160).
///
/// A cancel is `serve`'s. `Ctrl-C` at a `run` stops the process and leaves its
/// lifecycle row open, so `resume` carries the execution on: here the resumed
/// generation reaches the command the interrupted one was running and starts it
/// again — the second pid — which a row closed `cancelled` would have refused.
#[test]
fn an_interrupted_run_is_left_open_for_resume_rather_than_cancelled() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-run-posture");
    let (environment, log) = environment(&provider, &scratch);
    let Some((project, built)) = harness::build_under_toolchain(FIXTURE, "cancel-run-posture")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let stopped = harness::stop_run(
        &project,
        &["run", "flow.long", "--input", "linger=600"],
        &environment,
        libc::SIGINT,
        || pid_file.exists(),
    );
    assert!(stopped.status.is_some(), "the run stopped on `SIGINT`");
    let execution = child_of(&project, "").expect("the interrupted run left a lifecycle row");
    assert!(
        ended(&log)
            .iter()
            .all(|line| !line.starts_with("cancelled")),
        "an interrupted run is not a cancelled one: {:?}",
        ended(&log)
    );

    let killed = harness::crash_run(&project, &["resume", &execution], &environment, |_| {
        std::fs::read_to_string(&pid_file).is_ok_and(|held| held.lines().count() >= 2)
    });
    assert_eq!(killed.execution, execution, "{}", killed.stderr);
}

/// **A cancel closes an execution this build cannot run, and the open child it
/// left** (PRD resolved q81, Decision D165).
///
/// `flow.parent` detaches a ten-minute `flow.long` child and runs a ten-minute
/// command of its own when its `serve` is killed, so both rows are open. The
/// next build renames `flow.parent`: recovery leaves the parent's row in the
/// journal on no board, and the child — whose recovery waits for its open
/// parent to re-issue it — is on no board either, since nothing will. Neither
/// is answered by the status route.
///
/// The cancel of the parent closes both: `202` with the cancel complete, and
/// two rows ended `cancelled` with their requests recorded and `finally_error`
/// saying no finally node ran — the parent's naming the flow this build lacks,
/// the child's that its open parent is one this build's recovery could not
/// take, so nothing here re-issues it: the child's flow is declared, but no
/// recovery of this build will ever run it. No finally node ran
/// anywhere. Both are on the status route now, and a cancel of the child is
/// the `409` every ended execution gets.
#[test]
fn a_cancel_closes_an_execution_this_build_cannot_run_and_the_open_child_it_left() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-unrun-lineage");
    let (environment, log) = environment(&provider, &scratch);
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let Some(project) = harness::scratch_project("cancel-unrun-lineage") else {
        return;
    };
    let parent;
    {
        let Some(first) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        parent = started(
            &app,
            "/parent",
            &json!({ "child_lingers": [600], "linger": 600 }),
        );
        pids(&log, 2);
        // Dropped holding both: `SIGKILL` to the group, both rows left open.
    }
    let child = child_row_of(&project, &parent).expect("the parent's child has a row");

    let entrypoint = scratch.path().join("renamed.yml");
    std::fs::write(&entrypoint, renamed("flow.parent", "flow.parented"))
        .expect("the scratch area is writable");
    let Some(mut served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
    else {
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    for id in [&parent, &child] {
        let unknown = app
            .get(&format!("/executions/{id}"))
            .expect("the status route answers");
        assert_eq!(
            unknown.status,
            404,
            "{id} is on no board: {}",
            unknown.text()
        );
    }

    let (status, body) = cancel(&app, &parent);
    assert_eq!(status, 202, "{body:#}");
    assert_eq!(body["status"], "cancelled", "{body:#}");
    assert_eq!(body["cancelling"], false, "{body:#}");
    assert_eq!(
        body["status_url"],
        format!("/executions/{parent}"),
        "{body:#}"
    );

    let mut expected = vec![
        json!({
            "id": &child,
            "status": "cancelled",
            "error": null,
            "finally_error": closed_detail(&format!(
                "its open parent `{parent}` is one this build's recovery could not take, so \
                 nothing here re-issues it"
            )),
            "requested": 1,
        }),
        json!({
            "id": &parent,
            "status": "cancelled",
            "error": null,
            "finally_error": closed_detail("this build declares no flow `flow.parent`"),
            "requested": 1,
        }),
    ];
    expected.sort_by(|one, other| one["id"].as_str().cmp(&other["id"].as_str()));
    assert_eq!(
        closed_rows(&project),
        Value::Array(expected),
        "both rows closed `cancelled`, their requests recorded, saying why"
    );
    assert!(
        ended(&log).is_empty(),
        "no finally node ran: {:?}",
        ended(&log)
    );
    let child_report: Value = app
        .get(&format!("/executions/{child}"))
        .expect("the status route answers")
        .json();
    assert_eq!(child_report["status"], "cancelled", "{child_report:#}");
    let (again, refused) = cancel(&app, &child);
    assert_eq!(again, 409, "{refused:#}");

    let said = served.stop_and_read_stderr();
    kill_listed(&pid_file);
    assert_eq!(
        said.lines()
            .filter(|line| line.starts_with(&format!(
                "`{parent}` (flow.parent) could not be recovered by this build: "
            )))
            .count(),
        1,
        "{said}"
    );
}

/// **`agent-compose cancel` prints why it closed an execution the serving
/// build cannot run** (PRD resolved q81, Decision D165).
///
/// `flow.long` is running a ten-minute command when its `serve` is killed, and
/// the next build renames the flow. `agent-compose cancel` against that build
/// exits `0` and prints the route's `detail` — the row closed `cancelled`, no
/// finally node run — and the execution is `cancelled` on the status route.
/// Sent again, it is the app's `409`: exit `1`.
#[test]
fn agent_compose_cancel_prints_why_it_closed_an_execution_this_build_cannot_run() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-cli-unrun");
    let (environment, log) = environment(&provider, &scratch);
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let Some(project) = harness::scratch_project("cancel-cli-unrun") else {
        return;
    };
    let Some(cli_out) = harness::scratch_project("cancel-cli-unrun-build") else {
        return;
    };
    let execution;
    {
        let Some(first) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        execution = started(&app, "/long", &json!({ "linger": 600 }));
        pids(&log, 1);
    }
    let entrypoint = scratch.path().join("renamed.yml");
    std::fs::write(&entrypoint, renamed("flow.long", "flow.lasting"))
        .expect("the scratch area is writable");
    let Some(served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment) else {
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let cancelling = |format: &str| {
        compiler(&environment)
            .arg("cancel")
            .arg(&entrypoint)
            .arg(&execution)
            .args(["--url", &served.base_url, "--format", format])
            .arg("--out")
            .arg(&cli_out)
            .output()
            .expect("the compiler runs")
    };

    let closed = cancelling("human");
    kill_listed(&pid_file);
    assert_eq!(
        closed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&closed.stderr)
    );
    let printed = String::from_utf8_lossy(&closed.stdout);
    assert_eq!(
        printed.trim_end(),
        format!(
            "cancelled `{execution}`: this build could not take up this execution's recovery \
             (this build declares no flow `flow.long`), so no generation of it can run here: \
             its lifecycle row is closed `cancelled` and no finally node ran, which its \
             `finally_error` says"
        ),
        "{printed}"
    );
    let report = reaches(&app, &execution, "cancelled", SETTLE);
    assert_eq!(
        report["finally_error"],
        closed_detail("this build declares no flow `flow.long`"),
        "{report:#}"
    );

    let refused = cancelling("json");
    assert_eq!(
        refused.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    let document: Value =
        serde_json::from_slice(&refused.stdout).expect("`--format json` prints one document");
    assert_eq!(document["http_status"], 409, "{document:#}");
    assert_eq!(document["status"], "cancelled", "{document:#}");
}

/// **A cancel of an execution whose recovery diverged cancels the child this
/// process is running through the cascade, and closes the execution after
/// it** (PRD resolved q81, q75 ruling c; Decision D165, D160).
///
/// [`PARENT_WITH_STEP`]'s parent detaches a ten-minute child, runs a quick
/// step, and then a ten-minute command, when its `serve` is killed. The next
/// build gives the step an argument it never had: the parent's replay re-issues
/// its dispatch — so the child is resumed, and runs here again — and then meets
/// the recorded step this build would not make, and diverges. The parent is
/// `failed` on the status route with its row open; the child is `running`.
///
/// The cancel of the parent closes it and leaves the child to the cascade: the
/// child's command stops, its finally node runs at `cancelled` and its row
/// settles `cancelled` on its own — no `finally_error` — while the parent's
/// row is closed `cancelled` with the divergence in its `finally_error`, **after
/// the child's row has closed**. The `202` says the cancel is complete, so by
/// the time it is answered the child is settled too: its report already reads
/// `cancelled`, its finally node has already run, and its row ended no later
/// than its parent's.
#[test]
fn a_cancel_of_an_execution_whose_recovery_diverged_cancels_the_child_it_runs_through_the_cascade()
{
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-unrun-running-child");
    let (environment, log) = environment(&provider, &scratch);
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let before = scratch.path().join("before.yml");
    std::fs::write(&before, PARENT_WITH_STEP).expect("the scratch area is writable");
    let after = scratch.path().join("after.yml");
    let moved = PARENT_WITH_STEP.replace(
        "        command: \"${TALLY_BIN}/ended\"\n      input:\n        outcome: \"'stepped'\"",
        "        command: \"${TALLY_BIN}/ended\"\n        args: [\"--moved\"]\n      input:\n        outcome: \"'stepped'\"",
    );
    assert_ne!(
        moved, PARENT_WITH_STEP,
        "the step's binding is the one that moves"
    );
    std::fs::write(&after, moved).expect("the scratch area is writable");
    let Some(project) = harness::scratch_project("cancel-unrun-running-child") else {
        return;
    };

    let parent;
    {
        let Some(first) = harness::serve_entrypoint_into(&project, &before, &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        parent = started(
            &app,
            "/parent",
            &json!({ "child_lingers": [600], "linger": 600 }),
        );
        pids(&log, 2);
        assert_eq!(
            ended(&log),
            ["stepped|none"],
            "the step ran and was recorded"
        );
    }
    let child = child_row_of(&project, &parent).expect("the parent's child has a row");

    let Some(mut served) = harness::serve_entrypoint_into(&project, &after, &environment) else {
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let diverged = reaches(&app, &parent, "failed", Duration::from_secs(30));
    let reason = diverged["error"].as_str().unwrap_or_default().to_string();
    assert!(reason.contains("ReplayDivergence: "), "{diverged:#}");
    // The child is running here again: its command was the one in flight.
    let rerun = pids(&log, 3)[2];
    reaches(&app, &child, "running", Duration::from_secs(30));

    let (status, body) = cancel(&app, &parent);
    assert_eq!(status, 202, "{body:#}");
    assert_eq!(body["status"], "cancelled", "{body:#}");
    assert_eq!(body["cancelling"], false, "{body:#}");

    // Asked once, not polled: the cancel was complete when it was answered,
    // the child's settle with it.
    let child_report: Value = app
        .get(&format!("/executions/{child}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        child_report["status"], "cancelled",
        "the child had settled before the cancel was answered: {child_report:#}"
    );
    assert!(child_report.get("error").is_none(), "{child_report:#}");
    assert!(
        child_report.get("finally_error").is_none(),
        "the child's own finally node ran, and ran clean: {child_report:#}"
    );
    assert_eq!(
        ended(&log),
        ["stepped|none", "cancelled|none"],
        "the child's finally node had run at `cancelled` before the cancel was answered, \
         and the parent's ran nowhere"
    );
    assert!(!alive(rerun), "the child's command outlived the cascade");

    let rows = harness::journal_rows(
        &project,
        &format!(
            "SELECT executions.id AS id, executions.status AS status, \
             executions.finally_error AS finally_error, executions.ended_at AS ended_at, \
             cancellations.requested_at AS requested_at FROM executions \
             LEFT JOIN cancellations ON cancellations.execution = executions.id \
             WHERE executions.id IN ('{parent}', '{child}')"
        ),
    );
    let row = |id: &str| -> Value {
        rows.as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id}'s row: {rows:#}"))
    };
    let (parent_row, child_row) = (row(&parent), row(&child));
    assert_eq!(parent_row["status"], "cancelled", "{rows:#}");
    let detail = parent_row["finally_error"].as_str().unwrap_or_default();
    assert!(
        detail.starts_with(
            "no finally node ran: this build could not take up the execution's recovery ("
        ) && detail.contains("ReplayDivergence: "),
        "{rows:#}"
    );
    assert_eq!(child_row["status"], "cancelled", "{rows:#}");
    let child_requested = child_row["requested_at"]
        .as_str()
        .expect("the child's request");
    let child_ended = child_row["ended_at"].as_str().expect("the child's end");
    let parent_ended = parent_row["ended_at"].as_str().expect("the parent's end");
    assert!(
        child_requested <= parent_ended,
        "the child's cancel ({child_requested}) was recorded before the parent's row closed \
         ({parent_ended}): {rows:#}"
    );
    assert!(
        child_ended <= parent_ended,
        "the child's row ({child_ended}) had closed before the parent's did ({parent_ended}): \
         {rows:#}"
    );

    let said = served.stop_and_read_stderr();
    kill_listed(&pid_file);
    assert_eq!(
        said.lines()
            .filter(|line| line.starts_with(&format!(
                "`{parent}` (flow.parent) could not be recovered by this build: "
            )) && line.ends_with(&format!(
                "; its row stays open until `agent-compose cancel {parent}` closes it"
            )))
            .count(),
        1,
        "{said}"
    );
}

/// **A child the cascade reached still waiting for its permit, whose own replay
/// then diverged, is closed before its parent** (PRD resolved q81's "its open
/// children in the lineage closed the same way", q75 ruling c; Decision D165,
/// D160).
///
/// [`PARENT_OF_A_QUEUE`]'s parent detaches two children of `flow.queued` under
/// `max_concurrency: 2`, runs a quick step and then a ten-minute command, when
/// its `serve` is killed: the first child is in its ten-minute `work`, and the
/// second, whose `work` was over at once, has run its quick `step` and is in
/// its ten-minute `hold`. The next build bounds the fan-out at one and gives
/// both steps an argument they never had. The parent's replay re-issues both
/// dispatches: the first child takes the one permit and runs its `work` here
/// again — its record holds nothing it would diverge at — the second is still
/// waiting for the permit, on no board, and the parent diverges at its step.
///
/// The cancel of the parent cancels the first child through its controller,
/// and the second — which this process runs, waiting — has its request
/// recorded. When the first child gives the permit back, the second opens
/// carrying the request, and its replay to the frontier the cancel would fire
/// at meets the recorded step this build would not make: it diverges, and its
/// generation ends with its row still open. That child is then one nothing
/// here will take, and the cancel closes it — `cancelled`, the divergence in
/// its `finally_error` — before the parent's row closes, and the `202` names no
/// descendant left open.
#[test]
fn a_child_the_cascade_reached_waiting_for_its_permit_whose_replay_then_diverged_is_closed_before_its_parent()
 {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-unrun-queued-child");
    let (environment, log) = environment(&provider, &scratch);
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let before = scratch.path().join("before.yml");
    std::fs::write(&before, PARENT_OF_A_QUEUE).expect("the scratch area is writable");
    let after = scratch.path().join("after.yml");
    let moved = PARENT_OF_A_QUEUE
        .replace("max_concurrency: 2", "max_concurrency: 1")
        .replace(
            "        command: \"${TALLY_BIN}/ended\"\n      input:\n        outcome: \"'stepped'\"",
            "        command: \"${TALLY_BIN}/ended\"\n        args: [\"--moved\"]\n      input:\n        outcome: \"'stepped'\"",
        )
        .replace(
            "        command: \"${TALLY_BIN}/ended\"\n      input:\n        outcome: \"'child-stepped'\"",
            "        command: \"${TALLY_BIN}/ended\"\n        args: [\"--moved\"]\n      input:\n        outcome: \"'child-stepped'\"",
        );
    assert_eq!(
        moved.matches("--moved").count(),
        2,
        "both steps' bindings move"
    );
    assert!(
        moved.contains("max_concurrency: 1"),
        "the fan-out's bound moves"
    );
    std::fs::write(&after, moved).expect("the scratch area is writable");
    let Some(project) = harness::scratch_project("cancel-unrun-queued-child") else {
        return;
    };

    let parent;
    {
        let Some(first) = harness::serve_entrypoint_into(&project, &before, &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        parent = started(
            &app,
            "/parent",
            &json!({ "firsts": [600, 0], "linger": 600 }),
        );
        // The first child's `work`, the second's `work` and `hold`, and the
        // parent's `hold`.
        pids(&log, 4);
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let mut said = ended(&log);
            said.sort();
            if said == ["child-stepped|none", "stepped|none"] {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "both steps ran and were recorded: {said:?}"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }
    let children = harness::journal_rows(
        &project,
        &format!(
            "SELECT execution, item_index FROM lineage WHERE parent = '{parent}' \
             ORDER BY item_index"
        ),
    );
    let child = |index: i64| -> String {
        children
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["item_index"] == index)
            .and_then(|row| row["execution"].as_str())
            .unwrap_or_else(|| panic!("the parent's child at {index}: {children:#}"))
            .to_string()
    };
    let (running, waiting) = (child(0), child(1));

    let Some(mut served) = harness::serve_entrypoint_into(&project, &after, &environment) else {
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let diverged = reaches(&app, &parent, "failed", Duration::from_secs(30));
    assert!(
        diverged["error"]
            .as_str()
            .unwrap_or_default()
            .contains("ReplayDivergence: "),
        "{diverged:#}"
    );
    // The first child runs its `work` here again, holding the one permit…
    let rerun = pids(&log, 5)[4];
    reaches(&app, &running, "running", Duration::from_secs(30));
    // …and the second waits for it: issued, on this process's books, and on
    // no board until it runs.
    let queued = app
        .get(&format!("/executions/{waiting}"))
        .expect("the status route answers");
    assert_eq!(
        queued.status,
        404,
        "the second child is still waiting for its permit: {}",
        queued.text()
    );

    let (status, body) = cancel(&app, &parent);
    assert_eq!(status, 202, "{body:#}");
    assert_eq!(body["status"], "cancelled", "{body:#}");
    assert_eq!(body["cancelling"], false, "{body:#}");
    let detail = body["detail"].as_str().unwrap_or_default();
    assert!(
        !detail.contains(&waiting) && !detail.contains("open with the cancel recorded"),
        "no descendant is left open: {body:#}"
    );
    assert!(
        !alive(rerun),
        "the first child's command outlived the cascade"
    );

    // Asked once, not polled: the cancel was complete when it was answered.
    let report = |id: &str| -> Value {
        app.get(&format!("/executions/{id}"))
            .expect("the status route answers")
            .json()
    };
    let first_report = report(&running);
    assert_eq!(first_report["status"], "cancelled", "{first_report:#}");
    assert!(
        first_report.get("finally_error").is_none(),
        "the first child's own finally node ran, and ran clean: {first_report:#}"
    );
    let second_report = report(&waiting);
    assert_eq!(
        second_report["status"], "cancelled",
        "the second child was closed before the cancel was answered: {second_report:#}"
    );
    assert!(second_report.get("error").is_none(), "{second_report:#}");
    let second_detail = second_report["finally_error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        second_detail.starts_with(closed_detail("").trim_end_matches(')'))
            && second_detail.contains("ReplayDivergence: "),
        "{second_report:#}"
    );
    let mut said = ended(&log);
    said.sort();
    assert_eq!(
        said,
        ["cancelled|none", "child-stepped|none", "stepped|none"],
        "the first child's finally node ran at `cancelled`, and neither the second child's \
         nor the parent's ran anywhere"
    );

    let rows = harness::journal_rows(
        &project,
        &format!(
            "SELECT executions.id AS id, executions.status AS status, executions.error AS error, \
             executions.finally_error AS finally_error, executions.ended_at AS ended_at, \
             cancellations.requested_at AS requested_at FROM executions \
             LEFT JOIN cancellations ON cancellations.execution = executions.id \
             WHERE executions.id IN ('{parent}', '{running}', '{waiting}')"
        ),
    );
    let row = |id: &str| -> Value {
        rows.as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == id)
            .cloned()
            .unwrap_or_else(|| panic!("{id}'s row: {rows:#}"))
    };
    let (parent_row, first_row, second_row) = (row(&parent), row(&running), row(&waiting));
    for held in [&parent_row, &first_row, &second_row] {
        assert_eq!(held["status"], "cancelled", "{rows:#}");
        assert!(held["error"].is_null(), "{rows:#}");
        assert!(held["requested_at"].is_string(), "{rows:#}");
    }
    assert!(first_row["finally_error"].is_null(), "{rows:#}");
    assert_eq!(
        second_row["finally_error"].as_str(),
        Some(second_detail.as_str()),
        "{rows:#}"
    );
    assert!(
        parent_row["finally_error"]
            .as_str()
            .unwrap_or_default()
            .contains("ReplayDivergence: "),
        "{rows:#}"
    );
    let parent_ended = parent_row["ended_at"].as_str().expect("the parent's end");
    for held in [&first_row, &second_row] {
        let child_ended = held["ended_at"].as_str().expect("the child's end");
        assert!(
            child_ended <= parent_ended,
            "the child's row ({child_ended}) had closed before the parent's did \
             ({parent_ended}): {rows:#}"
        );
    }

    let said = served.stop_and_read_stderr();
    kill_listed(&pid_file);
    for (id, flow) in [(&parent, "flow.parent"), (&waiting, "flow.queued")] {
        assert_eq!(
            said.lines()
                .filter(|line| line.starts_with(&format!(
                    "`{id}` ({flow}) could not be recovered by this build: "
                )) && line.ends_with(&format!(
                    "; its row stays open until `agent-compose cancel {id}` closes it"
                )))
                .count(),
            1,
            "{said}"
        );
    }
}

/// **`serve`'s cancel leaves alone an execution a `run` beside it is running**
/// (PRD resolved q75 ruling d, q28; Decision D165's "a `run`'s row is still
/// `404`", D163's Status).
///
/// On SQLite a `run` opens the journal beside a `serve` that is up. Here the
/// project's own `run` of `flow.long` is running its ten-minute command while
/// the project is served: its row is open, its flow is declared, it is on no
/// board of the `serve`'s and the `serve` runs no generation of it — every
/// mark of an execution that process has not recovered, whose cancel it would
/// record for the recovery that takes it, save one: its trigger is `manual`.
/// So the cancel is the `404` of an
/// id the app does not know; the row stays open, with no cancel request
/// recorded and no `finally_error`; the status route does not know it; no
/// finally node ran; and the `run` is still running its command.
#[test]
fn serves_cancel_of_an_execution_a_run_beside_it_is_running_is_404_and_leaves_its_row_open() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-run-beside-serve");
    let (environment, log) = environment(&provider, &scratch);
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let Some(project) = harness::scratch_project("cancel-run-beside-serve") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, FIXTURE, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // The project's own `run`, launched as `agent-compose run` launches it.
    let mut command = harness::bun();
    command
        .arg(project.join("src/index.ts"))
        .args(["run", "flow.long", "--input", "linger=600"]);
    harness::seal(&mut command, &environment);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut beside = Beside {
        run: command.spawn().expect("bun runs"),
        pids: pid_file.clone(),
    };
    let lingering = pids(&log, 1)[0];

    let rows = closed_rows(&project);
    let execution = rows
        .as_array()
        .and_then(|rows| rows.first())
        .and_then(|row| row["id"].as_str())
        .unwrap_or_else(|| panic!("the run's lifecycle row: {rows:#}"))
        .to_string();
    let kind = harness::journal_rows(
        &project,
        &format!("SELECT trigger_kind FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        kind,
        json!([{ "trigger_kind": "manual" }]),
        "a `run` writes a `manual` row"
    );

    let (status, body) = cancel(&app, &execution);
    assert_eq!(
        status, 404,
        "a `run`'s execution is not `serve`'s to cancel: {body:#}"
    );
    assert_eq!(
        body["error"],
        format!("no execution `{execution}` was started by this process"),
        "{body:#}"
    );
    assert_eq!(
        closed_rows(&project),
        json!([{
            "id": &execution,
            "status": "open",
            "error": null,
            "finally_error": null,
            "requested": 0,
        }]),
        "the row is the `run`'s, open, with no cancel request recorded"
    );
    let unknown = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers");
    assert_eq!(unknown.status, 404, "{}", unknown.text());
    assert!(
        ended(&log).is_empty(),
        "no finally node ran: {:?}",
        ended(&log)
    );
    assert!(
        beside
            .run
            .try_wait()
            .expect("the run can be waited on")
            .is_none(),
        "the `run` is still running"
    );
    assert!(alive(lingering), "the `run`'s command is still running");
}

/// A `run` started beside a `serve`, and the commands it started, ended when
/// the test does, however it ends.
struct Beside {
    run: std::process::Child,
    /// Where the linger shim wrote the pids of the commands the run started,
    /// each in a process group of its own ([`kill_listed`]).
    pids: PathBuf,
}

impl Drop for Beside {
    fn drop(&mut self) {
        let _ = self.run.kill();
        let _ = self.run.wait();
        kill_listed(&self.pids);
    }
}

/// A host for a `serve` whose recovery walk cannot read the open executions
/// until the file `$RECOVERY_GATE` appears: the walk's own read of them is
/// refused while the file is missing — every other read of the journal goes
/// through, a cancel's cascade among them — so the start owes the walk, as a
/// standby's does on a journal another `serve` holds, and pays it on a retry
/// once the file is there.
const RECOVERY_HELD: &str = r#"import process from "node:process";
import { existsSync } from "node:fs";
import { openJournal } from "./src/journal.ts";

await import("./src/index.ts");
const journal = await openJournal();
const openExecutions = journal.openExecutions.bind(journal);
const gate = process.env.RECOVERY_GATE;
journal.openExecutions = async () => {
  // The walk's read and no other: the stack names who is asking.
  if (!existsSync(gate) && (new Error().stack ?? "").includes("recoverExecutions")) {
    throw new Error("the test holds the recovery walk until its gate opens");
  }
  return openExecutions();
};
const { runMain } = await import("./src/cli.ts");
await runMain(process.argv.slice(2));
"#;

/// **A cancel of an execution the recovery this process owes has not reached
/// is recorded, and that recovery completes it, finally node and all** (PRD
/// resolved q81, q75 rulings d and e; Decision D165).
///
/// `flow.long` is running a ten-minute command when its `serve` is killed. The
/// next `serve` of the same build starts with its recovery walk refused — its
/// read of the open executions fails until a gate file appears, the position a
/// standby is in while another `serve` holds its journal — so the execution is
/// open, its flow declared, and on no board: one this build *can* run, whose
/// recovery is still owed. A cancel of it is not a close. `agent-compose
/// cancel` exits `0` and prints the route's `detail` — the cancel recorded, the
/// row left for the recovery that takes it — and the route, asked again,
/// answers `202` with the journal's `open`, `cancelling: true` and no status
/// URL; the row is open with its request recorded and no `finally_error`, and
/// no finally node has run. Once the gate opens the walk recovers it, finds the
/// request and completes the cancel: the row settles `cancelled` with no
/// `error` and no `finally_error`, its finally node having run once at
/// `cancelled`, and a cancel after that is the `409` every ended execution
/// gets. The serve log says it was recovered, and never that it could not be.
#[test]
fn a_cancel_of_an_execution_the_owed_recovery_has_not_reached_is_completed_by_that_recovery() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-owed-recovery");
    let (mut environment, log) = environment(&provider, &scratch);
    let gate = scratch.path().join("recovery-gate");
    environment.push(("RECOVERY_GATE".to_string(), gate.display().to_string()));
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let Some(project) = harness::scratch_project("cancel-owed-recovery") else {
        return;
    };
    let Some(cli_out) = harness::scratch_project("cancel-owed-recovery-build") else {
        return;
    };
    let execution;
    {
        let Some(first) = harness::serve_into(&project, FIXTURE, &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        execution = started(&app, "/long", &json!({ "linger": 600 }));
        pids(&log, 1);
        // Dropped holding it: `SIGKILL` to the group, the row left open.
    }
    let Some(mut served) = harness::serve_built_hosted(&project, RECOVERY_HELD, &environment)
    else {
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let unknown = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers");
    assert_eq!(
        unknown.status,
        404,
        "the walk that would put it on the board is owed: {}",
        unknown.text()
    );

    let detail = "this process runs no generation of this execution yet, and a recovery of this \
                  build takes it up — the recovery this process owes, which has not read the \
                  journal yet — so the cancel is recorded and its lifecycle row stays open: that \
                  recovery completes the cancel, its finally node included";
    let printed = compiler(&environment)
        .arg("cancel")
        .arg(harness::fixture(FIXTURE))
        .arg(&execution)
        .args(["--url", &served.base_url])
        .arg("--out")
        .arg(&cli_out)
        .output()
        .expect("the compiler runs");
    assert_eq!(
        printed.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&printed.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&printed.stdout).trim_end(),
        format!("cancelling `{execution}`: {detail}"),
    );
    let (status, body) = cancel(&app, &execution);
    assert_eq!(status, 202, "the same request again: {body:#}");
    assert_eq!(body["status"], "open", "{body:#}");
    assert_eq!(body["cancelling"], true, "{body:#}");
    assert!(
        body.get("status_url").is_none(),
        "no route of this process reports it yet: {body:#}"
    );
    assert_eq!(body["detail"], detail, "{body:#}");
    assert_eq!(
        closed_rows(&project),
        json!([{
            "id": &execution,
            "status": "open",
            "error": null,
            "finally_error": null,
            "requested": 1,
        }]),
        "the cancel is recorded, and nothing closed the row"
    );
    assert!(
        ended(&log).is_empty(),
        "no finally node ran: {:?}",
        ended(&log)
    );

    std::fs::write(&gate, "").expect("the scratch area is writable");
    let report = reaches(&app, &execution, "cancelled", Duration::from_secs(60));
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        report.get("finally_error").is_none(),
        "its finally node ran, and ran clean: {report:#}"
    );
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the recovery completed the cancel, running the finally node once at `cancelled`"
    );
    let rows = eventually_rows(&project, &execution, "cancelled");
    assert_eq!(
        rows,
        json!([{
            "id": &execution,
            "status": "cancelled",
            "error": null,
            "finally_error": null,
            "requested": 1,
        }]),
    );
    let (again, refused) = cancel(&app, &execution);
    assert_eq!(again, 409, "{refused:#}");
    assert_eq!(refused["status"], "cancelled", "{refused:#}");

    let said = served.stop_and_read_stderr();
    kill_listed(&pid_file);
    assert_eq!(
        said.lines()
            .filter(|line| *line == format!("recovered {execution} (flow.long)"))
            .count(),
        1,
        "{said}"
    );
    assert!(
        !said.contains(&format!("`{execution}` (flow.long) could not be recovered")),
        "{said}"
    );
}

/// The lifecycle rows once `execution`'s reads `status`, within ten seconds:
/// a settle the status route already reports ends its row a moment after.
fn eventually_rows(project: &Path, execution: &str, status: &str) -> Value {
    let deadline = Instant::now() + SETTLE;
    loop {
        let rows = closed_rows(project);
        let row = rows
            .as_array()
            .into_iter()
            .flatten()
            .find(|row| row["id"] == execution);
        if row.is_some_and(|row| row["status"] == status) {
            return rows;
        }
        assert!(
            Instant::now() < deadline,
            "`{execution}`'s row did not reach `{status}`: {rows:#}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// [`a_standby_recovers_an_execution_before_cancelling_it_and_its_finally_node_runs`]'s
/// composition: `cancellation`'s `flow.long` alone, under a name each run
/// replaces `flow.standby` with — the fixture's detached dispatches are
/// refused under a durably checkpointed target such as a dialled journal's
/// (grammar 8.6 rule 7).
const STANDBY_COMPOSITION: &str = r#"version: "0.1"

defaults:
  timeout: 300s

triggers:
  on_long:
    type: http
    flow: flow.standby
    path: /long
    method: POST
    input:
      linger: "payload.body.linger"
    respond: async

flow.standby:
  description: A command that runs as long as it is told to, and a record of how the run ended.
  inputs:
    linger:
      type: integer
  outputs: {}
  nodes:
    work:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "input.linger"
    record:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : 'none'"
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: record
"#;

/// **A standby's cancel of an execution the `serve` it takes over from left
/// running recovers it first, and cancels it as a running one** (PRD resolved
/// q81, q75 rulings a and b; Decision D165; `docs/durability.md` §6.1).
///
/// The probe that found the case, as a test: a `serve` on a Postgres journal
/// runs [`STANDBY_COMPOSITION`]'s ten-minute command, and a second `serve` over the same
/// journal is its standby — its start cannot read the journal the first
/// holds, so it owes the recovery, retrying with backoff. The first is
/// `SIGKILL`ed a few seconds on, between two of those retries, and the cancel
/// is sent to the standby the moment it can read the journal: well before its
/// next retry. That cancel takes the recovery it owes first, so the execution
/// is on its board and running when the cancel is decided — `202` with
/// `cancelling: true`, not a close — and it settles `cancelled` with its
/// finally node run once at `cancelled` and no `finally_error`, on the status
/// route and on its row. A close would have ended the row with no finally node
/// run, an execution this build can run among the ones it could not take.
/// The flow is renamed per run, so no open row an earlier run left on the
/// shared database is one this build declares. Skipped, with a notice, where
/// `AGENT_COMPOSE_TEST_POSTGRES_URL` is unset.
#[test]
fn a_standby_recovers_an_execution_before_cancelling_it_and_its_finally_node_runs() {
    let Ok(url) = std::env::var("AGENT_COMPOSE_TEST_POSTGRES_URL") else {
        eprintln!(
            "notice: skipping the standby cancel case: AGENT_COMPOSE_TEST_POSTGRES_URL is unset"
        );
        return;
    };
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("cancel-standby");
    let (mut environment, log) = environment(&provider, &scratch);
    environment.push(("AGENT_COMPOSE_JOURNAL_URL".to_string(), url.clone()));
    let pid_file = PathBuf::from(format!("{}.pids", log.display()));
    let flow = format!(
        "flow.long_standby_{}_{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("after the epoch")
            .as_millis()
    );
    let composition = scratch.path().join("composition");
    std::fs::create_dir_all(composition.join("deploy")).expect("the scratch area is writable");
    let entrypoint = composition.join("main.yml");
    std::fs::write(
        &entrypoint,
        STANDBY_COMPOSITION.replace("flow.standby", &flow),
    )
    .expect("the scratch area is writable");
    std::fs::write(
        composition.join("deploy/pg.yml"),
        "version: \"0.1\"\njournal:\n  provider: postgres\n  url: ${AGENT_COMPOSE_JOURNAL_URL}\n",
    )
    .expect("the scratch area is writable");
    let (Some(one), Some(two)) = (
        harness::scratch_project("cancel-standby-one"),
        harness::scratch_project("cancel-standby-two"),
    ) else {
        return;
    };
    let Some(mut first) = harness::serve_target_into(&one, &entrypoint, "pg", &environment) else {
        return;
    };
    let execution = started(
        &Client::new(&first.base_url).expect("a client for the generated app"),
        "/long",
        &json!({ "linger": 600 }),
    );
    pids(&log, 1);
    let Some(mut second) = harness::serve_target_into(&two, &entrypoint, "pg", &environment) else {
        first.stop();
        kill_listed(&pid_file);
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");
    // The standby's retries come one, three and seven seconds after its start,
    // and the next at fifteen: the first goes at eight, and the cancel lands
    // in the window before that next retry.
    std::thread::sleep(Duration::from_secs(8));
    first.stop();
    let deadline = Instant::now() + Duration::from_secs(30);
    let body = loop {
        let (status, body) = cancel(&app, &execution);
        if status != 404 {
            assert_eq!(status, 202, "{body:#}");
            break body;
        }
        // Not the standby's to read until the server has let the first's
        // session go.
        assert!(
            Instant::now() < deadline,
            "the standby never read the journal the first left: {body:#}"
        );
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(
        body["cancelling"], true,
        "a running execution's cancel, not a close: {body:#}"
    );
    assert_eq!(
        body["status_url"],
        format!("/executions/{execution}"),
        "{body:#}"
    );

    let report = reaches(&app, &execution, "cancelled", Duration::from_secs(30));
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        report.get("finally_error").is_none(),
        "its finally node ran, and ran clean: {report:#}"
    );
    assert_eq!(
        ended(&log),
        ["cancelled|none"],
        "the finally node ran once, at `cancelled`"
    );
    let deadline = Instant::now() + SETTLE;
    let row = loop {
        let rows = harness::postgres_rows(
            &two,
            &url,
            &format!(
                "SELECT executions.status AS status, executions.error AS error, \
                 executions.finally_error AS finally_error, \
                 (SELECT COUNT(*) FROM cancellations WHERE cancellations.execution = executions.id) \
                 AS requested FROM executions WHERE executions.id = '{execution}'"
            ),
        );
        let row = rows[0].clone();
        if row["status"] == "cancelled" || Instant::now() > deadline {
            break row;
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    assert_eq!(row["status"], "cancelled", "{row:#}");
    assert!(row["error"].is_null(), "{row:#}");
    assert!(
        row["finally_error"].is_null(),
        "no close wrote one: {row:#}"
    );
    // Postgres answers a `COUNT(*)` as a `bigint`, which the driver hands over
    // as a string.
    assert_eq!(
        row["requested"].to_string().trim_matches('"'),
        "1",
        "the request is recorded once: {row:#}"
    );

    let said = second.stop_and_read_stderr();
    kill_listed(&pid_file);
    assert!(
        said.lines()
            .any(|line| line == format!("recovered {execution} ({flow})")),
        "the standby recovered it before the cancel was decided: {said}"
    );
    assert!(
        !said.contains(&format!("`{execution}` ({flow}) could not be recovered")),
        "{said}"
    );
}

/// [`a_cancel_of_an_execution_whose_recovery_diverged_cancels_the_child_it_runs_through_the_cascade`]'s
/// composition: `cancellation`'s `flow.long`, and a parent that detaches one,
/// runs a quick step of its own — `${TALLY_BIN}/ended`, recording `stepped` —
/// and then a command that lingers. The step is the effect a moved build
/// diverges at, after the dispatch its replay re-issues.
const PARENT_WITH_STEP: &str = r#"version: "0.1"

defaults:
  timeout: 300s

triggers:
  on_parent:
    type: http
    flow: flow.parent
    path: /parent
    method: POST
    input:
      child_lingers: "payload.body.child_lingers"
      linger: "payload.body.linger"
    respond: async

flow.long:
  description: A command that runs as long as it is told to, and a record of how the run ended.
  inputs:
    linger:
      type: integer
  outputs: {}
  nodes:
    work:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "input.linger"
    record:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : 'none'"
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: record

flow.parent:
  description: A detached child that lingers, a quick step, then a command of the parent's own.
  inputs:
    child_lingers:
      type: array
      max_items: 1
      items: { type: integer }
    linger:
      type: integer
  outputs: {}
  nodes:
    spawn:
      map:
        over: input.child_lingers
        as: seconds
        node: flow.long
        max_concurrency: 1
        detach: true
        input:
          linger: "seconds"
    step:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "'stepped'"
        error: "'none'"
    hold:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "input.linger"
    parent_end:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : 'none'"
  edges:
    - { from: start, to: spawn }
    - { from: spawn, to: step }
    - { from: step, to: hold }
    - { from: hold, to: end }
  finally: parent_end
"#;

/// [`a_child_the_cascade_reached_waiting_for_its_permit_whose_replay_then_diverged_is_closed_before_its_parent`]'s
/// composition: a parent that detaches a fan-out of `flow.queued` under
/// `max_concurrency: 2`, runs a quick step of its own — `${TALLY_BIN}/ended`,
/// recording `stepped` — and then a command that lingers. Each child lingers
/// for as long as its item says in `work`, records `child-stepped` in a quick
/// `step`, and then lingers ten minutes in `hold`. The steps are the effects a
/// moved build diverges at.
const PARENT_OF_A_QUEUE: &str = r#"version: "0.1"

defaults:
  timeout: 300s

triggers:
  on_parent:
    type: http
    flow: flow.parent
    path: /parent
    method: POST
    input:
      firsts: "payload.body.firsts"
      linger: "payload.body.linger"
    respond: async

flow.queued:
  description: A command that lingers as long as it is told to, a quick step, then a ten-minute command.
  inputs:
    first:
      type: integer
  outputs: {}
  nodes:
    work:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "input.first"
    step:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "'child-stepped'"
        error: "'none'"
    hold:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "600"
    record:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : 'none'"
  edges:
    - { from: start, to: work }
    - { from: work, to: step }
    - { from: step, to: hold }
    - { from: hold, to: end }
  finally: record

flow.parent:
  description: A bounded fan-out of detached children, a quick step, then a command of the parent's own.
  inputs:
    firsts:
      type: array
      max_items: 2
      items: { type: integer }
    linger:
      type: integer
  outputs: {}
  nodes:
    spawn:
      map:
        over: input.firsts
        as: seconds
        node: flow.queued
        max_concurrency: 2
        detach: true
        input:
          first: "seconds"
    step:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "'stepped'"
        error: "'none'"
    hold:
      exec:
        command: "${TALLY_BIN}/linger"
      input:
        linger: "input.linger"
    parent_end:
      exec:
        command: "${TALLY_BIN}/ended"
      input:
        outcome: "execution.outcome"
        error: "has(execution.error) ? execution.error : 'none'"
  edges:
    - { from: start, to: spawn }
    - { from: spawn, to: step }
    - { from: step, to: hold }
    - { from: hold, to: end }
  finally: parent_end
"#;

/// The fixture with one flow renamed, everything naming it following: a build
/// that no longer declares the flow an open execution ran.
fn renamed(flow: &str, to: &str) -> String {
    let source = std::fs::read_to_string(harness::fixture(FIXTURE)).expect("the fixture");
    let edited = source
        .replace(&format!("\n{flow}:\n"), &format!("\n{to}:\n"))
        .replace(&format!(" {flow}\n"), &format!(" {to}\n"));
    assert!(
        !edited.contains(&format!("\n{flow}:\n")),
        "`{flow}` is no longer declared"
    );
    edited
}

/// The child execution `parent`'s detached dispatch started, off its lineage
/// row.
fn child_row_of(project: &Path, parent: &str) -> Option<String> {
    harness::journal_rows(
        project,
        &format!("SELECT execution FROM lineage WHERE parent = '{parent}'"),
    )
    .as_array()?
    .first()?["execution"]
        .as_str()
        .map(str::to_string)
}

/// Every lifecycle row, with what a close writes on it, by id.
fn closed_rows(project: &Path) -> Value {
    harness::journal_rows(
        project,
        "SELECT executions.id AS id, executions.status AS status, executions.error AS error, \
         executions.finally_error AS finally_error, \
         (SELECT COUNT(*) FROM cancellations WHERE cancellations.execution = executions.id) \
         AS requested FROM executions ORDER BY executions.id",
    )
}

/// What a closed execution's `finally_error` says (PRD resolved q81).
fn closed_detail(reason: &str) -> String {
    format!("no finally node ran: this build could not take up the execution's recovery ({reason})")
}
