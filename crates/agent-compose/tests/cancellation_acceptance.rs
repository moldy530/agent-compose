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
//! * the cascade over a lineage, and a child cancelled alone —
//!   [`a_cancel_of_a_parent_cascades_to_the_child_it_started`] and
//!   [`a_child_cancelled_alone_leaves_its_parent_running_to_its_own_end`];
//! * `resume` refuses a cancelled execution, at the route and at the command
//!   line, naming the status —
//!   [`a_cancelled_execution_is_refused_by_resume_at_the_route_and_the_command_line`];
//! * a restart between the `202` and the settled row completes the cancel and
//!   issues nothing new —
//!   [`a_restart_in_the_middle_of_a_cancel_completes_it_and_issues_nothing_new`];
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
        // process group of its own (grammar 5.5, Decision D124), so the kill
        // above does not reach it, and a crash that left it alive to finish
        // would not be the crash this test is about.
        let slow = PathBuf::from(format!("{}.slow", log.display()));
        for pid in std::fs::read_to_string(&slow)
            .unwrap_or_default()
            .lines()
            .filter_map(|line| line.trim().parse::<i32>().ok())
        {
            // SAFETY: a pid this test's shim wrote; a gone one answers ESRCH.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
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
