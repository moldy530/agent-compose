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
//!   runs on a cancelled execution —
//!   [`a_finally_nodes_detached_delivery_runs_on_a_cancelled_execution`];
//! * `agent-compose cancel`'s exit codes —
//!   [`agent_compose_cancel_exits_by_what_the_app_answered`];
//! * and `run`'s posture, which a cancel does not change: an interrupted run's
//!   row stays open for `resume` (PRD resolved q28) —
//!   [`an_interrupted_run_is_left_open_for_resume_rather_than_cancelled`].

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::Command;
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
