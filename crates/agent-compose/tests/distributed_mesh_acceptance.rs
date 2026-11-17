//! A mesh, end to end, in the processes it really runs in
//! (`docs/distributed.md`, PRD resolved q37–q46).
//!
//! Every test here starts a **hub** — `agent-compose serve` over the
//! `placed-nodes` fixture, resolved for its `mesh` target — and one or more
//! **workers**, each a real `agent-compose worker` process. Nothing is stubbed
//! on either side: the worker joins over HTTP, is served the artifact the hub
//! built, materialises it under a data directory of its own, spawns
//! `src/worker-node.ts` per dispatch, streams the effects home and posts the
//! result. The graph really runs, in two processes, over the wire the document
//! fixes.
//!
//! This is the milestone's **acceptance** gate (CLAUDE.md's validation
//! strategy). Its sibling `tests/distributed_hub_wire.rs` is the *conformance*
//! one — it speaks §3 to the hub with an ordinary HTTP client and asserts the
//! document clause by clause — and `tests/distributed_worker_protocol.rs` is the
//! third: the worker's own status handling, against a hub that is a fixture
//! rather than a compiler. The three answer three different questions, and only
//! this one answers "does a distributed execution work".
//!
//! # The timings
//!
//! §2's poll hold and liveness window are 25s and 90s, which no test can wait
//! out; both are configurable and the relationship §2 requires is kept
//! ([`MESH_TIMINGS`]). The window is deliberately several seconds rather than
//! milliseconds: §2's backoff after a transport failure starts at one second, so
//! a window under it would declare a worker gone for one refused connection
//! during a hub restart, and that is the mechanism under test in
//! [`a_hub_restarted_under_its_own_name_is_met_with_a_re_join`] rather than the
//! thing it is asserting.
//!
//! # Why the worker's data directory sits under the toolchain
//!
//! A worker materialises the artifact and then installs its dependencies (§4
//! step 4). Module resolution walks up, so a data directory beneath the suite's
//! installed toolchain resolves the pinned set without a second install — which
//! is what `worker::artifact::install` checks for and skips on. The alternative
//! would be one `bun install` from the network per test.
//!
//! The check is on the **version** and not on the directory: the toolchain
//! fixture pins what the emitter pins
//! (`the_toolchain_fixture_pins_what_the_emitter_pins`), so what is installed
//! above these data directories is the LangGraph the artifact's own
//! `package.json` names, which is the only install §4.1 lets stand in for step
//! 4. A toolchain that drifted off the emitter's pins would run the step here
//! rather than execute a node under a LangGraph this release never pinned.
//!
//! So step 4 is **skipped in this file**, and is checked where it can be run
//! without a network: `worker::artifact`'s own
//! `a_materialised_tree_has_its_dependency_set_installed` materialises a tree
//! outside any install and runs the command in it, and
//! `tests/distributed_worker_protocol.rs` provisions its workers into scratch
//! directories where the same step runs for real.

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider, Outcome, Request, Response, Script, ToolCall};
use serde_json::{Value, json};

/// The wire version this build's hub speaks (§10), read off the compiler rather
/// than written out — see `tests/distributed_hub_wire.rs`'s own copy.
const PROTOCOL: u32 = compose_core::codegen::mesh::PROTOCOL_VERSION;

/// The credential every worker in this file presents.
const TOKEN: &str = "a-join-token-nobody-else-has";

/// The variable the fixture's `hub.join_token:` names.
const TOKEN_VARIABLE: &str = "MESH_JOIN_TOKEN";

/// The fixture, and the target that gives it a mesh.
const FIXTURE: &str = "placed-nodes";
const TARGET: &str = "mesh";

/// The placement the fixture declares, and the one every worker claims.
const PLACEMENT: &str = "mac";

/// The model `agent.signer` routes to.
const SONNET: &str = "claude-sonnet-4-6";

/// A hold and a window short enough for a test and still in §2's relationship.
const MESH_TIMINGS: &[(&str, &str)] = &[
    ("AGENT_COMPOSE_MESH_POLL_HOLD_MS", "400"),
    ("AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS", "4000"),
];

/// How long a test waits for something two processes do between them.
const PATIENCE: Duration = Duration::from_secs(90);

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// One served hub, its scripted provider, and the directory both live in.
struct Mesh {
    served: harness::Served,
    provider: MockProvider,
    /// The built project — and its journal — which outlive a restart.
    project: PathBuf,
    /// Removed when the test ends — or `None` where the test owns the project
    /// and serves it more than once ([`Mesh::start_as_built`]).
    _scratch: Option<harness::Scratch>,
    /// The composition this hub is served from.
    ///
    /// Ordinarily the fixture where it is committed. One test serves a **copy**
    /// with a deploy file of its own — the trace-sink export, whose collector
    /// takes a port the operating system chose and so cannot be named in a
    /// committed target — and a restart has to reach the same composition the
    /// first start did, so the path is held here rather than re-derived.
    entrypoint: PathBuf,
    /// The copy above, removed when the test ends; absent for the fixture.
    _composition: Option<harness::Scratch>,
    client: Client,
    base_url: String,
    /// What every process in this mesh is given.
    environment: Vec<(String, String)>,
}

impl Mesh {
    /// Serve the fixture under its mesh target, with a scripted provider beside
    /// it.
    ///
    /// `None` when the toolchain is not installed, which is the skip every
    /// suite here takes.
    fn start() -> Option<Self> {
        Self::start_from("mesh-acceptance", harness::fixture(FIXTURE), None)
    }

    /// The same, from a composition the caller supplies.
    ///
    /// The one caller that needs it serves a copy of the fixture with an extra
    /// deploy-layer key in its target; everything else serves the fixture where
    /// it is committed, which is what [`Mesh::start`] passes.
    fn start_from(
        purpose: &str,
        entrypoint: PathBuf,
        composition: Option<harness::Scratch>,
    ) -> Option<Self> {
        let project = harness::scratch_project(purpose)?;
        let provider = MockProvider::start().expect("a loopback port");
        let environment = environment(&provider);
        let served = harness::serve_target_into(&project, &entrypoint, TARGET, &environment)?;
        let client = Client::new(&served.base_url)
            .expect("the hub's address parses")
            .with_timeout(Duration::from_secs(30));
        let base_url = served.base_url.clone();
        Some(Self {
            served,
            provider,
            _scratch: Some(harness::Scratch::at(project.clone())),
            project,
            entrypoint,
            _composition: composition,
            client,
            base_url,
            environment,
        })
    }

    /// Serve a tree **already built** into `project` as it stands —
    /// `agent-compose serve --no-build` (PRD resolved q87) — with a scripted
    /// provider beside it.
    ///
    /// The project is the caller's, and outlives this hub: a test that serves
    /// one tree twice, with an edit between, owns the directory both serve.
    fn start_as_built(project: &Path) -> Option<Self> {
        let provider = MockProvider::start().expect("a loopback port");
        let environment = environment(&provider);
        let served = harness::serve_as_built(project, &environment)?;
        let client = Client::new(&served.base_url)
            .expect("the hub's address parses")
            .with_timeout(Duration::from_secs(30));
        let base_url = served.base_url.clone();
        Some(Self {
            served,
            provider,
            _scratch: None,
            project: project.to_path_buf(),
            entrypoint: harness::fixture(FIXTURE),
            _composition: None,
            client,
            base_url,
            environment,
        })
    }

    /// Replace the process behind this hub's name, over the same journal (§5).
    ///
    /// The port is the same, because §5's third rule is that a worker addresses
    /// a hub **name** and replacing the process behind it is invisible to one.
    fn restart(&mut self) {
        let port = self
            .base_url
            .rsplit(':')
            .next()
            .and_then(|port| port.parse::<u16>().ok())
            .expect("the hub's address names a port");
        // The old app **first**, and reaped, or the new one cannot bind the
        // port a worker is still addressing.
        self.served.stop();
        self.served = harness::serve_target_on(
            &self.project,
            &self.entrypoint,
            TARGET,
            port,
            &self.environment,
        )
        .expect("the hub is served again");
    }

    /// [`Mesh::restart`], with `between` run against the project while no
    /// process is serving it — for a test that writes the journal a crash at
    /// that moment would have left.
    fn restart_after(&mut self, between: impl FnOnce(&Path)) {
        let port = self
            .base_url
            .rsplit(':')
            .next()
            .and_then(|port| port.parse::<u16>().ok())
            .expect("the hub's address names a port");
        self.served.stop();
        between(&self.project);
        self.served = harness::serve_target_on(
            &self.project,
            &self.entrypoint,
            TARGET,
            port,
            &self.environment,
        )
        .expect("the hub is served again");
    }

    /// This execution's dispatch rows, as the hub's journal holds them.
    fn dispatch_rows(&self, execution: &str) -> Vec<Value> {
        harness::journal_rows(
            &self.project,
            &format!(
                "SELECT id, status, outcome, payload, detail FROM dispatches \
                 WHERE execution = '{execution}' ORDER BY parked_at"
            ),
        )
        .as_array()
        .cloned()
        .unwrap_or_default()
    }

    /// Wait until every one of this execution's dispatch rows has been
    /// **settled by its worker's cancelled result** (§3.4) — an `error`
    /// outcome named `ExecutionCancelled`, which only that result writes: the
    /// hub stopping waiting leaves a row a worker holds `dispatched` — and
    /// answer the rows.
    fn settled_cancelled(&self, execution: &str, count: usize, within: Duration) -> Vec<Value> {
        let deadline = Instant::now() + within;
        loop {
            let rows = self.dispatch_rows(execution);
            let settled = rows.iter().all(|row| {
                row["status"] == "settled"
                    && row["outcome"] == "error"
                    && row["payload"]
                        .as_str()
                        .is_some_and(|payload| payload.contains("\"ExecutionCancelled\""))
            });
            if rows.len() == count && settled {
                return rows;
            }
            assert!(
                Instant::now() < deadline,
                "the worker never settled this execution's dispatches with its cancelled \
                 result: {rows:#?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// Start one worker against this hub, claiming `PLACEMENT`.
    fn worker(&self, purpose: &str) -> Worker {
        Worker::start(&self.base_url, &self.environment, purpose, None)
    }

    /// The same, into a data directory the caller owns — which a cold start
    /// asserts about.
    fn worker_into(&self, purpose: &str, data_dir: &Path) -> Worker {
        Worker::start(
            &self.base_url,
            &self.environment,
            purpose,
            Some(data_dir.to_path_buf()),
        )
    }

    /// The same again, with `--no-build` on the worker's command line — which
    /// it accepts and which changes nothing, since a worker builds nothing
    /// (PRD resolved q87, Decision D170).
    fn worker_into_without_a_build(&self, purpose: &str, data_dir: &Path) -> Worker {
        Worker::start_with(
            &self.base_url,
            &self.environment,
            purpose,
            Some(data_dir.to_path_buf()),
            &["--no-build"],
        )
    }

    /// Send one request, failing the test on a transport error.
    fn send(&self, request: Request) -> Response {
        self.client
            .send(request)
            .unwrap_or_else(|error| panic!("the hub answered nothing: {error}"))
    }

    /// Start one execution through an `http` trigger, and answer its id.
    fn start_execution(&self, path: &str, body: &Value) -> String {
        let started = self.send(Request::post(path).json(body));
        assert_eq!(started.status, 202, "{}", body_of(&started));
        started.json()["execution_id"]
            .as_str()
            .expect("a start answers an execution id")
            .to_string()
    }

    /// One execution's status report.
    fn report(&self, execution: &str) -> Value {
        let answered = self.send(Request::get(format!("/executions/{execution}")));
        assert_eq!(answered.status, 200, "{}", body_of(&answered));
        answered.json()
    }

    /// Wait until this execution's report satisfies `wanted`, and answer it.
    fn until(&self, execution: &str, what: &str, wanted: impl Fn(&Value) -> bool) -> Value {
        let deadline = Instant::now() + PATIENCE;
        let mut last = Value::Null;
        while Instant::now() < deadline {
            last = self.report(execution);
            if wanted(&last) {
                return last;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("`{execution}` never {what}; its last report was {last:#}");
    }

    /// Wait until this execution is holding a pause, and answer that pause.
    ///
    /// The **hub's** report is what is read, and that is the assertion as much
    /// as the wait: a pause a worker opened is published exactly where a local
    /// one is (`docs/distributed.md` §3.4, PRD resolved q46).
    fn paused(&self, execution: &str) -> Value {
        let report = self.until(execution, "opened a pause", |report| {
            report["interrupts"]
                .as_array()
                .is_some_and(|waits| !waits.is_empty())
                || report["status"] == "failed"
        });
        assert_eq!(
            report["status"], "interrupted",
            "a pause a worker opened did not reach the hub's board: {report:#}"
        );
        report["interrupts"][0].clone()
    }

    /// Answer one pause through the surface a local pause is answered through.
    fn resume(&self, execution: &str, wait: &str, payload: &Value) -> Response {
        self.send(
            Request::post(format!(
                "/executions/{execution}/resume?wait={}",
                urlencoded(wait)
            ))
            .json(payload),
        )
    }

    /// Wait until this execution has completed, and answer its outputs.
    fn completed(&self, execution: &str) -> Value {
        let report = self.until(execution, "completed", |report| {
            report["status"] == "completed" || report["status"] == "failed"
        });
        assert_eq!(report["status"], "completed", "{report:#}");
        report["outputs"].clone()
    }

    /// A worker session held by the **test** rather than by a worker process.
    ///
    /// What an assertion about a status code needs and a real worker cannot
    /// give: a worker acts on §3's answers rather than reporting them.
    fn session(&self) -> String {
        let hash = self.artifact_hash();
        let joined = self.send(
            Request::post("/workers/join")
                .header("authorization", format!("Bearer {TOKEN}"))
                .json(&json!({
                    "protocol": PROTOCOL,
                    "compiler": compose_core::codegen::COMPILER_VERSION,
                    "runtime": "bun 1.2.3",
                    "claims": [PLACEMENT],
                    "artifact_hash": hash,
                    "env_ok": manifest_of(PLACEMENT),
                })),
        );
        assert_eq!(joined.status, 200, "{}", body_of(&joined));
        joined.json()["worker_session"]
            .as_str()
            .expect("the answer issues a session")
            .to_string()
    }

    /// The artifact this hub serves, as its own join answers it.
    fn artifact_hash(&self) -> String {
        let joined = self.send(
            Request::post("/workers/join")
                .header("authorization", format!("Bearer {TOKEN}"))
                .json(&json!({
                    "protocol": PROTOCOL,
                    "compiler": compose_core::codegen::COMPILER_VERSION,
                    "runtime": "bun 1.2.3",
                    "claims": [PLACEMENT],
                })),
        );
        assert_eq!(joined.status, 200, "{}", body_of(&joined));
        joined.json()["artifact"]["hash"]
            .as_str()
            .expect("the answer names the artifact")
            .to_string()
    }
}

/// One `agent-compose worker` process, killed when the test ends.
///
/// The **group**, not the command: a worker spawns `bun src/worker-node.ts` per
/// dispatch, so killing the process alone would leave a node running against a
/// hub the test is about to stop.
struct Worker {
    child: Child,
    reaped: bool,
    /// Where it keeps the artifacts it materialised.
    data_dir: PathBuf,
    /// Everything it has said, collected off a thread.
    said: Arc<Mutex<Vec<String>>>,
}

impl Worker {
    fn start(
        base_url: &str,
        environment: &[(String, String)],
        purpose: &str,
        data_dir: Option<PathBuf>,
    ) -> Self {
        Self::start_with(base_url, environment, purpose, data_dir, &[])
    }

    /// [`Worker::start`], with `extra` flags after the four every worker here
    /// is given.
    fn start_with(
        base_url: &str,
        environment: &[(String, String)],
        purpose: &str,
        data_dir: Option<PathBuf>,
        extra: &[&str],
    ) -> Self {
        let data_dir = data_dir.unwrap_or_else(|| scratch_data_dir(purpose));
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .arg("worker")
            .args(["--hub", base_url])
            .args(["--claim", PLACEMENT])
            .args(["--token-env", TOKEN_VARIABLE])
            .arg("--data-dir")
            .arg(&data_dir)
            .args(extra)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            command.process_group(0);
        }
        harness::seal(&mut command, environment);
        let mut child = command.spawn().expect("the worker starts");
        let said: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        for stream in [
            child.stdout.take().map(Reader::Out),
            child.stderr.take().map(Reader::Err),
        ]
        .into_iter()
        .flatten()
        {
            let collecting = Arc::clone(&said);
            std::thread::spawn(move || {
                let lines: Box<dyn BufRead> = match stream {
                    Reader::Out(held) => Box::new(BufReader::new(held)),
                    Reader::Err(held) => Box::new(BufReader::new(held)),
                };
                for line in lines.lines().map_while(Result::ok) {
                    collecting
                        .lock()
                        .expect("the buffer is not poisoned")
                        .push(line);
                }
            });
        }
        Self {
            child,
            reaped: false,
            data_dir,
            said,
        }
    }

    /// The node runners this worker has running right now — its child
    /// processes running `worker-node` — read off `/proc`. `None` off Linux,
    /// where there is no `/proc` to read them from.
    fn runners(&self) -> Option<Vec<i32>> {
        runners_of(self.child.id())
    }

    /// Everything this worker has written, for a failure message.
    fn transcript(&self) -> String {
        self.said
            .lock()
            .expect("the buffer is not poisoned")
            .join("\n")
    }

    /// Kill this worker and everything it started — the disconnect of §7.3.
    ///
    /// A runner leads a process group of its own, which a kill aimed at the
    /// worker's group does not reach, so each runner's group is killed too:
    /// a machine that died takes its runners with it.
    fn kill(&mut self) {
        self.kill_runners();
        end(&mut self.child);
        self.reaped = true;
    }

    /// `SIGKILL` to every runner's own group, read off `/proc`.
    fn kill_runners(&self) {
        #[cfg(unix)]
        for pid in self.runners().unwrap_or_default() {
            // SAFETY: the group of a runner this worker spawned; `ESRCH` for
            // one already gone.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
    }

    /// Whether this worker is still running.
    ///
    /// The assertion a restart needs: §3.1 makes a refused join terminal, so a
    /// worker that read a `410` — or the `409` its late result meets — as a
    /// refusal would be **gone**, and a mesh that healed on paper would have no
    /// worker left in it.
    fn running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("the worker can be waited on")
            .is_none()
    }

    /// Wait for this worker to exit on its own, and answer how it did.
    fn waited(&mut self) -> std::process::ExitStatus {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("the worker can be waited on") {
                self.reaped = true;
                return status;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let transcript = self.transcript();
        self.kill();
        panic!("the worker never exited; it said:\n{transcript}");
    }
}

enum Reader {
    Out(std::process::ChildStdout),
    Err(std::process::ChildStderr),
}

impl Drop for Worker {
    fn drop(&mut self) {
        if !self.reaped {
            // `SIGINT` first, which the worker hands to its runners' own
            // groups — each sweeping the `exec:` groups it detached — before
            // it ends; then the kill, for whatever is left.
            #[cfg(unix)]
            if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
                // SAFETY: a child this process spawned into a group of its own
                // and has not yet reaped.
                unsafe { libc::kill(-pid, libc::SIGINT) };
                let deadline = Instant::now() + Duration::from_secs(4);
                while Instant::now() < deadline && matches!(self.child.try_wait(), Ok(None)) {
                    std::thread::sleep(Duration::from_millis(20));
                }
            }
            self.kill_runners();
            end(&mut self.child);
        }
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

/// The child processes of `parent` running `worker-node`, off `/proc`.
#[cfg(target_os = "linux")]
fn runners_of(parent: u32) -> Option<Vec<i32>> {
    children_running(parent, "worker-node")
}

/// The child processes of `parent` whose command line holds `needle`, off
/// `/proc`.
#[cfg(target_os = "linux")]
fn children_running(parent: u32, needle: &str) -> Option<Vec<i32>> {
    let mut found = Vec::new();
    for entry in std::fs::read_dir("/proc").ok()?.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|name| name.parse::<i32>().ok())
        else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // `pid (comm) state ppid …`, where `comm` may hold spaces and parens.
        let Some(after) = stat.rfind(')').map(|at| &stat[at + 1..]) else {
            continue;
        };
        let ppid = after
            .split_whitespace()
            .nth(1)
            .and_then(|field| field.parse::<u32>().ok());
        if ppid != Some(parent) {
            continue;
        }
        // A zombie's command line is empty, so a runner that has exited and
        // not yet been reaped is not counted as running.
        let command = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
        if String::from_utf8_lossy(&command).contains(needle) {
            found.push(pid);
        }
    }
    Some(found)
}

#[cfg(not(target_os = "linux"))]
fn runners_of(_parent: u32) -> Option<Vec<i32>> {
    None
}

#[cfg(not(target_os = "linux"))]
fn children_running(_parent: u32, _needle: &str) -> Option<Vec<i32>> {
    None
}

/// Whether a process is gone within `budget`.
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

/// Kill one child **and its group**, and reap it.
fn end(child: &mut Child) {
    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: a child this process spawned into a group of its own and has
        // not yet reaped.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
}

/// A data directory beneath the installed toolchain — see the module header.
fn scratch_data_dir(purpose: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let root = harness::installed().expect("the toolchain is installed");
    let path = root.join("workers").join(format!(
        "{purpose}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&path);
    std::fs::create_dir_all(&path).expect("a worker data directory");
    path
}

/// The environment every process in this mesh is given.
fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    let mut held = harness::environment(provider);
    held.push((TOKEN_VARIABLE.to_string(), TOKEN.to_string()));
    // `mac`'s manifest, which the hub checks a join's `env_ok` against (§9.1):
    // the fixture's `tool.sign` carries it, and it is §9.1's own worked example
    // of a variable that belongs to a placement and not to the hub.
    held.push((
        "KEYCHAIN_PASSWORD".to_string(),
        "the-machine-keychain".to_string(),
    ));
    for (name, value) in MESH_TIMINGS {
        held.push(((*name).to_string(), (*value).to_string()));
    }
    held
}

/// The variables one placement's manifest names, as the artifact carries them.
fn manifest_of(placement: &str) -> Vec<String> {
    let entrypoint = harness::fixture(FIXTURE);
    let ir = compose_core::resolve_with_target(&entrypoint, TARGET)
        .ir
        .expect("the fixture resolves");
    let partition = compose_core::codegen::env::Partition::of(&ir);
    compose_core::codegen::env::References::for_process(
        &ir,
        &partition,
        &compose_core::codegen::env::Process::Placement(placement.to_string()),
    )
    .names()
    .map(str::to_string)
    .collect()
}

/// One wait id, as a query value spells it.
///
/// A wait id is `<instance path>/<ordinal>` and every component is an identifier
/// or a decimal (grammar §9.4), so the slashes are the whole of the escaping —
/// but they are escaped rather than assumed, because the id under test is
/// derived on a worker and this test should fail on what it is asserting.
fn urlencoded(wait: &str) -> String {
    wait.replace('/', "%2F")
}

/// A response body, for a failure message.
fn body_of(response: &Response) -> String {
    format!(
        "{} {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    )
}

/// The three calls `agent.signer` makes: the loop's tool call, the loop's
/// answer, and the pinned structured output that ends it.
///
/// Every one of them is made **on the worker** — the hub never calls a provider
/// for a placed node, which is the point of the fixture (§1: "the heavy work —
/// model calls, tool execution — happens on workers").
fn signing(signature: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "sign",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("signed, and here is the signature")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": signature })),
        ),
    ]
}

/// The same three calls for `agent.stamper`, whose loop calls the **module**
/// tool rather than the `exec:` one.
///
/// Every one of them is made on the worker for the same reason `signing`'s are,
/// and so is the tool call between them: an attached tool runs inside its
/// agent's own loop (grammar §14.1 rule 4), which for a placed agent is a loop
/// the worker is running.
fn stamping(stamp: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "stamp",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("stamped, and here is what it says")),
        Script::new(SONNET, Outcome::structured(json!({ "signature": stamp }))),
    ]
}

/// The same, with a **built-in** call ahead of the attached tool's (PRD
/// resolved q54).
///
/// `agent.signer` holds `builtin.bash` by the shorthand, so its loop can run a
/// command — and, being placed, runs it on the worker that took the dispatch.
///
/// The command is `pwd`, and it is `pwd` because a constant would print the same
/// thing from either process: the shorthand takes the **default** workspace, one
/// directory under the data root of whichever process is running the execution
/// (grammar 6.1), so what the command prints is the one fact in this transcript
/// that can tell a worker-side run from a hub-side one. It is also a bash
/// builtin, which matters here: the shorthand declares no `env:`, so the child
/// is scrubbed and has no `PATH` to look a program up on.
fn signing_after_a_command(signature: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("bash", json!({ "command": "pwd" }))]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "sign",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("signed, and here is the signature")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": signature })),
        ),
    ]
}

/// Wait until `wanted` answers `true`, or fail with what `said` describes.
fn until(what: &str, said: impl Fn() -> String, wanted: impl Fn() -> bool) {
    let deadline = Instant::now() + PATIENCE;
    while Instant::now() < deadline {
        if wanted() {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("never {what}; {}", said());
}

// ---------------------------------------------------------------------------
// 1. The steady state
// ---------------------------------------------------------------------------

/// A hub, a worker, and a flow with a placed node in it: the whole thing runs.
///
/// `flow.release` is two nodes and two processes. `sign` is `agent.signer`,
/// placed on `mac`, so it is dispatched over the wire and its model calls and
/// its attached `tool.sign` happen on the worker; `stamp` is a `function:` node
/// over an **unplaced** tool, so it runs on the hub in the same execution. One
/// run does both, which is what makes "a placement decides which process runs a
/// node" observable rather than asserted.
///
/// Three things are checked, and each fails differently: the flow's outputs,
/// which say the answer crossed the wire and was held to the node's own
/// surface; the hub's journal, which says the worker's effects came home (§3.3);
/// and the provider's transcript, which says the model calls were made once
/// each.
#[test]
fn a_placed_node_runs_on_a_worker_and_its_answer_reaches_the_graph() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(signing("signed-on-the-worker"));
    let _worker = mesh.worker("steady");

    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"], "signed-on-the-worker",
        "the placed node's answer is what the graph wrote: {outputs:#}"
    );
    assert_eq!(
        outputs["ticket"], "notarized",
        "the hub's own node ran in the same execution: {outputs:#}"
    );

    // The effects the worker issued are in the **hub's** journal: workers send,
    // the hub inserts (§3.3, PRD resolved q42).
    until(
        "the worker's effects reached the hub's journal",
        || format!("the run produced {outputs:#}"),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0", "sign/0#model/2"]),
    );

    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained(),
        "the placed agent did not make the three calls it was scripted: {snapshot:#?}"
    );
}

/// A placed **module** tool runs on the worker, out of the artifact it fetched
/// (grammar §6.1, PRD resolved q48, q49).
///
/// This is the mesh consequence of putting authored code inside the emitted
/// project, executed rather than asserted. The worker holds no checkout and no
/// YAML — it is served a tarball of `ARTIFACT_FILES` and runs
/// `src/worker-node.ts` out of it (§4) — so a `function:` node over a placed
/// module tool can only work if the file the binding names is *in* that
/// tarball. It is, because the artifact's file list widened to "what `build`
/// wrote plus the authored files the spec references".
///
/// Two things are checked and each fails differently:
///
///   * the **answer**, which says the authored file crossed the wire and ran.
///     No model is called for this flow at all: a `function:` node names the
///     tool directly, so what runs on the worker is `src/tools/stamp.ts` and
///     nothing else.
///   * the **marker in it**, which says the binding's `env:` declaration became
///     the argument the authored file reads: `src/tools/stamp.ts` spells
///     `undefined` into its answer when nothing was handed to it, so a codegen
///     that dropped the declaration on the way to `callModule` comes back saying
///     so.
///
/// It says nothing about the **partition**, and the distinction is worth
/// keeping: this harness seals its whole environment into every process it
/// starts (`harness::environment`), so `STAMP_MARKER` is on the worker either
/// way and an answer carrying the marker would survive a partition that had
/// contributed nothing for the binding. What the partition did is asserted
/// directly below instead — the variable on `mac`'s manifest and off the hub's
/// own `src/env.ts` — which is where §9.1's "exactly the processes that can
/// execute it" is decided.
#[test]
fn a_placed_module_tool_runs_on_the_worker_out_of_the_artifact_it_fetched() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let _worker = mesh.worker("module");

    let execution = mesh.start_execution("/stampings", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        json!(format!("release.dmg {}", harness::STAMP_MARKER_VALUE)),
        "the authored module did not run on the worker with the environment its \
         binding declared: {outputs:#}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        0,
        "a `function:` node over a placed tool calls no model on either side"
    );

    // The partition, from both ends. The variable is on the placement's list…
    assert!(
        manifest_of(PLACEMENT).contains(&"STAMP_MARKER".to_string()),
        "a module binding's declared environment is not on its placement's \
         manifest: {:?}",
        manifest_of(PLACEMENT)
    );
    // …and off the hub's own, which is what "reaches exactly the processes that
    // can execute it" means (§9.1).
    let hub_environment = std::fs::read_to_string(mesh.project.join("src/env.ts"))
        .expect("the hub's own environment module is readable");
    assert!(
        !hub_environment.contains("STAMP_MARKER"),
        "the hub demands a variable only the worker's tool reads: {hub_environment}"
    );
    // …and the artifact says which of its files the compiler did not write, so a
    // reader of the tree can tell the two halves apart without a JavaScript
    // runtime (PRD resolved q47).
    let manifest = std::fs::read_to_string(mesh.project.join("manifest.json"))
        .expect("the artifact's manifest is readable");
    assert!(manifest.contains("\"src/tools/stamp.ts\""), "{manifest}");
}

/// The same authored file, called from inside a **placed agent's tool loop**
/// (grammar §6.1, §14.1 rule 4, PRD resolved q48, q49).
///
/// PRD resolved q49 puts a module-bound tool where its tool executes, "on a
/// worker, when placed **or reached through attachment**", and those are two
/// different lowerings rather than one. The test above is the first: a
/// `function:` node, where the tool's own placement is the whole of the answer
/// and the dispatched node *is* the tool call. This is the second:
/// `agent.stamper` is dispatched to `mac`, the worker spawns
/// `src/worker-node.ts`, and the loop running there decides to call the module —
/// so the registry seam is exercised inside a dispatched agent node rather than
/// beside one. A seam wired up only on the hub's entrypoint, or only on the
/// `function:` lowering, passes the test above and fails this one.
///
/// Three things carry it, and none of them is the flow's output — that is the
/// model's scripted answer and would be the same if the tool had never run:
///
///   * the **transcript**, where the loop hands the tool's result back to the
///     model. It is read off the provider rather than the graph because the
///     provider is on the other side of the dispatch from the hub: a body
///     carrying `release.dmg <marker>` is the module's own return value, and it
///     was produced in whichever process made that call.
///   * the **effect record** in the **hub's** journal: `attest/0#tool/0`, the
///     ordinary tool key under the placed node's own site, holding a request
///     whose `"surface"` is `"module"`. A worker streams its effects home
///     (§3.3), so this is the journaling parity PRD resolved q48 asks for
///     — same identity, same shape, only the binding differs — stated on the
///     mesh's own surface.
///   * the **provider's transcript being drained**, which says the loop made its
///     three calls rather than short-circuiting after the tool call failed.
#[test]
fn a_module_tool_attached_to_a_placed_agent_runs_in_its_loop_on_the_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(stamping("stamped-in-the-loop"));
    let _worker = mesh.worker("attached-module");

    let execution = mesh.start_execution("/attestations", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"], "stamped-in-the-loop",
        "the placed agent's answer is what the graph wrote: {outputs:#}"
    );

    // The loop handed the module's return value back to the model, so the module
    // ran — in the process that made the call, which is the worker's.
    let requests = mesh.provider.requests();
    let stamped = format!("release.dmg {}", harness::STAMP_MARKER_VALUE);
    assert!(
        requests
            .iter()
            .any(|request| request.body_text.contains(&stamped)),
        "no model call carries the authored module's result, so the loop never called it \
         or called it and dropped the answer: {requests:#?}"
    );

    // And the record of that call came home over `/workers/effects`, keyed under
    // the placed node — the same key an unplaced loop would have written.
    until(
        "the module tool's effect record reached the hub's journal",
        || format!("the run produced {outputs:#}"),
        || harness::journal_holds(&mesh.project, &["attest/0#tool/0", r#""surface":"module""#]),
    );

    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained(),
        "the placed agent did not make the three calls it was scripted: {snapshot:#?}"
    );
}

/// The three calls `agent.tidier` makes on the worker: the attached flow, a
/// turn after it, and the structured answer.
fn tidying(signature: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "tidying",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("looked over, and tidied up after")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": signature })),
        ),
    ]
}

/// **A flow instance that ends on a worker brings both halves of its end home**
/// (grammar §7.9, §14.1 rule 4, `docs/distributed.md` §3.3, §3.4, PRD resolved
/// q76).
///
/// `flow.tidied`'s node is the placed `agent.tidier`, and its model calls the
/// attached `flow.tidying`, whose finally node fails. The attached flow runs in
/// the agent's placement, so its instance ends **on the worker**:
///
/// * the `finally` boundary its end crosses is streamed home over
///   `/workers/effects` like any effect — a record of a kind the hub must take,
///   or the batch is refused and the dispatch ends with it — and lands in the
///   hub's journal under the key a local run derives;
/// * what its finally node raised comes home on `/workers/result` as
///   `finally_error`, and the hub holds it as the execution's own: on the status
///   report and on the lifecycle row, beside a `"completed"` it did not change.
#[test]
fn an_instance_that_ends_on_a_worker_journals_its_boundary_and_reports_its_finally_error() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(tidying("tidied-on-the-worker"));
    let worker = mesh.worker("attached-finally");

    let execution = mesh.start_execution("/tidyings", &json!({ "path": "release.dmg" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "a finally node changes no outcome, on a worker as anywhere: {report:#}
{}",
        worker.transcript()
    );
    assert_eq!(
        report["outputs"]["signature"], "tidied-on-the-worker",
        "{report:#}"
    );
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the finally node's failure on the worker is the execution's: {report:#}
{}",
            worker.transcript()
        )
    });
    assert!(
        finally_error.starts_with("flow.tidying node `tidy_up` failed: "),
        "{finally_error}"
    );

    let rows = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, request, payload FROM effects \
             WHERE execution = '{execution}' AND kind = 'finally'"
        ),
    );
    let rows = rows.as_array().expect("rows");
    assert_eq!(rows.len(), 1, "one instance ended, one boundary: {rows:?}");
    assert_eq!(
        rows[0]["k"], "tidy/0/tidying/0/tidy_up/0#finally/0",
        "the boundary is keyed under the placed node, as an unplaced loop keys it"
    );
    let request: Value = serde_json::from_str(rows[0]["request"].as_str().expect("a request"))
        .expect("the request is JSON");
    assert_eq!(request, json!({ "node": "tidy_up" }));
    let recorded: Value = serde_json::from_str(rows[0]["payload"].as_str().expect("a payload"))
        .expect("the payload is JSON");
    assert_eq!(
        recorded["outcome"], "completed",
        "the end the worker's instance reached is what the boundary holds"
    );

    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status, finally_error FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "completed", "finally_error": finally_error }]),
        "the hub wrote the worker's failure into the row it alone writes"
    );
}

/// A placed agent reads the conversation the node before it left, across the
/// wire (§3.2, §4.3).
///
/// `flow.conversation` is one flow and two processes again, the other way round
/// from `flow.release`: `brief` is an **unplaced** `agent.briefer`, so it runs on
/// the hub and its turns go into the shared `messages` channel (grammar §10.4);
/// `sign` is placed, so it is dispatched — and what it is handed has to be what
/// the same node unplaced would have been handed. §4.3 fixes why: "a placement
/// decides which *process* runs a node rather than which code exists where", and
/// PRD 5.6 says it as "zero change to the logical definition".
///
/// The assertion is made where it cannot be faked: the **provider's transcript**.
/// The worker's own model call carries `agent.signer`'s prompt and, in the same
/// body, the turn the hub's agent produced. A dispatch that dropped the history
/// would leave a body with the prompt and not the turn, and the run would still
/// succeed — which is exactly why this is asserted against the request rather
/// than against the outputs.
#[test]
fn a_placed_agent_is_dispatched_with_the_conversation_the_hubs_own_agent_left() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    // One call on the hub, then the placed agent's three on the worker. All on
    // one model, so the queue is drawn from in flow order.
    mesh.provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "brief": "a release of release.dmg" })),
    ));
    mesh.provider.enqueue_all(signing("signed-after-the-brief"));
    let _worker = mesh.worker("conversation");

    let execution = mesh.start_execution(
        "/conversations",
        &json!({ "path": "release.dmg", "session": "release-42" }),
    );
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"], "signed-after-the-brief",
        "the placed node's answer is what the graph wrote: {outputs:#}"
    );

    let requests = mesh.provider.requests();
    let carried: Vec<&str> = requests
        .iter()
        .filter(|request| request.body_text.contains("Sign the path you are given"))
        .map(|request| request.body_text.as_str())
        .collect();
    assert!(
        !carried.is_empty(),
        "the placed agent made no model call, so nothing here is about it: {requests:#?}"
    );
    assert!(
        carried
            .iter()
            .any(|body| body.contains("a release of release.dmg")),
        "the placed agent was dispatched without the turns `agent.briefer` left in the shared \
         `messages` channel, so it answered from its input object alone — placing the node \
         changed what it does (docs/distributed.md §3.2, §4.3): {carried:#?}"
    );

    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained(),
        "the two agents did not make the four calls they were scripted: {snapshot:#?}"
    );
}

// ---------------------------------------------------------------------------
// 2. The cold start
// ---------------------------------------------------------------------------

/// A worker with an empty data directory bootstraps into the mesh (§3.1, §4).
///
/// The five steps of §4, observed from the outside: the provisioning join
/// carries no `artifact_hash` and no `env_ok`, the answer names one, the worker
/// fetches it, verifies it, materialises it **keyed by hash**, and joins again —
/// and that second join is the one it is dispatched to.
///
/// What is asserted is the two ends: the tree on disk is named by the hash the
/// hub serves, and a dispatch reached this worker. The middle is not observable
/// from here and is `tests/distributed_worker_protocol.rs`'s.
#[test]
fn a_worker_with_an_empty_data_directory_fetches_the_artifact_and_is_dispatched() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let data_dir = scratch_data_dir("cold-start");
    assert!(
        std::fs::read_dir(&data_dir)
            .expect("the data directory exists")
            .next()
            .is_none(),
        "this test starts from an empty data directory"
    );

    mesh.provider
        .enqueue_all(signing("signed-after-a-cold-start"));
    let worker = mesh.worker_into("cold-start", &data_dir);

    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(outputs["signature"], "signed-after-a-cold-start");

    // §4 step 3: materialised under the data directory, keyed by hash, so the
    // previous artifact survives a rollback.
    let hash = mesh.artifact_hash();
    let tree = data_dir.join("artifacts").join(&hash);
    assert!(
        tree.join("src/worker-node.ts").is_file(),
        "the worker did not materialise `{hash}` under `{}`; it said:\n{}",
        data_dir.display(),
        worker.transcript()
    );
    assert_eq!(
        std::fs::read_to_string(data_dir.join("held"))
            .expect("the worker records which artifact it holds")
            .trim(),
        hash
    );
    // …and the whole tree, not a slice of it (§4.3): a worker receives the code
    // of nodes it will never run.
    assert!(tree.join("src/graph.ts").is_file());
    assert!(tree.join("manifest.json").is_file());
}

/// **A hub served without a build from a restamped tree hands its workers the
/// restamped artifact, and they accept it** (PRD resolved q87, Decision D170).
///
/// The mesh's half of the integrity promise, end to end. The fixture is built
/// for its mesh target and one byte of its tree — the `README.md` every
/// artifact carries — is edited by hand on the hub's disk. Two serves of that
/// one tree, each `serve --no-build`, and two workers, each started with the
/// `--no-build` it accepts and ignores:
///
/// 1. **Before the restamp** the hub still names the hash the build wrote, and
///    tars the edited bytes off its disk for it (§3.5), so the worker's
///    verification — the compiler's own rule over the entries it unpacked —
///    answers a different hash and the worker stops, exit `2`, naming both,
///    having materialised nothing. That is the refusal the ruling keeps.
/// 2. **After `build --restamp`** the hub names the restamped hash in its join
///    answer, which is the hash of the files on disk; the worker fetches that
///    tarball, its verification agrees, it materialises the tree under that
///    hash — the hand edit in it — and runs the placed node to completion.
///
/// The worker hashes the **entries it unpacked**, not the tarball's bytes
/// (`worker::artifact::materialise`), which is why the restamp's answer is the
/// one it computes: both are `hash_of` over the same files' bytes.
#[test]
fn a_hub_served_without_a_build_from_a_restamped_tree_hands_workers_the_restamped_artifact() {
    let Some(project) = harness::scratch_project("mesh-restamped") else {
        return;
    };
    let _cleanup = harness::Scratch::at(project.clone());
    let built = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
        .arg("build")
        .arg(harness::fixture(FIXTURE))
        .args(["--target", TARGET])
        .arg("--out")
        .arg(&project)
        .env("NO_COLOR", "1")
        .output()
        .expect("the compiler runs");
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let stamp_of = |tree: &Path| {
        compose_core::codegen::artifact::Stamp::parse(
            &std::fs::read_to_string(tree.join("src/artifact.ts")).expect("the tree has a stamp"),
        )
        .expect("the stamp reads")
    };
    let as_built = stamp_of(&project);
    let edit = "\n<!-- edited by hand on the hub, after the build -->\n";
    let readme = project.join("README.md");
    let mut text = std::fs::read_to_string(&readme).expect("the build wrote a README");
    text.push_str(edit);
    std::fs::write(&readme, &text).expect("the tree is writable");
    let on_disk = compose_core::codegen::artifact::rehash(&project, &as_built.files)
        .expect("every listed file is there");
    assert_ne!(
        on_disk, as_built.hash,
        "the edit moved what the tree hashes to"
    );

    // 1. Unrestamped: the hub's name for its tree is not what its tree is.
    {
        let Some(mesh) = Mesh::start_as_built(&project) else {
            return;
        };
        assert_eq!(mesh.artifact_hash(), as_built.hash);
        let data_dir = scratch_data_dir("unrestamped");
        let mut worker = mesh.worker_into_without_a_build("unrestamped", &data_dir);
        let status = worker.waited();
        assert_eq!(
            status.code(),
            Some(2),
            "the worker refused the artifact: {}",
            worker.transcript()
        );
        let said = worker.transcript();
        assert!(
            said.contains(&format!(
                "the artifact this hub served hashes to `{on_disk}` and was asked for as `{}`",
                as_built.hash
            )),
            "{said}"
        );
        assert!(
            !data_dir.join("artifacts").join(&as_built.hash).exists()
                && !data_dir.join("artifacts").join(&on_disk).exists(),
            "nothing was materialised"
        );
    }

    // 2. Restamped: the hub names the tree it has, and the worker takes it.
    let restamped = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
        .arg("build")
        .arg(project.join("no-spec").join("main.yml"))
        .arg("--out")
        .arg(&project)
        .arg("--restamp")
        .env("NO_COLOR", "1")
        .output()
        .expect("the compiler runs");
    assert!(
        restamped.status.success(),
        "{}",
        String::from_utf8_lossy(&restamped.stderr)
    );
    let stamped = stamp_of(&project);
    assert_eq!(
        stamped.hash, on_disk,
        "the restamp wrote the tree's own hash"
    );

    let Some(mesh) = Mesh::start_as_built(&project) else {
        return;
    };
    assert_eq!(
        mesh.artifact_hash(),
        stamped.hash,
        "the join handshake carries the restamped hash"
    );
    mesh.provider
        .enqueue_all(signing("signed-out-of-a-restamped-tree"));
    let data_dir = scratch_data_dir("restamped");
    let worker = mesh.worker_into_without_a_build("restamped", &data_dir);
    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        "signed-out-of-a-restamped-tree",
        "the placed node ran on the worker: {}",
        worker.transcript()
    );
    assert_eq!(
        std::fs::read_to_string(data_dir.join("held"))
            .expect("the worker records which artifact it holds")
            .trim(),
        stamped.hash
    );
    let materialised = data_dir.join("artifacts").join(&stamped.hash);
    assert!(
        std::fs::read_to_string(materialised.join("README.md"))
            .expect("the worker materialised the tree")
            .ends_with(edit),
        "the tree the worker runs out of is the hub's, hand edit and all"
    );
}

// ---------------------------------------------------------------------------
// 3. Parking, and the join that wakes it
// ---------------------------------------------------------------------------

/// A placed node with no worker **parks**, and a join is what wakes it (§6).
///
/// The pause of §6.4's first row: nothing has failed, the execution is on the
/// board with a placement wait the status route publishes, and it costs the
/// execution nothing while it waits. The wake is a join — "a joining worker's
/// claims are scanned against the open placement-waits, and dispatch resumes in
/// park order" — and there is no polling anywhere in it.
#[test]
fn a_placed_node_with_no_worker_parks_and_a_join_is_what_wakes_it() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(signing("signed-after-parking"));

    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let parked = mesh.until(&execution, "parked on its placement", |report| {
        !report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty)
    });
    let wait = &parked["placement_waits"][0];
    assert_eq!(wait["placement"], PLACEMENT, "{parked:#}");
    assert_eq!(wait["node"], "flow.release.sign", "{parked:#}");
    assert_eq!(
        wait["status"], "parked",
        "a wait nothing has taken is parked, not dispatched: {parked:#}"
    );
    assert_eq!(
        parked["status"], "running",
        "an undispatched node is a pause rather than a failure (§6.4): {parked:#}"
    );

    // The wake. Nothing else changes — the execution was never touched.
    let _worker = mesh.worker("wake");
    let outputs = mesh.completed(&execution);
    assert_eq!(outputs["signature"], "signed-after-parking");
}

// ---------------------------------------------------------------------------
// 4. The mid-node disconnect
// ---------------------------------------------------------------------------

/// A worker that vanishes **while running a node** fails that attempt, and the
/// retry replays what it had already done (§6.3, §7.2, §7.3).
///
/// The whole of §7.3 in one run. The first worker gets as far as journaling the
/// loop's first model call and is then killed; the hub's liveness window closes,
/// supersedes the dispatch it was holding, and the node's `retry:` grants a
/// second attempt which parks because nothing is claiming the placement. A fresh
/// worker joins, is handed the dispatch **with the journaled effect history**,
/// and replays to the frontier — so the model call already paid for is not paid
/// for twice, which the provider's transcript is what proves.
///
/// And the late result: a result posted for a dispatch the hub superseded is
/// answered `409` and discarded (§3.4), which is asserted directly, because a
/// real worker acts on that status rather than reporting it.
#[test]
fn a_worker_killed_mid_node_fails_an_attempt_and_the_retry_replays_what_it_did() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    // The first attempt: one loop call that answers at once and is journaled,
    // then a call that never comes back inside this test's patience — which is
    // what "the worker was in the middle of the node" means.
    mesh.provider.enqueue_all([
        Script::new(SONNET, Outcome::text("thinking about it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "never delivered" }))
                .after(Duration::from_secs(600)),
        ),
    ]);
    let mut first = mesh.worker("disconnect");

    let execution = mesh.start_execution("/retried-releases", &json!({ "path": "release.dmg" }));
    // Killed once the effect is **committed** on the hub, rather than after a
    // sleep: the record is what the retry replays, and a kill that raced the
    // `POST` would be testing a different thing.
    until(
        "the first attempt's model call reached the hub's journal",
        || first.transcript(),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0"]),
    );
    let dispatched = mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    let superseded = dispatched["placement_waits"][0]["dispatch_id"]
        .as_str()
        .expect("a dispatched wait names the dispatch holding it")
        .to_string();
    first.kill();

    // §6.3: the hub declares the session gone, supersedes the dispatch, and the
    // attempt fails under the node's own `retry:` — which re-parks, because
    // nothing is claiming `mac` any more.
    let reparked = mesh.until(&execution, "re-parked after the supersede", |report| {
        report["placement_waits"][0]["dispatch_id"]
            .as_str()
            .is_some_and(|held| held != superseded)
    });
    assert_eq!(
        reparked["placement_waits"][0]["status"], "parked",
        "{reparked:#}"
    );

    // The late result of the worker that vanished: `409`, and discarded.
    let session = mesh.session();
    let late = mesh.send(
        Request::post("/workers/result")
            .header("authorization", format!("Bearer {TOKEN}"))
            .header("x-worker-session", &session)
            .json(&json!({
                "dispatch_id": superseded,
                "output": { "signature": "too late" },
            })),
    );
    assert_eq!(late.status, 409, "{}", body_of(&late));

    // The retry, on a worker that joins afterwards. It replays the recorded
    // loop call and goes live at the one the journal does not hold.
    mesh.provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "signature": "signed on the second attempt" })),
    ));
    let _second = mesh.worker("revived");
    let outputs = mesh.completed(&execution);
    assert_eq!(outputs["signature"], "signed on the second attempt");

    // **The assertion this test exists for.** Three requests reached the
    // provider: the first attempt's two, and the retry's one. A retry that had
    // re-issued the recorded call would have made four.
    let made = mesh.provider.requests();
    assert_eq!(
        made.len(),
        3,
        "the retry re-issued an effect the journal already held (docs/distributed.md §7.2): {made:#?}"
    );
}

// ---------------------------------------------------------------------------
// 5. The hub restart
// ---------------------------------------------------------------------------

/// A hub replaced behind its own name loses its sessions and nothing else (§5).
///
/// "A hub restarted under a stable name loses sessions and nothing else:
/// workers' next requests are answered `410` — they join again, and dispatch
/// resumes from the journal." The worker is never told about the restart and
/// never reconfigured: it addresses a *name*, and the process behind it changed.
#[test]
fn a_hub_restarted_under_its_own_name_is_met_with_a_re_join() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue_all(signing("signed-across-a-restart"));

    // One execution before the restart, which is how this test knows the worker
    // has really joined: a run that completed is a session that was issued.
    let worker = mesh.worker("restart");
    let before = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    assert_eq!(
        mesh.completed(&before)["signature"],
        "signed-across-a-restart"
    );

    // The process behind the name is replaced. The worker is not told, is not
    // restarted, and is not reconfigured.
    mesh.provider
        .enqueue_all(signing("signed-by-the-process-that-replaced-it"));
    mesh.restart();

    // Its next request meets `410`, it joins again under the process that
    // replaced its predecessor, and dispatch resumes.
    let after = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&after);
    assert_eq!(
        outputs["signature"],
        "signed-by-the-process-that-replaced-it",
        "the worker did not come back to the hub that replaced the one it joined; it said:\n{}",
        worker.transcript()
    );
}

/// …and one replaced **while a worker is holding a dispatch** (§5, §3.4, §6.3).
///
/// The test above replaces an idle hub, where the worker's next request is a
/// poll on a session that is holding nothing. This one replaces the process with
/// a dispatch out at a live worker, which is the state §5 writes a rule for — "an
/// unsettled dispatch on an ended session is superseded exactly as §6.3
/// supersedes one" — and it is the whole of the worker's side of a restart that
/// the idle case never reaches:
///
/// * the **poll thread** meets `410` and re-joins;
/// * the **node thread**, still running `bun src/worker-node.ts` from before the
///   restart, meets `410` on its own effect and result `POST`s and has to take
///   the session that re-join produced rather than joining a second time itself;
/// * its result names a dispatch the replacement process superseded at start, so
///   it meets `409` — which §3.4 makes "discard the result and keep the session"
///   and §3.1 would make terminal if a worker read it as a refusal;
/// * and the attempt the retry opened is accepted **while that first runner is
///   still alive**, which is §2's queued dispatch on a session the hub has every
///   right to hand one to.
///
/// A regression in any of those strands every worker in a mesh at the first
/// redeployment, and leaves a suite of idle restarts green. So what is asserted
/// is that the same worker process — never restarted, never reconfigured — is
/// still running at the end and is the one that answered the retry.
#[test]
fn a_hub_restarted_over_a_dispatch_a_worker_is_running_is_met_with_a_re_join() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    /// What both attempts answer with — see the third script.
    const SIGNED: &str = "signed-across-a-restart-mid-node";
    mesh.provider.enqueue_all([
        // The first attempt: one loop call that answers at once and is
        // journaled, and then a pinned-output call that does not come back
        // until the process behind the hub has been replaced.
        Script::new(SONNET, Outcome::text("thinking about it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": SIGNED })).after(Duration::from_secs(3)),
        ),
        // Whatever the second attempt still has to ask for. A redispatch carries
        // the history the journal held when the worker **took** it (§7.2), so
        // whether the pinned-output call is replayed or re-issued turns on
        // whether the re-join beat a three-second model call — a race between
        // two processes, and both of its answers are this one.
        Script::new(SONNET, Outcome::structured(json!({ "signature": SIGNED }))),
    ]);
    let mut worker = mesh.worker("restart-mid-node");

    let execution = mesh.start_execution("/retried-releases", &json!({ "path": "release.dmg" }));
    // The restart happens once the worker is **inside** the node: the first
    // effect is committed on the hub, and the wait says a session is holding
    // the dispatch.
    until(
        "the first attempt's model call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0"]),
    );
    let holding = mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    let superseded = holding["placement_waits"][0]["dispatch_id"]
        .as_str()
        .expect("a dispatched wait names the dispatch holding it")
        .to_string();

    // The process behind the name is replaced, with that dispatch out at a
    // worker that is still executing it.
    mesh.restart();

    // §5: the dispatch the replaced process was holding is superseded, and the
    // node's `retry:` opens another — a different dispatch, at the next ordinal.
    let retried = mesh.until(&execution, "opened a second attempt", |report| {
        report["placement_waits"][0]["dispatch_id"]
            .as_str()
            .is_some_and(|held| held != superseded)
    });
    assert_eq!(
        retried["placement_waits"][0]["wait_id"], "sign/0/1",
        "the attempt after a superseded one is a wait of its own (§6.1): {retried:#}"
    );

    // …and the worker that was mid-node when the process changed under it is the
    // one that answers it.
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        SIGNED,
        "the retry did not reach the worker that survived the restart; it said:\n{}",
        worker.transcript()
    );
    assert!(
        worker.running(),
        "the worker stopped somewhere across the restart — a `410` or the `409` its late result \
         met was read as a refusal (§3.1, §3.4); it said:\n{}",
        worker.transcript()
    );
}

/// The fixture with `flow.retried`'s `retry:` written as `retry`, served under
/// its mesh target — for a case that needs the second attempt to wait a known
/// time rather than the fixture's millisecond.
fn retried_under(purpose: &str, retry: &str) -> Option<Mesh> {
    let deploy = std::fs::read_to_string(
        harness::fixture(FIXTURE)
            .parent()
            .expect("a fixture directory")
            .join("deploy")
            .join(format!("{TARGET}.yml")),
    )
    .expect("the fixture's mesh target is readable");
    let (composition, entrypoint) = harness::staged_with_deploy(purpose, FIXTURE, TARGET, &deploy);
    let main = std::fs::read_to_string(&entrypoint).expect("the staged composition is readable");
    let at = main
        .find("\nflow.retried:\n")
        .expect("the fixture declares `flow.retried`");
    let (head, tail) = main.split_at(at);
    // The first `retry:` after the flow's own line is its `sign`'s: the flow
    // declares one node.
    const WRITTEN: &str = "retry: { max: 2, backoff: 1ms }";
    assert!(
        tail.contains(WRITTEN),
        "`flow.retried` no longer says `{WRITTEN}`"
    );
    std::fs::write(
        &entrypoint,
        format!("{head}{}", tail.replacen(WRITTEN, retry, 1)),
    )
    .expect("the staged composition is writable");
    Mesh::start_from(purpose, entrypoint, Some(composition))
}

/// **What a worker's stop cuts off is never journaled, and the retry makes it**
/// (`docs/distributed.md` §3.2, §5, §7.3; `docs/durability.md` §6.1; PRD
/// resolved q75, q79).
///
/// `flow.retried` once more, with its second attempt six seconds after the
/// first and no jitter — so the order below is the same every run. The first
/// attempt's loop call answers and is journaled; its pinned-output call is held
/// for thirty seconds, and the process behind the hub is replaced while it is
/// in flight. The worker meets `410`, joins again, and stops the runner it was
/// running under the ended session (§5) by the stop the `stop` list uses —
/// which aborts that call. The stop is the hub having stopped waiting, not the
/// provider failing the call, so the runner records nothing for it; recorded,
/// the failure would have been posted home under the new session (§3.3) and
/// handed to the retry six seconds later as its history (§7.3), and **every**
/// attempt would have replayed it — the execution failing on a
/// `DispatchStopped` nobody asked for.
///
/// So the run completes with the third script's answer, which only a call the
/// retry made live could have fetched; the provider was asked three times —
/// the loop call once, never again, and the cut-off call twice; the journal's
/// record at that call's key is the retry's answer; and no record anywhere in
/// the execution carries the stop. The runner is gone long before its held call
/// would have answered, which is what says the stop really met it in flight.
#[test]
fn a_model_call_a_workers_stop_cut_off_is_not_journaled_and_the_retry_makes_it() {
    let Some(mut mesh) = retried_under(
        "mesh-stop-unrecorded",
        "retry: { max: 2, backoff: 6s, jitter: false }",
    ) else {
        return;
    };
    /// What the retry's live call answers, and nothing else does.
    const SIGNED: &str = "signed-by-the-retry-the-stop-left-to-ask";
    /// How long the first attempt's pinned-output call is held.
    const HELD: Duration = Duration::from_secs(30);
    mesh.provider.enqueue_all([
        Script::new(SONNET, Outcome::text("thinking about it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "answered after the stop" })).after(HELD),
        ),
        Script::new(SONNET, Outcome::structured(json!({ "signature": SIGNED }))),
    ]);
    let mut worker = mesh.worker("stop-unrecorded");
    let execution = mesh.start_execution("/retried-releases", &json!({ "path": "release.dmg" }));

    // The worker is **inside** the held call: the loop call is committed on
    // the hub, the dispatch is held, and the provider has the second request.
    until(
        "the first attempt's loop call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0"]),
    );
    mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    until(
        "the first attempt's pinned-output call reached the provider",
        || worker.transcript(),
        || mesh.provider.requests().len() == 2,
    );
    let in_flight = Instant::now();
    let runner = worker
        .runners()
        .and_then(|running| running.first().copied());

    mesh.restart();

    // The stop met the call in flight: the runner is gone well before the
    // provider would have answered it.
    if let Some(runner) = runner {
        assert!(
            gone_within(
                runner,
                (HELD - Duration::from_secs(5)).saturating_sub(in_flight.elapsed())
            ),
            "the runner the ended session left was not stopped in the middle of its call: {}",
            worker.transcript()
        );
    }

    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        SIGNED,
        "the retry did not make the call the stop cut off; it said:\n{}",
        worker.transcript()
    );
    let made = mesh.provider.requests();
    assert_eq!(
        made.len(),
        3,
        "the retry re-issued the recorded loop call, or never made the one the stop cut off: \
         {made:#?}"
    );

    let rows = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, outcome, payload FROM effects \
             WHERE execution = '{execution}' ORDER BY \"key\""
        ),
    );
    let rows = rows.as_array().expect("rows");
    let stopped: Vec<&Value> = rows
        .iter()
        .filter(|row| {
            row["payload"]
                .as_str()
                .is_some_and(|payload| payload.contains("DispatchStopped"))
        })
        .collect();
    assert!(
        stopped.is_empty(),
        "an effect the worker's stop aborted was journaled as the world's answer: {stopped:#?}"
    );
    let cut_off = rows
        .iter()
        .find(|row| row["k"] == "sign/0#model/1")
        .unwrap_or_else(|| panic!("the call the stop cut off has no record at all: {rows:#?}"));
    assert!(
        cut_off["payload"]
            .as_str()
            .is_some_and(|payload| payload.contains(SIGNED) && payload.contains("\"ok\":true")),
        "the record at the cut-off call's key is not the retry's answer: {cut_off:#}"
    );
    assert!(worker.running(), "{}", worker.transcript());
}

// ---------------------------------------------------------------------------
// 6. The refusal matrix, from the worker's side
// ---------------------------------------------------------------------------

/// A refused join is **terminal**: the worker exits non-zero, echoing it (§3.1).
///
/// `tests/distributed_hub_wire.rs` holds the hub's half of §3.1 — every row at
/// the status and the shape the document gives it, and the order they are
/// applied in. This is the other half, and it is the one only a real worker can
/// answer: "so a worker refused at join **stops**: it exits non-zero, naming the
/// refusal as it was given, and starting it again is an operator's act".
///
/// Two rows are driven here because they are the two a worker's *invocation*
/// can produce — a credential that does not verify, and a claim naming no
/// placement — and both are conditions another join would meet identically.
#[test]
fn a_worker_refused_at_join_stops_rather_than_joining_again() {
    let Some(mesh) = Mesh::start() else {
        return;
    };

    // The token does not verify: `401`, and a refused credential is told nothing
    // about why (§3.1's first row).
    let mut wrong = {
        let mut environment = mesh.environment.clone();
        environment.retain(|(name, _)| name != TOKEN_VARIABLE);
        environment.push((TOKEN_VARIABLE.to_string(), "not-the-token".to_string()));
        Worker::start(&mesh.base_url, &environment, "bad-token", None)
    };
    let status = wrong.waited();
    assert_eq!(
        status.code(),
        Some(2),
        "a worker refused at join exits non-zero; it said:\n{}",
        wrong.transcript()
    );
    let said = wrong.transcript();
    assert!(
        said.contains("401"),
        "the worker did not echo the refusal it was given:\n{said}"
    );
    assert!(
        !said.contains("not-the-token"),
        "the worker echoed the credential it presented:\n{said}"
    );

    // A claim that names no placement: `400`, naming the claim and the target's
    // own placement names.
    let mut unknown = {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .arg("worker")
            .args(["--hub", &mesh.base_url])
            .args(["--claim", "a-placement-nobody-declared"])
            .args(["--token-env", TOKEN_VARIABLE])
            .arg("--data-dir")
            .arg(scratch_data_dir("unknown-claim"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        harness::seal(&mut command, &mesh.environment);
        command.output().expect("the worker runs")
    };
    assert_eq!(unknown.status.code(), Some(2));
    let said = String::from_utf8_lossy(&std::mem::take(&mut unknown.stderr)).into_owned();
    assert!(
        said.contains("a-placement-nobody-declared") && said.contains(PLACEMENT),
        "the refusal names neither the claim nor the target's placements:\n{said}"
    );
}

/// A manifest this machine does not satisfy is `403`, naming **variables** and
/// nothing else (§3.1, §9.2).
///
/// The check exists for one machine — the one joined before its keychain was set
/// up — and this is that machine: every variable of `mac`'s manifest but
/// `KEYCHAIN_PASSWORD`, which is §9.1's own worked example of a secret that
/// belongs to a placement and not to the hub.
#[test]
fn a_worker_whose_manifest_is_unmet_is_refused_by_variable_name() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let mut environment = mesh.environment.clone();
    environment.retain(|(name, _)| name != "KEYCHAIN_PASSWORD");
    let mut worker = Worker::start(&mesh.base_url, &environment, "unmet", None);
    let status = worker.waited();
    assert_eq!(
        status.code(),
        Some(2),
        "a worker refused at join exits non-zero; it said:\n{}",
        worker.transcript()
    );
    let said = worker.transcript();
    assert!(said.contains("403"), "{said}");
    assert!(
        said.contains("KEYCHAIN_PASSWORD"),
        "the refusal does not name the variable that is missing:\n{said}"
    );
    assert!(
        !said.contains("the-machine-keychain"),
        "a refusal carried a value; §9 gives it names and never values:\n{said}"
    );
}

// ---------------------------------------------------------------------------
// 7. Queueing onto a pool of one
// ---------------------------------------------------------------------------

/// `max_concurrency:` bounds admission; the placement delivers what its pool can
/// run (§2, §6.4).
///
/// "A `max_concurrency: 8` over an `agent.signer` placed on `mac`, with one
/// worker on the Mac, runs one at a time: the map admits eight, the placement
/// delivers one, and the other seven are queued to the placement." The fixture's
/// map admits four onto a pool of one, so what has to hold is that no two of
/// them are ever **dispatched** at once — which the status route publishes, one
/// row per wait.
#[test]
fn a_fan_out_onto_a_one_worker_placement_runs_one_item_at_a_time() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    for index in 0..4 {
        mesh.provider
            .enqueue_all(signing(&format!("signature-{index}")));
    }
    let _worker = mesh.worker("queueing");

    let execution = mesh.start_execution(
        "/batches",
        &json!({ "paths": ["a.dmg", "b.dmg", "c.dmg", "d.dmg"] }),
    );

    // Sampled while it runs: the map admits four, and never more than one of
    // them is in a worker's hands.
    let watching = Instant::now() + Duration::from_secs(30);
    let mut seen_parked = 0usize;
    loop {
        let report = mesh.report(&execution);
        if report["status"] != "running" {
            break;
        }
        let waits = report["placement_waits"]
            .as_array()
            .cloned()
            .unwrap_or_default();
        let dispatched = waits
            .iter()
            .filter(|wait| wait["status"] == "dispatched")
            .count();
        assert!(
            dispatched <= 1,
            "two items of one fan-out were dispatched at once onto a pool of one \
             (docs/distributed.md §2): {report:#}"
        );
        seen_parked = seen_parked.max(waits.len().saturating_sub(dispatched));
        if Instant::now() > watching {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }

    let outputs = mesh.completed(&execution);
    let mut signatures: Vec<String> = outputs["signatures"]
        .as_array()
        .expect("the map wrote its signatures")
        .iter()
        .map(|value| value.as_str().unwrap_or_default().to_string())
        .collect();
    signatures.sort();
    assert_eq!(
        signatures,
        [
            "signature-0".to_string(),
            "signature-1".to_string(),
            "signature-2".to_string(),
            "signature-3".to_string(),
        ],
        "every item of the fan-out was signed on the one worker"
    );
    assert!(
        seen_parked >= 1,
        "no item of the fan-out was ever seen waiting, so this test observed no queue at all"
    );
}

// ---------------------------------------------------------------------------
// 8. A placed pause comes home (§3.4, PRD resolved q46)
// ---------------------------------------------------------------------------

/// The three model calls `agent.escalator` makes across a pause: the loop's
/// request for the attached flow, the loop's answer once a person has decided,
/// and the pinned structured output that ends the node.
///
/// **The first is made before the pause and the other two after it**, on two
/// dispatches of one node — which is what makes the count in each test below an
/// assertion about replay rather than a tally: a redispatch that re-issued the
/// pre-pause call would make four requests out of a three-script queue.
fn escalating(approval: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "escalation",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("the person decided")),
        Script::new(SONNET, Outcome::structured(json!({ "approval": approval }))),
    ]
}

/// The pause `flow.escalation`'s `ask` node opens, as a hub-side wait would
/// publish it — asserted field by field, because parity is the acceptance bar.
///
/// PRD resolved q46: "a placed `human:` node must mean what the same node
/// unplaced means: timeout/retry/on_error semantics, status visibility,
/// lifecycle webhooks and the resume surface are the single-process ones,
/// reached over the wire". The status shape is the half a test can read
/// directly, and every key here is `src/serve.ts`'s `question()` — the one
/// function both kinds of pause are published through, which is why the schema
/// and the resume URL are the node's own rather than something that travelled.
fn assert_is_the_pause_the_composition_declares(wait: &Value, execution: &str) {
    let id = wait["wait_id"].as_str().unwrap_or_default();
    assert!(
        id.ends_with("/ask/0"),
        "the wait is not addressed by the identity grammar §9.4 gives the `ask` node: {wait:#}"
    );
    assert!(
        id.starts_with("escalate/0/"),
        "the wait's identity is not derived under the placed node's own instance path, so a \
         resume prepared against one generation would not find it in the next \
         (docs/distributed.md §6.1): {wait:#}"
    );
    assert_eq!(wait["flow"], "flow.escalation", "{wait:#}");
    assert_eq!(wait["node"], "ask", "{wait:#}");
    assert_eq!(
        wait["input"],
        json!({ "path": "release.dmg" }),
        "the pause does not show what the node's `input:` evaluated to: {wait:#}"
    );
    // The composition's own contract, read off the hub's copy of the descriptor
    // rather than off anything the wire carried (§4.3): the artifact is
    // everywhere, so the schema a UI is handed is this node's `output:`.
    assert_eq!(
        wait["output_schema"]["properties"]["decision"]["enum"],
        json!(["approve", "reject"]),
        "the published contract is not the `human:` node's own `output:`: {wait:#}"
    );
    assert_eq!(
        wait["resume_url"],
        json!(format!(
            "/executions/{execution}/resume?wait={}",
            urlencoded(id)
        )),
        "the pause is not answered through the surface every other pause is: {wait:#}"
    );
    assert!(
        wait["paused_at"].is_string(),
        "the pause does not say when the wait began: {wait:#}"
    );
}

/// **This report holds no placement wait**, asserted against a report that is
/// demonstrably still publishing wait state.
///
/// `placement_waits` is *omitted* from a report holding none
/// (`runtime.ts`'s `executionReport`), so "absent or empty" is the shape being
/// asserted — and on its own that is one-sided: a status route that stopped
/// publishing the key at all would satisfy it while the property it exists for,
/// "a paused dispatch is settled rather than parked", had quietly become
/// unobservable. The companion is what closes that: the same document publishes
/// the pause as an `interrupts` entry, so the report's wait state is live and
/// what it does not carry is a placement wait rather than a key that went away.
fn assert_holds_no_placement_wait(report: &Value, what: &str) {
    assert!(
        report["interrupts"]
            .as_array()
            .is_some_and(|open| !open.is_empty()),
        "this report publishes no open question at all, so `placement_waits` says nothing about \
         whether a placed dispatch is parked: {report:#}"
    );
    assert!(
        report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "{what}: {report:#}"
    );
}

/// A `human:` node a **placed** agent reaches parks on the hub's board, and the
/// answer sends the node back to a worker (§3.4, PRD resolved q46).
///
/// Grammar §14.1 rule 4 puts everything an attached `flow.*` reaches in the
/// attaching agent's placement, so `agent.escalator`'s `flow.escalation` asks its
/// question on the **worker**. The wait board is the hub's — it is what a status
/// report publishes, what the resume route answers and what a recovery
/// re-derives (§1, PRD resolved q4/q28) — so the dispatch settles *paused* and
/// the wait comes home.
///
/// Six things are checked and each fails differently:
///
///  * the pause is on the hub's board, published as any pause is;
///  * the dispatch is **settled**, not parked — the worker is free while a
///    person thinks, which is §3.4's whole reason for making paused a way of
///    being settled;
///  * the resume surface takes the answer;
///  * the node re-enters dispatch and finishes **on a worker**;
///  * the pre-pause model call is **not re-issued** — the answered pause rides
///    the redispatch's `effect_history` and the replay goes live past it (§7.2);
///  * the answer is journaled as the pause's own effect record.
#[test]
fn a_pause_a_placed_agent_reaches_comes_home_and_its_answer_sends_the_node_back() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue_all(escalating("approved by a person"));
    let receiver = harness::Receiver::start().expect("a loopback port");
    let worker = mesh.worker("escalation");

    let execution = mesh.start_execution(
        "/escalations",
        &json!({
            "path": "release.dmg",
            "callback_url": format!("{}/hook", receiver.base_url),
        }),
    );
    let wait = mesh.paused(&execution);
    assert_is_the_pause_the_composition_declares(&wait, &execution);

    // **The same lifecycle webhook a local pause fires** (§6.6, PRD resolved
    // q34). A parking is an execution that has stopped advancing on its own, and
    // one whose only open work is a question nobody has answered is that
    // whichever process asked it — so the delivery is `parked`, its body is the
    // report the status route serves, and the wait it names is this one.
    //
    // Searched rather than counted, because this execution has **two** parkings
    // in it and which of them is announced is a race a test should not decide:
    // the dispatch parks first if no worker has claimed `mac` yet (§6.4's first
    // row), and the pause parks second. What is asserted is that the second one
    // is announced, and announced as a question rather than as a machine.
    let announced = {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let held = receiver.of_event("parked");
            if let Some(found) = held
                .iter()
                .find(|delivered| delivered.body["interrupts"][0]["wait_id"] == wait["wait_id"])
            {
                break found.body.clone();
            }
            assert!(
                Instant::now() < deadline,
                "no `parked` webhook announced the pause the worker opened; the deliveries were \
                 {:#?}",
                held.iter().map(|one| one.body.clone()).collect::<Vec<_>>()
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    assert_eq!(announced["execution_id"], json!(execution));
    assert_eq!(
        announced["status"],
        json!("interrupted"),
        "an execution holding a worker's question is interrupted, exactly as one holding its \
         own is: {announced:#}"
    );
    assert_eq!(
        announced["interrupts"][0]["output_schema"]["properties"]["decision"]["enum"],
        json!(["approve", "reject"]),
        "the delivery published a contract other than the `human:` node's own: {announced:#}"
    );
    assert_holds_no_placement_wait(
        &announced,
        "the parking announced a placement wait for a dispatch a pause settled",
    );

    // **Settled, not parked.** A paused result ends the dispatch, so the hub is
    // holding no placement wait for this node and the session that posted it may
    // be dispatched other work.
    let report = mesh.report(&execution);
    assert_holds_no_placement_wait(
        &report,
        "the hub is still holding a placement wait for a dispatch a paused result settled",
    );
    let settled = harness::journal_rows(
        &mesh.project,
        "SELECT status, node FROM dispatches ORDER BY rowid",
    );
    assert_eq!(
        settled,
        json!([{ "status": "settled", "node": "flow.escalated.escalate" }]),
        "the paused dispatch is not settled on the board: {settled:#}"
    );

    // The ordinary resume surface, with the payload the node's `output:` admits.
    let id = wait["wait_id"].as_str().expect("the wait names itself");
    let answered = mesh.resume(&execution, id, &json!({ "decision": "approve" }));
    assert_eq!(answered.status, 202, "{}", body_of(&answered));
    assert_eq!(answered.json()["wait"], json!(id), "{}", body_of(&answered));

    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["approval"],
        "approved by a person",
        "the node did not finish after the answer; the worker said:\n{}",
        worker.transcript()
    );

    // **The redispatch is a second dispatch of the same node**, and it settled
    // with the answer: the node re-entered dispatch rather than being resumed in
    // place, which is what makes the pause the hub's and the work the worker's.
    let board = harness::journal_rows(
        &mesh.project,
        "SELECT status, placement, node FROM dispatches ORDER BY rowid",
    );
    assert_eq!(
        board,
        json!([
            { "status": "settled", "placement": PLACEMENT, "node": "flow.escalated.escalate" },
            { "status": "settled", "placement": PLACEMENT, "node": "flow.escalated.escalate" },
        ]),
        "the answered pause did not send the node back through dispatch: {board:#}"
    );

    // **The pre-pause model call was replayed, not re-issued.** Three scripts,
    // three requests: the loop's first call was made once, before the pause, and
    // the redispatch consumed it out of `effect_history` (§7.2, §7.3's "a model
    // call already paid for is not paid for twice").
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the redispatch re-issued a model call the journal already held: {requests:#?}"
    );
    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained(),
        "the escalating agent did not make the three calls it was scripted: {snapshot:#?}"
    );

    // …and the answer is in the journal as the pause's own effect record, which
    // is what the redispatch replayed and what a second resume would find.
    let human = harness::journal_rows(
        &mesh.project,
        "SELECT key, kind FROM effects WHERE kind = 'human'",
    );
    let records = human.as_array().expect("the query answers rows");
    assert_eq!(
        records.len(),
        1,
        "the answered pause is not journaled once as a `human` effect: {human:#}"
    );
    // At the pause's **own site**, which for a `human` node is the wait's own
    // identity (`docs/durability.md` §4): the key the replaying node claims is
    // derived from the very path the pause was opened at, so the record the hub
    // wrote and the record the redispatch looks up cannot be two.
    assert_eq!(
        records[0]["key"],
        json!(format!("{id}#human/0")),
        "the answer is journaled under a key the redispatched node would not look up: {human:#}"
    );
}

/// The worker is **gone** when the answer arrives: the redispatch parks, and a
/// fresh worker takes it (§6.2, §6.4, PRD resolved q39).
///
/// This is the property a paused result buys that a held wait could not: the
/// dispatch is settled, so the worker that asked the question is free and may
/// leave. When the answer comes, the node re-enters dispatch like any placed
/// node with nobody claiming its placement — it parks on the board, in park
/// order, and the wake is a join.
#[test]
fn a_placed_pause_answered_after_its_worker_left_parks_until_another_joins() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue_all(escalating("approved after the mac left"));
    let mut asked = mesh.worker("escalation-asked");

    let execution =
        mesh.start_execution("/abandoned-escalations", &json!({ "path": "release.dmg" }));
    let wait = mesh.paused(&execution);
    let id = wait["wait_id"]
        .as_str()
        .expect("the wait names itself")
        .to_string();

    // The machine that asked the question goes away while the person thinks.
    asked.kill();

    let answered = mesh.resume(&execution, &id, &json!({ "decision": "approve" }));
    assert_eq!(answered.status, 202, "{}", body_of(&answered));

    // The redispatch has nobody to take it, so it is a **pause** rather than a
    // failure (§6.4's first row): on the board, and costing the execution
    // nothing but its own `timeout:`. Not necessarily at once, though: the dead
    // worker's session is presumed live until the window of §2 expires, so a
    // hub may first hand the redispatch to the session that is gone — and the
    // expiry then supersedes it and parks the retry (§6.3). Both readings end
    // on the board, so the board's **parked** row is what is waited for, never
    // the first row to appear.
    let parked = mesh.until(&execution, "parked its redispatch", |report| {
        report["placement_waits"]
            .as_array()
            .is_some_and(|waits| waits.iter().any(|wait| wait["status"] == "parked"))
    });
    assert_eq!(
        parked["status"], "running",
        "an execution waiting for a machine is running, not interrupted: {parked:#}"
    );
    assert_eq!(
        parked["placement_waits"][0]["status"], "parked",
        "the redispatch was handed to a session that is gone: {parked:#}"
    );

    // …and the wake is a join.
    let fresh = mesh.worker("escalation-answering");
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["approval"],
        "approved after the mac left",
        "the fresh worker did not finish the answered node; it said:\n{}",
        fresh.transcript()
    );
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the worker that took the redispatch re-issued the call the first one made: {requests:#?}"
    );
}

/// The three calls `agent.escalator` makes when it asks the question with a
/// **budget** on it, and nobody answers.
///
/// The same three as [`escalating`] — the loop's request for the attached flow,
/// the loop's answer once the tool has returned, and the pinned structured
/// output that ends the node — with the first naming the *other* attached flow.
/// Which of the two questions a run reaches is this queue's to decide, which is
/// what lets one agent, one trigger and one placement serve both endings of a
/// wait.
fn expiring(approval: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "deadline",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("nobody was at the machine")),
        Script::new(SONNET, Outcome::structured(json!({ "approval": approval }))),
    ]
}

/// A placed pause **runs out of time**, and the redispatch takes the node's own
/// `on_timeout:` route on a worker (§3.4, grammar §8.7, PRD resolved q46).
///
/// The other ending of a wait, end to end and in real processes: the question is
/// asked on the worker, the wait is planted on the hub's board with the budget
/// `flow.deadline`'s `ask` declares, nobody answers it, the hub journals the
/// expiry as the pause's own record, and the node re-enters dispatch carrying
/// it. What the replay then does is the whole of the parity claim — it raises
/// the `HumanExpiry` the *composition's* `on_timeout:` routes, at the same line
/// of the same function an unplaced pause's expiry is raised at — and the proof
/// is `lapse`, a node no edge targets: a value it wrote reached the model, so
/// the route was taken rather than the question answered.
///
/// The wait is deliberately **not** read off the board here. Its budget is two
/// seconds of real time and this test starts two processes, so a report that
/// caught the question open would be a race a test should not run; the wire
/// suite asserts the publication, and what is asserted here is what only real
/// processes can show.
#[test]
fn a_placed_pause_that_runs_out_of_time_takes_its_on_timeout_route_on_a_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(expiring("nobody approved it"));
    let worker = mesh.worker("escalation-expiring");

    let execution = mesh.start_execution("/escalations", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["approval"],
        "nobody approved it",
        "the node did not finish after its wait expired; the worker said:\n{}",
        worker.transcript()
    );

    // **The expiry is journaled as the pause's own record**, which is what the
    // redispatch replayed: one `human` effect, settled `expired`, carrying no
    // answer — a wait nobody answered may not journal one.
    let human = harness::journal_rows(
        &mesh.project,
        "SELECT key, payload FROM effects WHERE kind = 'human'",
    );
    let records = human.as_array().expect("the query answers rows");
    assert_eq!(
        records.len(),
        1,
        "the expiry is not journaled once as a `human` effect: {human:#}"
    );
    let payload = records[0]["payload"].as_str().unwrap_or_default();
    assert!(
        payload.contains("\"settled\":\"expired\""),
        "the wait was journaled as something other than an expiry: {human:#}"
    );
    assert!(
        !payload.contains("\"output\""),
        "an expired wait was journaled with an answer nobody gave: {human:#}"
    );
    assert!(
        payload.contains("\"expiresAt\""),
        "the record does not say when the budget ran out: {human:#}"
    );

    // …and the node went back through dispatch to reach its route: two settled
    // rows at one instance path, the second of which is the one that replayed
    // the expiry.
    let board = harness::journal_rows(
        &mesh.project,
        "SELECT status, placement, node FROM dispatches ORDER BY rowid",
    );
    assert_eq!(
        board,
        json!([
            { "status": "settled", "placement": PLACEMENT, "node": "flow.escalated.escalate" },
            { "status": "settled", "placement": PLACEMENT, "node": "flow.escalated.escalate" },
        ]),
        "the expired pause did not send the node back through dispatch: {board:#}"
    );

    // **`on_timeout:` routed, and it routed on the worker.** `lapse` is reached
    // by no edge, so the only way its value exists is the route the replayed
    // expiry raised — and it comes back to the model as the attached flow's
    // answer, which is where this test can read it.
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the redispatch re-issued a model call the journal already held: {requests:#?}"
    );
    let said = serde_json::to_string(&requests[1]).expect("a request serializes");
    assert!(
        said.contains("nobody answered in time"),
        "the attached flow answered without taking its `on_timeout:` route, so the expiry ended \
         the node rather than routing it: {said}"
    );
    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained(),
        "the escalating agent did not make the three calls it was scripted: {snapshot:#?}"
    );
}

/// The **one interval a mesh adds** to a wait, and the one place a placed pause
/// is not indistinguishable from a local one (§3.4, §6.5, PRD resolved q46).
///
/// A settled wait — answered or expired — starts the dispatching node's budget
/// running again, and what runs next is a redispatch waiting for a session to
/// claim it. §6.5 puts that queueing *inside* the budget, which is what "fail if
/// the machine is not up in ten minutes" means; so a placement nobody is
/// claiming when the wait settles costs the node its budget, and the pause's own
/// `on_timeout:` route is never reached — where the same node unplaced takes
/// that route in the same process, in the instant the wait expires.
///
/// It is a divergence rather than a parity, which is why it is written down
/// rather than left to be met: an author reading `on_timeout: lapse` beside
/// `timeout: 15s` should be able to find out that the second can eat the first.
/// The two halves were each documented and their interaction was not, and this
/// is the test that keeps the sentence honest.
///
/// `flow.impatient` exists for the budget: `flow.escalated`'s node takes the
/// fixture's minute, which no test can wait out. Everything else here is that
/// flow's own — the same agent, the same attached `flow.deadline`, and the same
/// two-second question.
#[test]
fn a_placed_pauses_route_is_lost_when_the_node_spends_its_budget_waiting_for_a_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    // The **first** of the same three calls, and the only one this run reaches:
    // the loop asks for `flow.deadline`, the question is opened on the worker,
    // and the node never comes back to be told what happened.
    let mut scripted = expiring("nobody approved it");
    scripted.truncate(1);
    mesh.provider.enqueue_all(scripted);
    let mut asked = mesh.worker("escalation-impatient");

    let execution =
        mesh.start_execution("/impatient-escalations", &json!({ "path": "release.dmg" }));
    // The question comes home, which holds the node's budget still…
    mesh.paused(&execution);
    // …and the machine that asked it goes away, so nothing will claim the
    // redispatch the expiry makes.
    asked.kill();

    let ended = mesh.until(&execution, "ended", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        ended["status"], "failed",
        "the node outlived the budget it was given while its redispatch sat on the board: \
         {ended:#}"
    );
    let said = ended["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("timed out") && said.contains("15000ms"),
        "the execution ended on something other than the dispatching node's own budget, which is \
         what §6.5 says bounds waiting for a machine: {ended:#}"
    );

    // **The wait really did expire**, so what was lost is the route and not the
    // question: one `human` record, settled `expired`, exactly as in the test
    // above.
    let human = harness::journal_rows(
        &mesh.project,
        "SELECT payload FROM effects WHERE kind = 'human'",
    );
    let records = human.as_array().expect("the query answers rows");
    assert_eq!(
        records.len(),
        1,
        "the wait did not settle, so this run never reached the interval under test: {human:#}"
    );
    assert!(
        records[0]["payload"]
            .as_str()
            .unwrap_or_default()
            .contains("\"settled\":\"expired\""),
        "the wait ended some other way: {human:#}"
    );

    // …and the redispatch was made and then given up on, which is the interval
    // itself: a second row at the same instance path, superseded by the hub
    // rather than settled by a worker.
    let board = harness::journal_rows(
        &mesh.project,
        "SELECT status, node FROM dispatches ORDER BY rowid",
    );
    assert_eq!(
        board,
        json!([
            { "status": "settled", "node": "flow.impatient.escalate" },
            { "status": "superseded", "node": "flow.impatient.escalate" },
        ]),
        "the expiry did not redispatch, or the redispatch was not the row the budget ended: \
         {board:#}"
    );

    // **`lapse` was never reached**, on either side of the wire: no second model
    // call carrying its value back to the loop, and nothing written to the
    // channel it writes.
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        1,
        "the flow got past the question it never had a worker to route: {requests:#?}"
    );
    assert_ne!(
        ended["outputs"]["approval"],
        json!("nobody answered in time"),
        "the `on_timeout:` route ran without a worker to run it on: {ended:#}"
    );
}

/// The **hub** goes away between the question and the answer, and the wait comes
/// back (§5, `docs/durability.md` §6).
///
/// The pause is journaled — it is what settled the dispatch — so the process
/// that replaces this one re-derives the wait rather than remembering it, under
/// the identity its predecessor published and dated by its own planting. That is
/// the same discipline every other open wait is recovered by, reaching the one
/// kind of wait that was opened in another process entirely.
///
/// The wait this flow opens declares no `timeout:`, so nothing here is about a
/// budget; `distributed_hub_wire.rs` is where the re-derived budget and the
/// deadline published beside it are asserted.
#[test]
fn a_placed_pause_is_re_derived_by_a_hub_restarted_before_the_answer() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue_all(escalating("approved across a restart"));
    let mut asked = mesh.worker("escalation-before-restart");

    let execution = mesh.start_execution("/escalations", &json!({ "path": "release.dmg" }));
    let before = mesh.paused(&execution);
    let id = before["wait_id"]
        .as_str()
        .expect("the wait names itself")
        .to_string();

    // Both processes of the pause go: the one that asked, and the one holding
    // the question. Nothing is left in memory anywhere.
    asked.kill();
    mesh.restart();

    // The replay reaches the placed node, finds the row its predecessor settled
    // paused, and re-derives the wait — **under the same identity**, which is
    // what lets a resume prepared against the dead process still land.
    let after = mesh.until(&execution, "re-derived its pause", |report| {
        report["interrupts"]
            .as_array()
            .is_some_and(|waits| !waits.is_empty())
    });
    assert_eq!(after["interrupts"][0]["wait_id"], json!(id), "{after:#}");
    // …and **re-dated by the planting**, exactly as a re-parked local wait is
    // (`docs/durability.md` §3.4, §5): the generation holding a question is the
    // one that dates it, so the pair a reader is shown is this process's and
    // never a mixture of two.
    assert!(
        after["interrupts"][0]["paused_at"].as_str() > before["paused_at"].as_str(),
        "the re-derived wait kept the instant its predecessor published rather than the one this \
         planting dated it at, so a restart leaves a pair no single clock read: {after:#}"
    );

    let fresh = mesh.worker("escalation-after-restart");
    let answered = mesh.resume(&execution, &id, &json!({ "decision": "approve" }));
    assert_eq!(answered.status, 202, "{}", body_of(&answered));

    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["approval"],
        "approved across a restart",
        "the recovered execution did not finish after the answer; the worker said:\n{}",
        fresh.transcript()
    );
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the restart cost the execution a model call it had already paid for: {requests:#?}"
    );
}

// ---------------------------------------------------------------------------
// 9. A journal written before the board
// ---------------------------------------------------------------------------

/// The statement the surgery below makes about a journal, which is the shape of
/// the file a build **before** the dispatch board wrote
/// (`docs/durability.md` §3.8, §11.2).
///
/// `DROP TABLE` without `IF EXISTS` refuses a file that has no such table, so a
/// build whose journal is not what this describes fails here rather than passing
/// over an assumption that had quietly stopped being true.
const JOURNAL_BEFORE_THE_DISPATCH_BOARD: &str = "DROP TABLE dispatches;\n";

/// A journal written before the dispatch board existed opens under this build,
/// replays the execution it holds, and takes this build's own board
/// (`docs/durability.md` §11.2).
///
/// `JOURNAL_VERSION` did not move when the board arrived, and §11.2 is where the
/// argument for that lives: the table is created on first open, as
/// `CREATE TABLE IF NOT EXISTS`, and a dispatch row holds no §4 key and is never
/// consumed by a replay — so an execution open in an older file replays
/// identically and its frontier does not move. Until this test that argument was
/// made only in prose, and every other test in the suite creates its journal
/// fresh under the current schema, so a `NOT NULL` column added to `dispatches`
/// without the `PRAGMA table_info` probe the migrations beside it use would
/// leave `cargo test` green and every deployed hub unable to park a dispatch.
/// §11.2's own delivery-ledger test is the sibling this is one of.
///
/// There is no older build in the tree to write the file, so the file is made: a
/// real hub's journal, parked on a placed node, with the `dispatches` table
/// dropped out from under it. What is then asserted is the three halves of the
/// claim — it opens, the execution it holds runs to the end over the wire, and
/// this build's board is written into the same file.
#[test]
fn a_journal_written_before_the_dispatch_board_opens_and_serves_under_this_build() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue_all(signing("signed-over-a-journal-that-had-no-board"));

    // An execution open **at** a placed node, and no worker: the row on the
    // board is exactly the kind a build before the board could not have written.
    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    mesh.until(&execution, "parked on its placement", |report| {
        !report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty)
    });

    // The process goes down, and the board is taken out of the file it left.
    // Stopped first, because the surgery is a writer and so is the hub.
    mesh.served.stop();
    harness::journal_sql(&mesh.project, JOURNAL_BEFORE_THE_DISPATCH_BOARD);
    mesh.restart();

    // It opened. The execution it holds is recovered, reaches the placed node
    // again, and — finding no row where its predecessor left one — parks a fresh
    // wait, which is what a worker then answers.
    let worker = mesh.worker("older-journal");
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        "signed-over-a-journal-that-had-no-board",
        "the recovered execution did not finish over the older journal; the worker said:\n{}",
        worker.transcript()
    );
    assert_eq!(
        outputs["ticket"], "notarized",
        "the hub's own node did not run in the recovered execution: {outputs:#}"
    );

    // …and the board this build writes is in the file that had none: a settled
    // row, for the wait this run really parked.
    let board = harness::journal_rows(
        &mesh.project,
        "SELECT status, placement, node FROM dispatches WHERE status = 'settled'",
    );
    let settled = board.as_array().expect("the query answers rows");
    assert_eq!(
        settled.len(),
        1,
        "this build wrote no settled dispatch into the journal it opened: {board:#}"
    );
    assert_eq!(settled[0]["placement"], PLACEMENT, "{board:#}");
    assert_eq!(settled[0]["node"], "flow.release.sign", "{board:#}");
}

/// **The hub exports a mesh execution's whole trace, the placed node's entries
/// included** (grammar §14.5, PRD resolved q50, q51).
///
/// Resolved q51 makes the export **hub-side only**, and says why: "the hub owns
/// the trace, workers stream collectors home already, so a mesh needs no
/// per-worker OTel story". This is that claim executed rather than asserted — a
/// real worker runs the placed node in a process of its own, and what the
/// collector receives is one export from one address, carrying the entries the
/// worker produced beside the ones the hub did.
///
/// The trace sink reaches this fixture through a **copy** of it: the collector's
/// port is the operating system's, so the target is written beside the
/// composition rather than committed in it (see [`harness::staged_with_deploy`]).
/// The rest of `deploy/mesh.yml` is the fixture's own, read and re-written
/// verbatim, so what this serves is the mesh every other test here serves plus
/// one key.
#[test]
fn a_mesh_execution_exports_its_whole_trace_from_the_hub() {
    let collector = harness::Receiver::start().expect("a loopback collector");
    let target = format!(
        "{}\ntrace_sink:\n  url: \"{}/v1/traces\"\n",
        std::fs::read_to_string(
            harness::fixture(FIXTURE)
                .parent()
                .expect("a fixture has a directory")
                .join(format!("deploy/{TARGET}.yml")),
        )
        .expect("the fixture's mesh target is readable"),
        collector.base_url,
    );
    let (composition, entrypoint) =
        harness::staged_with_deploy("mesh-trace-sink", FIXTURE, TARGET, &target);
    let Some(mesh) = Mesh::start_from("mesh-trace-sink", entrypoint, Some(composition)) else {
        return;
    };
    mesh.provider
        .enqueue_all(signing_after_a_command("signed-for-the-collector"));
    let worker = mesh.worker("exporting");

    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"], "signed-for-the-collector",
        "{outputs:#}"
    );

    let exported = collector.wait_for_event("settled", 1, Duration::from_secs(60));
    let body = &exported[0].body;
    assert_eq!(body["execution_id"], execution, "{body:#}");
    assert_eq!(body["status"], "completed", "{body:#}");
    assert_eq!(body["trace_version"], 7, "{body:#}");

    // The placed node's entry is in the hub's export, which is the whole of
    // "the hub owns the trace": `sign` ran in the worker's process, and its
    // record came home on the result the worker posted (§3.4).
    let nodes: Vec<&str> = body["entries"]
        .as_array()
        .expect("an envelope carries entries")
        .iter()
        .filter_map(|entry| entry["node"].as_str())
        .collect();
    assert!(
        nodes.contains(&"sign"),
        "the placed node's entry is missing from the hub's export: {body:#}"
    );
    assert!(
        nodes.contains(&"stamp"),
        "the hub's own node is missing from its export: {body:#}"
    );

    // …and the model calls the **worker** made are on that entry, which is what
    // says the export carries the trace rather than a summary of it.
    let signed = body["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|entry| entry["node"] == "sign")
        .expect("the placed node's entry");
    assert!(
        signed["models"]
            .as_array()
            .is_some_and(|calls| !calls.is_empty()),
        "the worker's model calls did not reach the hub's export: {signed:#}"
    );

    // …and the **program** a built-in ran there, which is the mesh half of PRD
    // resolved q54: a placed agent runs model-authored commands on the worker
    // that took its dispatch, and the record of what it ran comes home on the
    // same result as everything else the node did (`docs/trace.md` §7.4,
    // `docs/distributed.md` §3.4). A mesh that carried the model calls and lost
    // this would leave the one machine an operator most wants an account of
    // with no account of the command it ran.
    let programs: Vec<&Value> = signed["models"]
        .as_array()
        .expect("the placed node's model calls")
        .iter()
        .flat_map(|call| call["toolCalls"].as_array().into_iter().flatten())
        .filter_map(|call| call.get("program"))
        .collect();
    assert_eq!(
        programs,
        vec![&json!({ "tool": "bash", "command": "pwd", "exitCode": 0 })],
        "the placed agent's `builtin.bash` call is missing from the hub's trace, or          carries something other than what the model ran: {signed:#}"
    );

    // …and it ran **on the worker**, which is the other half of the mesh claim
    // and the half a program record cannot make on its own: the command is the
    // same string whichever process executed it. What differs is where it
    // printed from — the default workspace is one directory under the *running*
    // process's data root (grammar 6.1), and the worker's is the `--data-dir`
    // this test gave it, a directory the hub has never heard of. So the answer
    // the model was handed names that directory, and a build that hoisted a
    // placed agent's built-in calls back to the hub would hand it the hub
    // project's path instead and fail here.
    let ran_under = std::fs::canonicalize(&worker.data_dir)
        .expect("the worker's data directory")
        .display()
        .to_string();
    let hub_side = std::fs::canonicalize(&mesh.project)
        .expect("the hub's project directory")
        .display()
        .to_string();
    let transcript: Vec<String> = mesh
        .provider
        .requests()
        .iter()
        .map(|asked| asked.body().to_string())
        .collect();
    assert!(
        transcript.iter().any(|body| body.contains(&ran_under)),
        "the placed agent's command printed a working directory that is not under the \
         worker's own data directory `{ran_under}`, so it did not run on the worker: \
         {transcript:#?}"
    );
    assert!(
        !transcript.iter().any(|body| body.contains(&hub_side)),
        "the placed agent's command printed a working directory under the **hub's** \
         project `{hub_side}`: a placed built-in runs inside the dispatch, on the machine \
         that took it (`docs/distributed.md` §3.2): {transcript:#?}"
    );

    // **…and from nowhere else**, which is the half resolved q51's "export is
    // hub-side only" turns on and the half the wait above cannot give: it
    // returns on the *first* arrival, so a second exporter's POST would land
    // milliseconds later and leave this test green. A worker that exported would
    // be reaching this same collector — it runs the same artifact under the same
    // target — so the claim is made after a window wide enough for its request
    // to have arrived, and from both ends:
    //
    //  * the collector holds **one** `settled` delivery id. Counted as ids
    //    rather than as requests for [`harness::Receiver::distinct`]'s reason:
    //    delivery is at-least-once, so a repeat under one id is the contract and
    //    a second id is a second export;
    //  * and the hub's journal — the mesh's only delivery ledger, since a worker
    //    refuses the table outright (`src/worker-node.ts`'s
    //    `WorkerJournalReach`) — holds **one** `trace_sink` row, which is the
    //    half a collector cannot show: an intent journaled and not yet sent is
    //    an export still owed.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        collector.distinct("settled"),
        [format!("{execution}:0")],
        "a second export reached the collector, so this mesh exports from somewhere besides \
         the hub: {:?}",
        collector.delivered()
    );
    let rows = harness::journal_rows(
        &mesh.project,
        "SELECT kind, status FROM deliveries WHERE kind = 'trace_sink' ORDER BY ordinal ASC",
    );
    let held = rows.as_array().expect("the query answers rows");
    assert_eq!(
        held.len(),
        1,
        "one export intent per settled execution: {rows:#}"
    );
    assert_eq!(
        held[0]["status"], "delivered",
        "the hub's one export is still owed, so the collector's delivery came from somewhere \
         else: {rows:#}"
    );
}

// ---------------------------------------------------------------------------
// 10. A session's capacity, and a cancel over the wire (PRD resolved q75, q79)
// ---------------------------------------------------------------------------

/// The fixture, staged with `max_running: <cap>` on its `mac` placement.
fn capped(purpose: &str, cap: u32) -> Option<Mesh> {
    let deploy = std::fs::read_to_string(
        harness::fixture(FIXTURE)
            .parent()
            .expect("a fixture directory")
            .join("deploy")
            .join(format!("{TARGET}.yml")),
    )
    .expect("the fixture's mesh target is readable");
    let deploy = deploy.replacen("  mac:\n", &format!("  mac:\n    max_running: {cap}\n"), 1);
    assert!(
        deploy.contains(&format!("max_running: {cap}")),
        "the placement was not capped: {deploy}"
    );
    let (composition, entrypoint) = harness::staged_with_deploy(purpose, FIXTURE, TARGET, &deploy);
    Mesh::start_from(purpose, entrypoint, Some(composition))
}

/// The two model calls one item of `flow.batch` makes — a turn of its loop,
/// then the pinned structured answer, held for `held` so the item is in a
/// worker's hands for that long.
///
/// Each is matched to **its** item's call rather than queued: items run at
/// once here, and a queue would hand one item's answer to another's call. The
/// loop's turn carries the item's path; the answer's request carries the turn
/// it said, which no other item's request does.
fn held_signature(path: &str, held: Duration) -> [Script; 2] {
    let said = format!("thinking about {path}");
    [
        Script::new(SONNET, Outcome::text(said.clone())).matching(path),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": format!("signed {path}") })).after(held),
        )
        .matching(said),
    ]
}

/// Sample an execution's report while it runs, answering the most of its
/// placement waits that were ever `dispatched` at once.
fn most_dispatched_at_once(mesh: &Mesh, execution: &str, cap: usize) -> usize {
    let watching = Instant::now() + Duration::from_secs(60);
    let mut most = 0usize;
    loop {
        let report = mesh.report(execution);
        if report["status"] != "running" || Instant::now() > watching {
            return most;
        }
        let dispatched = report["placement_waits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|wait| wait["status"] == "dispatched")
            .count();
        assert!(
            dispatched <= cap,
            "{dispatched} items were dispatched at once onto a pool whose caps sum to {cap} \
             (docs/distributed.md §2, Decision D161): {report:#}"
        );
        most = most.max(dispatched);
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// **Two workers at a cap of one run at most two items at once** (§2, PRD
/// resolved q79, Decision D161).
///
/// A placement that declares no `max_running:` holds one dispatch per session,
/// so a pool of two runs a five-item fan-out two at a time — never three, and,
/// with each item held long enough to be seen, two at some point.
#[test]
fn two_workers_at_a_cap_of_one_run_at_most_two_items_at_once() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    for path in ["a.dmg", "b.dmg", "c.dmg", "d.dmg", "e.dmg"] {
        mesh.provider
            .enqueue_all(held_signature(path, Duration::from_millis(1500)));
    }
    let _first = mesh.worker("pool-of-two-a");
    let _second = mesh.worker("pool-of-two-b");
    let execution = mesh.start_execution(
        "/batches",
        &json!({ "paths": ["a.dmg", "b.dmg", "c.dmg", "d.dmg", "e.dmg"] }),
    );
    let most = most_dispatched_at_once(&mesh, &execution, 2);
    assert_eq!(most, 2, "a pool of two never ran two items at once");
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signatures"].as_array().map(Vec::len),
        Some(5),
        "{outputs:#}"
    );
}

/// **One worker whose placement says `max_running: 3` runs three at once**
/// (§2, PRD resolved q79, Decision D161).
///
/// The same fan-out, three items, on a pool of one worker: the session holds
/// all three, the worker runs them side by side — each item's call is held
/// three seconds, so three dispatched at once is three running at once — and
/// the fan-out completes.
#[test]
fn one_worker_whose_placement_says_max_running_three_runs_three_at_once() {
    let Some(mesh) = capped("mesh-capacity-three", 3) else {
        return;
    };
    for path in ["a.dmg", "b.dmg", "c.dmg"] {
        mesh.provider
            .enqueue_all(held_signature(path, Duration::from_secs(3)));
    }
    let _worker = mesh.worker("capacity-three");
    let started = Instant::now();
    let execution =
        mesh.start_execution("/batches", &json!({ "paths": ["a.dmg", "b.dmg", "c.dmg"] }));
    let most = most_dispatched_at_once(&mesh, &execution, 3);
    assert_eq!(most, 3, "one session at a cap of three never held three");
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signatures"].as_array().map(Vec::len),
        Some(3),
        "{outputs:#}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(8),
        "three items held three seconds each took {:?}: the worker ran them one after the \
         other",
        started.elapsed()
    );
}

/// **A cancel stops a placed node on its worker** and settles the execution
/// `cancelled` (PRD resolved q75 ruling b, `docs/distributed.md` §3.2, §3.4).
///
/// `flow.retried`'s `sign` is placed on `mac` and declares a `retry:`. Its first
/// model call answers and is journaled; its second is held for ten minutes —
/// the worker is in the middle of the node. The cancel reaches the worker on
/// its next poll, the worker stops the runner and posts the cancelled ending,
/// and the row settles `cancelled` within ten seconds, with the node's entry
/// `cancelled` and no `error`. No retry ran — the provider was asked twice,
/// never a third time — the held call the cancel aborted wrote no record
/// (`docs/durability.md` §6.1), and the worker is still serving.
///
/// And the worker is what stopped it: the hub settles the node — and so the
/// execution — only once the worker's `status: "cancelled"` result is home,
/// because a cancelled node waits for what it began, and a placed node began
/// it on the worker (Decision D160). So by the time the report says
/// `cancelled` the journal's dispatch row is already settled with the
/// `ExecutionCancelled` outcome that only that result writes (§3.4), and the
/// runner process the worker started for the dispatch is gone — both inside
/// the ten seconds.
#[test]
fn a_cancel_stops_a_placed_node_on_its_worker_and_settles_the_execution_cancelled() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all([
        Script::new(SONNET, Outcome::text("thinking about it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "never delivered" }))
                .after(Duration::from_secs(600)),
        ),
    ]);
    let mut worker = mesh.worker("cancelled-mid-node");
    let execution = mesh.start_execution("/retried-releases", &json!({ "path": "release.dmg" }));
    until(
        "the first model call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0"]),
    );
    mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    let running = worker.runners();
    if let Some(running) = &running {
        assert_eq!(
            running.len(),
            1,
            "one runner for the one dispatch: {running:?}"
        );
    }

    let asked = Instant::now();
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));
    let report = mesh.until(&execution, "was cancelled", |report| {
        report["status"] == "cancelled"
    });
    // Already settled — the execution waited for it.
    mesh.settled_cancelled(&execution, 1, Duration::ZERO);
    for pid in running.into_iter().flatten() {
        assert!(
            gone_within(pid, Duration::from_secs(10).saturating_sub(asked.elapsed())),
            "the runner {pid} the worker started for the cancelled dispatch is still alive: {}",
            worker.transcript()
        );
    }
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to settle a placed node",
        asked.elapsed()
    );
    assert!(report.get("error").is_none(), "{report:#}");
    let sign = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "sign")
        .unwrap_or_else(|| panic!("the placed node has an entry: {report:#}"));
    assert_eq!(sign["outcome"], "cancelled", "{sign:#}");
    assert!(sign.get("error").is_none(), "{sign:#}");
    assert_eq!(
        mesh.provider.requests().len(),
        2,
        "a cancelled placed node was retried"
    );
    // The call the cancel aborted wrote no record (`docs/durability.md` §6.1):
    // the runner's effects are posted ahead of its result, which settled the
    // row above, so a record it had kept would be in the journal by now.
    let aborted = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, payload FROM effects \
             WHERE execution = '{execution}' AND \"key\" = 'sign/0#model/1'"
        ),
    );
    assert_eq!(
        aborted,
        json!([]),
        "the model call the cancel aborted was journaled as an answer"
    );
    assert!(
        worker.running(),
        "the worker stopped with the node it was asked to cancel: {}",
        worker.transcript()
    );
}

/// **A cancel reaches a worker holding two of an execution's dispatches**
/// (PRD resolved q75, q79; `docs/distributed.md` §3.2).
///
/// At `max_running: 2` one worker holds both items of a two-item fan-out, each
/// in the middle of a ten-minute call. The cancel names both in the worker's
/// next poll's `cancel` list, the worker stops both runners, and both cancelled
/// results settle the row `cancelled` — read off the journal's dispatch rows,
/// each settled with the outcome only the worker's result writes, and off the
/// runner processes, both gone.
#[test]
fn a_cancel_reaches_a_worker_holding_two_of_its_dispatches() {
    let Some(mesh) = capped("mesh-cancel-two", 2) else {
        return;
    };
    for path in ["a.dmg", "b.dmg"] {
        mesh.provider
            .enqueue_all(held_signature(path, Duration::from_secs(600)));
    }
    let mut worker = mesh.worker("cancel-two");
    let execution = mesh.start_execution("/batches", &json!({ "paths": ["a.dmg", "b.dmg"] }));
    mesh.until(&execution, "had both items dispatched", |report| {
        report["placement_waits"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|wait| wait["status"] == "dispatched")
            .count()
            == 2
    });
    // Both runners are up once both dispatches are in the worker's hands.
    let running = worker.runners().map(|_| {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            let held = worker.runners().unwrap_or_default();
            if held.len() == 2 || Instant::now() > deadline {
                break held;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    if let Some(running) = &running {
        assert_eq!(
            running.len(),
            2,
            "two runners for two dispatches: {running:?}"
        );
    }
    let asked = Instant::now();
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));
    let report = mesh.until(&execution, "was cancelled", |report| {
        report["status"] == "cancelled"
    });
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "{:?}",
        asked.elapsed()
    );
    assert!(report.get("error").is_none(), "{report:#}");
    mesh.settled_cancelled(
        &execution,
        2,
        Duration::from_secs(10).saturating_sub(asked.elapsed()),
    );
    for pid in running.into_iter().flatten() {
        assert!(
            gone_within(pid, Duration::from_secs(10).saturating_sub(asked.elapsed())),
            "a runner the worker started for a cancelled dispatch is still alive: {}",
            worker.transcript()
        );
    }
    assert!(worker.running(), "{}", worker.transcript());
}

/// **A restart while a worker holds a cancelled execution's dispatch
/// completes the cancel** (PRD resolved q75 ruling e, `docs/distributed.md`
/// §5, `docs/durability.md` §6.1).
///
/// `flow.release`'s `sign` declares no `retry:` and its flow `on_error: fail`,
/// so an attempt read as failed ends the execution `failed`. The worker is in
/// the middle of the node; the hub is killed, and the journal is left as a
/// crash right after the cancel route recorded its request would leave it —
/// the request on the `cancellations` table, the dispatch still `dispatched`
/// to a session the dead process issued. The hub that replaces it does not
/// read that row as an orphan: the replay reaches it as the frontier of the
/// cancel, so the node is cancelled, the execution settles `cancelled` with no
/// `error`, and the row is withdrawn as the cancel's rather than superseded as
/// a lost session's.
#[test]
fn a_restart_while_a_worker_holds_a_cancelled_dispatch_completes_the_cancel() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all([
        Script::new(SONNET, Outcome::text("thinking about it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "never delivered" }))
                .after(Duration::from_secs(5)),
        ),
    ]);
    let mut worker = mesh.worker("restart-mid-cancel");
    let execution = mesh.start_execution("/releases", &json!({ "path": "release.dmg" }));
    until(
        "the first model call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["sign/0#model/0"]),
    );
    mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });

    mesh.restart_after(|project| {
        let wrote = harness::journal_sql(
            project,
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
    });

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "cancelled" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"], "cancelled",
        "a restart read the cancelled execution's held dispatch as an orphan: {report:#}"
    );
    assert!(report.get("error").is_none(), "{report:#}");
    let sign = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "sign")
        .unwrap_or_else(|| panic!("the placed node has an entry: {report:#}"));
    assert_eq!(sign["outcome"], "cancelled", "{sign:#}");
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(rows[0]["status"], "superseded", "{rows:#?}");
    assert!(
        rows[0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("was cancelled")),
        "the held dispatch was ended as an orphan rather than as the cancel's: {rows:#?}"
    );
    assert!(worker.running(), "{}", worker.transcript());
}

/// What the resume route answers for an execution somebody cancelled
/// (`src/serve.ts`'s `CANCELLED_RESUME`, PRD resolved q75 ruling e).
const CANCELLED_RESUME: &str = "this execution was cancelled, so nothing is waiting for an \
                                answer: a cancelled execution is ended for good and is never \
                                resumed";

/// Assert that a cancelled execution holds no wait any more, that its placed
/// node's entry says `cancelled`, and that an answer to the wait it held is
/// refused naming the status.
fn assert_released_and_refused(mesh: &Mesh, execution: &str, wait: &str, report: &Value) {
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(
        report["interrupts"].as_array().is_none_or(Vec::is_empty),
        "a cancelled execution still publishes the question it was waiting on: {report:#}"
    );
    assert!(
        report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a cancelled execution still holds a placement wait: {report:#}"
    );
    let escalate = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "escalate")
        .unwrap_or_else(|| panic!("the placed node has an entry: {report:#}"));
    assert_eq!(escalate["outcome"], "cancelled", "{escalate:#}");
    assert!(escalate.get("error").is_none(), "{escalate:#}");
    let refused = mesh.resume(execution, wait, &json!({ "decision": "approve" }));
    assert_eq!(refused.status, 409, "{}", body_of(&refused));
    assert_eq!(
        refused.json(),
        json!({
            "execution_id": execution,
            "status": "cancelled",
            "error": CANCELLED_RESUME,
        }),
        "the answer to a released wait was not refused naming the cancel"
    );
}

/// **A cancel releases a pause a placed agent brought home** (PRD resolved q75
/// ruling e: "human and placement waits are released"; Decision D160,
/// `docs/distributed.md` §3.4).
///
/// `flow.escalated`'s placed agent reaches `flow.escalation`'s `ask` on the
/// worker; the dispatch settles paused and the question is planted on the
/// hub's board. A cancel while it waits answers `202`, abandons the wait — the
/// report publishes no question, the placed node's entry is `cancelled` — and
/// settles the row `cancelled` inside the field report's ten seconds, after
/// which the answer the question was waiting for is refused naming the status.
/// The node is not redispatched: one dispatch row, one model call.
#[test]
fn a_cancel_releases_a_pause_a_placed_agent_brought_home_and_settles_cancelled() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(escalating("never decided"));
    let mut worker = mesh.worker("cancel-at-a-remote-pause");
    let execution = mesh.start_execution("/escalations", &json!({ "path": "release.dmg" }));
    let wait = mesh.paused(&execution);
    let id = wait["wait_id"].as_str().expect("the wait names itself");

    let asked = Instant::now();
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "interrupted" && report["status"] != "running"
    });
    assert_eq!(
        report["status"], "cancelled",
        "a cancel of an execution parked at a worker's question did not end it cancelled: \
         {report:#}"
    );
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to settle an execution parked at a pause",
        asked.elapsed()
    );
    assert_released_and_refused(&mesh, &execution, id, &report);
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.len(),
        1,
        "the cancelled node was dispatched again: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "a cancelled node made a model call past its pause"
    );
    assert!(worker.running(), "{}", worker.transcript());
}

/// **A hub restarted mid-cancel while a placed agent's pause waits completes
/// the cancel** (PRD resolved q75 ruling e, `docs/durability.md` §6.1).
///
/// The journal is left as a `SIGKILL` right after the cancel route recorded its
/// request would leave it: the request on the `cancellations` table, the
/// dispatch settled paused with no answer recorded — an abandoned remote pause
/// writes nothing. The recovered replay reaches the placed node and finds the
/// unanswered pause, which is a frontier exactly as a local wait's claim is:
/// the cancel fires there instead of the question going back on the board, so
/// the row settles `cancelled` with no wait published and the answer is
/// refused naming the status. Re-planted instead, the wait would refuse every
/// answer as "being cancelled" while nothing ever ended it.
#[test]
fn a_hub_restarted_mid_cancel_at_a_placed_pause_completes_the_cancel() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(escalating("never decided"));
    let mut asked = mesh.worker("restart-mid-cancel-at-a-pause");
    let execution = mesh.start_execution("/escalations", &json!({ "path": "release.dmg" }));
    let wait = mesh.paused(&execution);
    let id = wait["wait_id"]
        .as_str()
        .expect("the wait names itself")
        .to_string();
    asked.kill();

    mesh.restart_after(|project| {
        let wrote = harness::journal_sql(
            project,
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
    });

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "cancelled" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"], "cancelled",
        "the recovered execution did not complete its cancel: {report:#}"
    );
    assert_released_and_refused(&mesh, &execution, &id, &report);
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.len(),
        1,
        "the recovery dispatched the cancelled node again: {rows:#?}"
    );
    assert_eq!(rows[0]["status"], "settled", "{rows:#?}");
    let human = harness::journal_rows(
        &mesh.project,
        "SELECT key FROM effects WHERE kind = 'human'",
    );
    assert_eq!(
        human,
        json!([]),
        "the released pause was journaled as answered: {human:#}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the recovery made a model call past the cancelled pause"
    );
}

/// The one call `agent.closer` makes on the worker: the attached flow, whose
/// body outlasts every test — so nothing past it is scripted.
fn closing() -> Vec<Script> {
    vec![Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "closing",
            json!({ "path": "release.dmg" }),
        )]),
    )]
}

/// **A cancel on a worker lets the finally node it started run whole** (grammar
/// §7.9: "a cancel stops none of it, and nothing it begins"; PRD resolved q75,
/// q76, Decision D160, `docs/distributed.md` §3.2, §3.4).
///
/// `flow.closed`'s placed agent attaches `flow.closing`, whose body is a
/// ten-minute command and whose finally node takes eight seconds and then
/// fails. The cancel lands while the body runs: the worker sends the runner
/// `SIGTERM`, the runner cancels, the attached instance ends and starts its
/// finally node — and the worker's five-second grace does **not** kill it,
/// because the runner reports finally work under way. So past the grace the
/// runner is still alive and the dispatch still held; and when the row settles,
/// by the worker's cancelled result, that result carries the failure the
/// finally node ended on after its eight seconds — work a kill at five would
/// have cut off, leaving a result with no `finally_error` at all.
///
/// **And the execution waits for it.** A cancelled node settles only once every
/// instance it began has ended, and a placed node began this one on the
/// worker: so past the grace the execution is still not settled, and it
/// settles only after the dispatch row has. The inner finally node's failure is
/// then the **execution's** `finally_error` — on the status report and the
/// journal's lifecycle row (PRD resolved q76) — and the root's own finally
/// node, `noted`, which fails too, ran after it: the first failure stands, and
/// the one that stands is the worker's, innermost first.
#[test]
fn a_cancel_on_a_worker_lets_the_finally_node_it_started_run_whole() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(closing());
    let mut worker = mesh.worker("cancel-runs-finally-whole");
    let execution = mesh.start_execution("/closings", &json!({ "path": "release.dmg" }));
    until(
        "the placed agent's model call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["close/0#model/0"]),
    );
    // The attached flow's body is running once the runner has a `sleep` of its
    // own; off Linux, where there is no `/proc` to ask, it is given a moment.
    let runner = worker.runners().map(|_| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let held = worker.runners().unwrap_or_default();
            if let Some(runner) = held.first().copied() {
                let body = u32::try_from(runner)
                    .ok()
                    .and_then(|runner| children_running(runner, "sleep"))
                    .unwrap_or_default();
                if !body.is_empty() {
                    break runner;
                }
            }
            assert!(
                Instant::now() < deadline,
                "the attached flow's body never started on the worker: {}",
                worker.transcript()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    if runner.is_none() {
        std::thread::sleep(Duration::from_secs(2));
    }

    let asked = Instant::now();
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));

    // Past the worker's grace, and inside the finally node's eight seconds: the
    // runner is alive, the dispatch held — and the execution not settled.
    std::thread::sleep(Duration::from_millis(6_500).saturating_sub(asked.elapsed()));
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.iter()
            .map(|row| row["status"].clone())
            .collect::<Vec<_>>(),
        [json!("dispatched")],
        "the dispatch settled inside the finally node's eight seconds, so the runner did not \
         run it whole: {rows:#?}\n{}",
        worker.transcript()
    );
    let early = mesh.report(&execution);
    assert_eq!(
        early["status"], "running",
        "the execution settled while the instance its placed node began on the worker was still \
         running its finally node: {early:#}"
    );
    if let Some(runner) = runner {
        assert!(
            !gone_within(runner, Duration::ZERO),
            "the worker killed the runner at its grace while a finally node was running in it: \
             {}",
            worker.transcript()
        );
    }

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running" && report["status"] != "interrupted"
    });
    assert_eq!(report["status"], "cancelled", "{report:#}");
    assert!(
        asked.elapsed() >= Duration::from_secs(8),
        "the execution settled {:?} after the cancel, before the finally node's eight seconds",
        asked.elapsed()
    );
    // Settled before the execution was, so read once the execution is.
    let rows = mesh.dispatch_rows(&execution);
    assert!(
        rows.len() == 1
            && rows[0]["status"] == "settled"
            && rows[0]["payload"]
                .as_str()
                .is_some_and(|payload| payload.contains("\"ExecutionCancelled\"")),
        "the execution settled before its worker's cancelled result did: {rows:#?}\n{}",
        worker.transcript()
    );
    let payload = rows[0]["payload"].as_str().unwrap_or_default();
    assert!(
        payload.contains("flow.closing node `close_up` failed: ") && payload.contains("exited 3"),
        "the cancelled result does not carry the failure the finally node ended on, so it did \
         not run to its end: {rows:#?}\n{}",
        worker.transcript()
    );
    // The worker's finally failure is the execution's, and it came first: the
    // root's own `noted` failed after it, and the first failure stands.
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the cancelled execution carries no `finally_error`, though its worker's result did: \
             {report:#}"
        )
    });
    assert!(
        finally_error.starts_with("flow.closing node `close_up` failed: ")
            && finally_error.contains("exited 3"),
        "the execution's `finally_error` is not the worker's, so the root's finally node ran \
         before the instance inside it had ended: {finally_error}"
    );
    let noted = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "noted")
        .unwrap_or_else(|| panic!("the root's finally node has an entry: {report:#}"));
    assert_eq!(noted["outcome"], "failed", "{noted:#}");
    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status, finally_error FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "cancelled", "finally_error": finally_error }]),
        "the journal's lifecycle row does not carry the worker's finally failure"
    );
    if let Some(runner) = runner {
        assert!(
            gone_within(runner, Duration::from_secs(10)),
            "the runner outlived the result it posted: {}",
            worker.transcript()
        );
    }
    assert!(worker.running(), "{}", worker.transcript());
}

/// What the model asks for in `flow.lapsed`: the attached flow, whose body
/// outlasts the placed node's four seconds — so nothing past it is scripted.
fn lapsing() -> Vec<Script> {
    vec![Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "lapsing",
            json!({ "path": "release.dmg" }),
        )]),
    )]
}

/// **A placed node's deadline ends what it began on the worker `failed`, never
/// `cancelled` — and waits for it to end** (grammar §7.9, §9.2; PRD resolved
/// q75, q76; Decision D159; `docs/distributed.md` §3.2, §3.3, §3.4).
///
/// `flow.lapsed`'s placed `lapse` has four seconds, and its model calls
/// `flow.lapsing`, whose body runs for ten minutes on the worker and whose
/// finally node fails at once. At four seconds the hub stops waiting: the node
/// fails on its budget and the execution with it — `failed`, because nobody
/// cancelled anything — and the worker is told to **stop** the dispatch, on the
/// poll's `stop` list. The runner stops the placed node with a failure rather
/// than firing its execution's cancel, so the attached instance ends `failed`
/// and its finally node runs at that outcome; the boundary it crosses is
/// journaled from the worker, and says `failed` — what the same instance
/// records when an unplaced node runs out of time
/// (`a_flow_node_whose_timeout_ran_out_waits_for_its_instances_finally_node`).
/// Told to cancel instead, the runner would end it `cancelled` in an execution
/// whose own outcome is `failed`, and a retry's redispatch would hand that
/// record back to be replayed as the cancel nobody made.
///
/// **And the node waits for that end**, as a node whose deadline ran out waits
/// for every instance it began: the hub holds `lapse` open at its site until
/// the worker's result for the stopped dispatch comes home. So the boundary is
/// in the journal by the time the execution settles, and the finally node's
/// failure — carried on that result, which the hub refuses `409` — is the
/// execution's `finally_error`, on the report and on the lifecycle row. A node
/// that settled the moment its budget ran out would settle the execution
/// before the instance on the worker had ended, and drop that failure with the
/// refused result.
#[test]
fn a_deadline_on_a_worker_ends_the_instances_it_began_failed_not_cancelled() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(lapsing());
    let mut worker = mesh.worker("deadline-ends-failed");
    let execution = mesh.start_execution("/lapsings", &json!({ "path": "release.dmg" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running" && report["status"] != "interrupted"
    });
    assert_eq!(
        report["status"],
        "failed",
        "a placed node that ran out of time fails its execution, and cancels nothing: \
         {report:#}\n{}",
        worker.transcript()
    );
    assert!(
        report["error"]
            .as_str()
            .is_some_and(|error| error.contains("timed out")),
        "{report:#}"
    );

    // The instance on the worker ended once the runner had been told to stop,
    // and the node waited for it: its boundary came home over
    // `/workers/effects` **before** the execution settled on the hub.
    const BOUNDARY: &str = "lapse/0/lapsing/0/lapse_up/0#finally/0";
    assert!(
        harness::journal_holds(&mesh.project, &[BOUNDARY]),
        "the execution settled before the instance its placed node began on the worker had \
         ended: {report:#}\n{}",
        worker.transcript()
    );
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the execution carries no `finally_error`, though the finally node the stop started \
             on the worker failed and its result said so: {report:#}\n{}",
            worker.transcript()
        )
    });
    assert!(
        finally_error.starts_with("flow.lapsing node `lapse_up` failed: ")
            && finally_error.contains("exited 5"),
        "the execution's `finally_error` is not the one the worker's result carried: \
         {finally_error}"
    );
    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status, finally_error FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "failed", "finally_error": finally_error }]),
        "the journal's lifecycle row does not carry the worker's finally failure"
    );
    let rows = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, payload FROM effects \
             WHERE execution = '{execution}' AND kind = 'finally'"
        ),
    );
    let rows = rows.as_array().expect("rows");
    assert_eq!(rows.len(), 1, "one instance ended, one boundary: {rows:?}");
    assert_eq!(rows[0]["k"], BOUNDARY, "{rows:?}");
    let recorded: Value = serde_json::from_str(rows[0]["payload"].as_str().expect("a payload"))
        .expect("the payload is JSON");
    assert_eq!(
        recorded["outcome"],
        "failed",
        "the instance a placed node began on the worker ended as its node's deadline ends it, \
         and not as a cancel nobody made: {recorded:#}\n{}",
        worker.transcript()
    );

    // The dispatch was ended by the hub's deadline, never settled as a cancel.
    let dispatches = mesh.dispatch_rows(&execution);
    assert!(
        dispatches.len() == 1
            && dispatches[0]["status"] == "superseded"
            && dispatches[0]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("stopped waiting")),
        "{dispatches:#?}"
    );
    assert!(worker.running(), "{}", worker.transcript());
}

/// **A stop on a worker leaves none of the commands the placed node's attached
/// flow was running** (Decision D160's "a stopped command is stopped whole";
/// `docs/distributed.md` §3.2; PRD resolved q79).
///
/// `flow.lapsed` again: at four seconds the hub stops waiting for `lapse`,
/// while the `sleep 600` of `flow.lapsing`'s body is running in a process
/// group of its own under the runner. The stop is what ends that command too —
/// every node the runner runs chains to it beside the execution's cancel —
/// not only the placed node, whose attached instance ends on it and lets its
/// running node go. Asserted **past the worker's grace**, which is when the
/// command would otherwise show itself: the runner kept alive by it would be
/// killed with its own group, and the command — a group of its own — would
/// survive it, re-parented, still writing into the worker's workspace beside
/// whatever the freed slot was handed next.
#[test]
fn a_stop_on_a_worker_leaves_none_of_the_commands_its_attached_flow_ran() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all(lapsing());
    let mut worker = mesh.worker("stop-takes-the-command");
    let started = Instant::now();
    let execution = mesh.start_execution("/lapsings", &json!({ "path": "release.dmg" }));
    // The runner, and the attached flow's command under it, off `/proc`; off
    // Linux there is no `/proc` to ask, and the case has nothing to read.
    if worker.runners().is_none() {
        return;
    }
    let (runner, command) = {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let held = worker.runners().unwrap_or_default();
            if let Some(runner) = held.first().copied() {
                let body = u32::try_from(runner)
                    .ok()
                    .and_then(|runner| children_running(runner, "sleep"))
                    .unwrap_or_default();
                if let Some(command) = body.first().copied() {
                    break (runner, command);
                }
            }
            assert!(
                Instant::now() < deadline,
                "the attached flow's command never started on the worker: {}",
                worker.transcript()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    };

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running" && report["status"] != "interrupted"
    });
    assert_eq!(
        report["status"],
        "failed",
        "{report:#}\n{}",
        worker.transcript()
    );

    // Past the node's four seconds and the worker's five-second grace after
    // them, with a second to spare.
    std::thread::sleep(Duration::from_secs(10).saturating_sub(started.elapsed()));
    assert!(
        gone_within(command, Duration::ZERO),
        "the attached flow's command outlived the stop that ended its instance, past the \
         worker's grace: {}",
        std::fs::read_to_string(format!("/proc/{command}/stat")).unwrap_or_default()
    );
    assert!(
        gone_within(runner, Duration::ZERO),
        "the runner outlived the grace: {}",
        worker.transcript()
    );
    assert!(worker.running(), "{}", worker.transcript());
}

// ---------------------------------------------------------------------------
// A `coder:` node on a worker
// ---------------------------------------------------------------------------

/// The `placed-coder` fixture: a placed agent whose only tool is an unplaced
/// flow holding a `coder:` node and a `finally:` node; the same build step as a
/// placed flow of its own (Decision D171); and a placed agent attaching that
/// placed flow on its own placement.
const CODER_FIXTURE: &str = "placed-coder";

/// The code the pinned Agent SDK reports a shadowed permission callback under.
const SHADOWED: &str = "CLAUDE_SDK_CAN_USE_TOOL_SHADOWED";

/// The Agent SDK's `query`, stood in on the worker.
///
/// A harness SDK speaks its own wire, which the mock provider never sees (PRD
/// resolved q57), so the run is scripted at the SDK: the two messages the `cc`
/// driver reads a run off — a top-level assistant turn, and a successful result
/// carrying the structured output the node's `output:` asked for. It answers
/// the options it is handed **the way the pinned SDK does** in the one respect
/// a `full_access` run meets on every call: a permission callback set beside
/// `bypassPermissions` is reported with `process.emitWarning` under the SDK's
/// own code, synchronously, while `query()` reads its options.
const CC_STAND_IN: &str = r#"export function query({ prompt, options }) {
  if (options.canUseTool !== undefined && options.permissionMode === "bypassPermissions") {
    process.emitWarning(
      "canUseTool will not be invoked: permissionMode 'bypassPermissions' auto-approves every tool call (except explicit deny rules) before the callback is consulted. To gate every tool call, use a PreToolUse hook instead.",
      { code: "CLAUDE_SDK_CAN_USE_TOOL_SHADOWED" },
    );
  }
  return (async function* answered() {
    yield { type: "assistant", parent_tool_use_id: null, message: { content: [] } };
    yield {
      type: "result",
      subtype: "success",
      parent_tool_use_id: null,
      usage: { input_tokens: 10, output_tokens: 5 },
      total_cost_usd: 0.001,
      stop_reason: "end_turn",
      structured_output: { summary: `built on the worker for ${JSON.parse(prompt).goal}` },
    };
  })();
}
"#;

/// Install [`CC_STAND_IN`] where a worker's runner resolves the SDK from: the
/// worker's own data directory, above every artifact it materialises.
///
/// Module resolution takes the nearest `node_modules` holding the package, and
/// the materialised tree has none of its own — so the runner imports this one,
/// while the dependency set it does not shadow (LangGraph, and the check the
/// worker makes for it) still resolves from the suite's toolchain above. The
/// hub never runs the coder node, so the hub is left with the real SDK.
fn stand_in_the_agent_sdk(data_dir: &Path) {
    let package = data_dir
        .join("node_modules")
        .join("@anthropic-ai")
        .join("claude-agent-sdk");
    std::fs::create_dir_all(&package).expect("the data directory is writable");
    std::fs::write(
        package.join("package.json"),
        serde_json::to_vec_pretty(&json!({
            "name": "@anthropic-ai/claude-agent-sdk",
            "version": "0.0.0-stood-in",
            "type": "module",
            "main": "./sdk.mjs",
            "exports": { ".": "./sdk.mjs" },
        }))
        .expect("the manifest serializes"),
    )
    .expect("the stand-in's manifest is written");
    std::fs::write(package.join("sdk.mjs"), CC_STAND_IN).expect("the stand-in is written");
}

/// The three calls `agent.builder` makes on the worker: the attached flow, a
/// turn after it, and the structured answer.
fn building(summary: &str) -> Vec<Script> {
    building_through("building", summary)
}

/// The three calls an agent attaching a build-step flow as the tool `tool`
/// makes on the worker: the call, a turn after it, and the structured answer.
fn building_through(tool: &str, summary: &str) -> Vec<Script> {
    vec![
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                tool,
                json!({ "goal": "make the release build" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("built, and here is what it did")),
        Script::new(SONNET, Outcome::structured(json!({ "summary": summary }))),
    ]
}

/// **A `coder:` node runs on a worker, and its run comes home** (grammar §8.9,
/// §14.1 rule 4, `docs/distributed.md` §3.3, §3.4, `docs/durability.md` §3.9,
/// §3.10).
///
/// The shape a build step on a build machine took before a flow could be
/// placed itself, and which stays legal beside it: `agent.builder` is placed on
/// `mac`, and its only tool is `flow.building` — **unplaced** — whose `build`
/// node is a `full_access` coder node and whose `finally:` node tidies up. The
/// attached flow runs in the agent's placement (grammar §14.1 rule 4) and in
/// its loop, never dispatched, so all of it runs **on the worker** — and every
/// half of it has to reach the hub:
///
/// * the coder's run, journaled as a `harness` record and streamed home over
///   `/workers/effects`. A hub that took only the four calls and the `finally`
///   boundary refused that batch `400`, the worker settled the dispatch
///   `WorkerEffectsRefused`, and the run was lost — the defect this test would
///   have caught;
/// * the flow instance's end, journaled as its `finally` record from the
///   worker, after the finally node ran there;
/// * the dispatch, settled with the node's value, and the hub's flow completed
///   with it.
///
/// And the run's stderr — the runner's, which the worker inherits — holds the
/// SDK's shadowed-callback report on **one line**, once: `full_access` pairs the
/// callback with `bypassPermissions` deliberately, and a runtime printing the
/// warning with its stack beneath it made every such run look like a failure.
#[test]
fn a_coder_node_in_a_placed_agents_attached_flow_runs_on_the_worker_and_its_run_comes_home() {
    let Some(mesh) = Mesh::start_from("mesh-coder", harness::fixture(CODER_FIXTURE), None) else {
        return;
    };
    mesh.provider
        .enqueue_all(building("the release build ran on the worker"));
    let data_dir = scratch_data_dir("placed-coder");
    stand_in_the_agent_sdk(&data_dir);
    let worker = mesh.worker_into("placed-coder", &data_dir);

    let execution = mesh.start_execution("/builds", &json!({ "goal": "make the release build" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "the build step on the worker did not complete the hub's flow: {report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        report["outputs"]["summary"], "the release build ran on the worker",
        "{report:#}"
    );
    assert!(
        report.get("finally_error").is_none_or(Value::is_null),
        "the finally node ran clean on the worker: {report:#}"
    );

    // The coder's run, journaled from the worker.
    let records = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, kind, payload FROM effects \
             WHERE execution = '{execution}' AND kind IN ('harness', 'finally') ORDER BY \"key\""
        ),
    );
    let records = records.as_array().cloned().unwrap_or_default();
    let harness_run = records
        .iter()
        .find(|row| row["kind"] == "harness")
        .unwrap_or_else(|| {
            panic!(
                "the coder's `harness` record never reached the hub's journal: {records:#?}\n{}",
                worker.transcript()
            )
        });
    assert_eq!(
        harness_run["k"], "build/0/building/0/build/0#harness/0",
        "the run is keyed under the placed node, as an unplaced loop keys it: {records:#?}"
    );
    assert!(
        harness_run["payload"]
            .as_str()
            .unwrap_or_default()
            .contains("built on the worker for make the release build"),
        "the record holds the run's answer as the worker's harness gave it: {harness_run:#}"
    );

    // The flow instance's end, journaled from the worker after its finally node.
    let boundary = records
        .iter()
        .find(|row| row["kind"] == "finally")
        .unwrap_or_else(|| {
            panic!("the attached flow's `finally` record never reached the hub: {records:#?}")
        });
    assert_eq!(
        boundary["k"], "build/0/building/0/tidy_up/0#finally/0",
        "{records:#?}"
    );
    let ended: Value = serde_json::from_str(boundary["payload"].as_str().unwrap_or_default())
        .expect("the payload is JSON");
    assert_eq!(
        ended["outcome"], "completed",
        "the instance that ran the coder ended completed on the worker: {boundary:#}"
    );

    // The dispatch, settled with the node's value — not `WorkerEffectsRefused`.
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(rows.len(), 1, "one placed node, one dispatch: {rows:#?}");
    assert_eq!(
        (rows[0]["status"].as_str(), rows[0]["outcome"].as_str()),
        (Some("settled"), Some("value")),
        "the dispatch did not settle with the node's value: {rows:#?}\n{}",
        worker.transcript()
    );
    assert!(
        rows[0]["payload"]
            .as_str()
            .unwrap_or_default()
            .contains("the release build ran on the worker"),
        "{rows:#?}"
    );

    // One line for the SDK's report, and no stack beneath it.
    let transcript = worker.transcript();
    let reported: Vec<&str> = transcript
        .lines()
        .filter(|line| line.contains(SHADOWED) || line.contains("canUseTool will not be invoked"))
        .collect();
    assert_eq!(
        reported.len(),
        1,
        "the SDK's shadowed-callback report is one line per run: {reported:#?}\n{transcript}"
    );
    assert!(
        reported[0].contains("flow.building") && reported[0].contains("full_access"),
        "the line names the node and the preset that pairs the callback deliberately: {}",
        reported[0]
    );
    let frames: Vec<&str> = transcript
        .lines()
        .filter(|line| line.trim_start().starts_with("at "))
        .collect();
    assert!(
        frames.is_empty(),
        "the run printed a stack trace: {frames:#?}\n{transcript}"
    );
}

// ---------------------------------------------------------------------------
// Placed flows (grammar §14.1, PRD resolved q84, Decision D171)
// ---------------------------------------------------------------------------

/// The dispatch rows one execution parked, with the columns that say where each
/// went and what it carried: the placement, the address the worker looks the
/// work up under, and the instance path it keys its effects by.
fn dispatches_of(mesh: &Mesh, execution: &str) -> Vec<Value> {
    harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT placement, node, site, wait, status, session, outcome FROM dispatches \
             WHERE execution = '{execution}' ORDER BY parked_at, wait"
        ),
    )
    .as_array()
    .cloned()
    .unwrap_or_default()
}

/// The node ids of one trace's entries, in order.
fn nodes_of(trace: &Value) -> Vec<String> {
    trace
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["node"].as_str().map(str::to_string))
        .collect()
}

/// `agent.inspector`'s one call — no tools, so the pinned answer is the whole of
/// it — matched to the path it was asked about, so instances that run side by
/// side are each answered with their own.
fn inspecting(path: &str, signature: &str) -> Script {
    Script::new(
        SONNET,
        Outcome::structured(json!({ "signature": signature })),
    )
    .matching(path)
}

/// **A placed flow's root execution runs whole on a worker, and its outputs
/// come home** (Decision D171, `docs/distributed.md` §2).
///
/// `flow.placed_build` is a member of `mac`, and the trigger starts it as a
/// root execution: the hub opens the lifecycle row and dispatches the instance
/// — one row, at the empty instance path, keyed by the execution and the
/// flow's address — and a worker claiming `mac` runs its `agent:` node and its
/// `finally:` node. What the hub then holds is the worker's: the outputs, the
/// effects the instance journaled over `/workers/effects` under the keys the hub
/// would have derived, and the instance's trace. The hub calls no provider and
/// runs no node of it.
#[test]
fn a_placed_flows_root_execution_runs_whole_on_a_worker_and_its_outputs_come_home() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-on-the-worker"));
    let worker = mesh.worker("placed-flow-root");

    let execution = mesh.start_execution("/placed-builds", &json!({ "path": "release.dmg" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        report["outputs"]["signature"], "inspected-on-the-worker",
        "the instance's outputs are the execution's: {report:#}"
    );
    assert_eq!(
        nodes_of(&report["trace"]),
        ["inspect", "settle"],
        "the trace is the instance's own, its finally node's entry last: {report:#}"
    );

    // One dispatch: the instance, to `mac`, at the root path — and a worker
    // session settled it with the instance's value.
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(rows.len(), 1, "one instance, one dispatch: {rows:#?}");
    assert_eq!(rows[0]["placement"], PLACEMENT, "{rows:#?}");
    assert_eq!(rows[0]["node"], "flow.placed_build", "{rows:#?}");
    assert_eq!(
        rows[0]["site"], "",
        "a root instance sits at the empty path: {rows:#?}"
    );
    assert_eq!(rows[0]["wait"], "flow.placed_build/0", "{rows:#?}");
    assert_eq!(
        (rows[0]["status"].as_str(), rows[0]["outcome"].as_str()),
        (Some("settled"), Some("value")),
        "{rows:#?}"
    );
    assert!(
        rows[0]["session"]
            .as_str()
            .is_some_and(|session| session.starts_with("wrk_")),
        "a worker session held the dispatch: {rows:#?}"
    );

    // The instance's effects, streamed home and keyed as the hub keys a root.
    until(
        "the instance's effects reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["inspect/0#model/0", "settle/0#finally/0"]),
    );
    let snapshot = mesh.provider.snapshot();
    assert!(
        snapshot.is_drained() && mesh.provider.requests().len() == 1,
        "the instance's one model call was made once: {snapshot:#?}"
    );
}

/// **A `flow:` node and a `map` over a placed flow each dispatch the instance**
/// (Decision D171).
///
/// `flow.placed_builds` runs on the hub; its `first` node instantiates the
/// placed flow once and its `every` map once per path. Each instance is a
/// dispatch of its own to `mac`, at the instance path of whatever started it,
/// and each one's outputs reach the hub's channels the way an instance run
/// here would write them — the `flow:` node's into `signature`, the map's items
/// element-wise into `signatures`. The instances' traces come home too: the
/// `flow:` node's entry carries its instance as `inner`, and each map item's
/// dispatch record carries its own.
#[test]
fn a_flow_node_and_a_map_over_a_placed_flow_dispatch_each_instance_to_its_placement() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all([
        inspecting("a.dmg", "first a.dmg"),
        inspecting("a.dmg", "item a.dmg"),
        inspecting("b.dmg", "item b.dmg"),
    ]);
    let worker = mesh.worker("placed-flow-nodes");

    let execution = mesh.start_execution(
        "/placed-build-batches",
        &json!({ "paths": ["a.dmg", "b.dmg"] }),
    );
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(report["outputs"]["signature"], "first a.dmg", "{report:#}");
    assert_eq!(
        report["outputs"]["signatures"],
        json!(["item a.dmg", "item b.dmg"]),
        "{report:#}"
    );

    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.len(),
        3,
        "three instances, three dispatches: {rows:#?}"
    );
    for row in &rows {
        assert_eq!(row["placement"], PLACEMENT, "{rows:#?}");
        assert_eq!(row["node"], "flow.placed_build", "{rows:#?}");
        assert_eq!(row["status"], "settled", "{rows:#?}");
    }
    assert_eq!(
        rows[0]["site"], "first/0",
        "the `flow:` node's instance: {rows:#?}"
    );
    assert!(
        rows[1..].iter().all(|row| row["site"]
            .as_str()
            .is_some_and(|site| site.starts_with("every/0/"))),
        "the map's instances sit under the map node: {rows:#?}"
    );

    let first = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "first")
        .unwrap_or_else(|| panic!("the `flow:` node has an entry: {report:#}"));
    assert_eq!(
        nodes_of(&first["inner"]),
        ["inspect", "settle"],
        "the instance's trace came home as the node's `inner`: {first:#}"
    );
    let every = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "every")
        .unwrap_or_else(|| panic!("the map node has an entry: {report:#}"));
    let records = every["dispatches"].as_array().cloned().unwrap_or_default();
    assert_eq!(records.len(), 2, "{every:#}");
    for record in &records {
        assert_eq!(record["outcome"], "completed", "{record:#}");
        assert_eq!(
            nodes_of(&record["inner"]),
            ["inspect", "settle"],
            "{record:#}"
        );
    }
}

/// **A placed flow attached to an agent the hub runs is dispatched, not run in
/// the loop** (Decision D171).
///
/// `agent.coordinator` has no placement, so its tool loop runs on the hub, and
/// its one tool is `flow.placed_build`. The model's call of it is a flow-as-tool
/// call like any other — the arguments parsed against the flow's `inputs:`, an
/// instance under the call's own frame — except that the instance is
/// dispatched to `mac` and run there. Its outputs are the tool's result the
/// model is handed, and its trace is the call's dispatch record.
#[test]
fn a_placed_flow_attached_to_an_unplaced_agent_runs_each_call_on_a_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "placed_build",
                json!({ "path": "release.dmg" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "inspected-for-the-coordinator" })),
        ),
        Script::new(SONNET, Outcome::text("the build came back inspected")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "coordinated" })),
        ),
    ]);
    let worker = mesh.worker("placed-flow-tool");

    let execution = mesh.start_execution("/coordinations", &json!({ "path": "release.dmg" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(report["outputs"]["signature"], "coordinated", "{report:#}");

    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(rows.len(), 1, "one call, one dispatch: {rows:#?}");
    assert_eq!(rows[0]["placement"], PLACEMENT, "{rows:#?}");
    assert_eq!(rows[0]["node"], "flow.placed_build", "{rows:#?}");
    assert_eq!(
        rows[0]["site"], "coordinate/0/placed_build/0",
        "the instance sits under the call's own frame: {rows:#?}"
    );

    // The model was handed what the worker's instance answered.
    let handed = mesh
        .provider
        .requests()
        .iter()
        .any(|request| request.body_text.contains("inspected-for-the-coordinator"));
    assert!(
        handed,
        "the tool result the model saw was not the instance's outputs"
    );

    let coordinate = report["trace"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|entry| entry["node"] == "coordinate")
        .unwrap_or_else(|| panic!("the agent node has an entry: {report:#}"));
    let calls = coordinate["toolDispatches"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    assert_eq!(calls.len(), 1, "{coordinate:#}");
    assert_eq!(calls[0]["target"], "flow.placed_build", "{coordinate:#}");
    assert_eq!(
        nodes_of(&calls[0]["inner"]),
        ["inspect", "settle"],
        "the call's record carries the instance's trace: {coordinate:#}"
    );
}

/// **A placed flow's finally node runs on the worker, and its failure reaches
/// the hub** (grammar §7.9, Decision D171, `docs/distributed.md` §3.4).
///
/// `flow.placed_tidying` is started as a root execution; its body succeeds and
/// its finally node fails, both on the worker. The run completes — a finally
/// node changes no outcome — and what the finally node raised comes home on the
/// dispatch's result as `finally_error`, onto the lifecycle row the hub alone
/// writes. The instance's boundary is journaled from the worker at the key a
/// root's finally node takes.
#[test]
fn a_placed_flows_finally_node_runs_on_the_worker_and_its_failure_is_the_executions() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let worker = mesh.worker("placed-flow-finally");

    let execution = mesh.start_execution("/placed-tidyings", &json!({ "path": "release.dmg" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "a finally node changes no outcome, on a worker as anywhere: {report:#}\n{}",
        worker.transcript()
    );
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the finally node's failure on the worker is the execution's: {report:#}\n{}",
            worker.transcript()
        )
    });
    assert!(
        finally_error.starts_with("flow.placed_tidying node `tidy_up` failed: "),
        "{finally_error}"
    );
    assert_eq!(
        nodes_of(&report["trace"]),
        ["look", "tidy_up"],
        "{report:#}"
    );

    let boundary = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, payload FROM effects \
             WHERE execution = '{execution}' AND kind = 'finally'"
        ),
    );
    let boundary = boundary.as_array().cloned().unwrap_or_default();
    assert_eq!(
        boundary.len(),
        1,
        "one instance ended, one boundary: {boundary:?}"
    );
    assert_eq!(boundary[0]["k"], "tidy_up/0#finally/0", "{boundary:?}");
    let recorded: Value = serde_json::from_str(boundary[0]["payload"].as_str().unwrap_or_default())
        .expect("the payload is JSON");
    assert_eq!(recorded["outcome"], "completed", "{recorded:#}");

    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status, finally_error FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "completed", "finally_error": finally_error }]),
        "the hub wrote the worker's failure into the row it alone writes"
    );
}

/// **A cancel of a running placed-flow execution reaches its worker, and the
/// execution settles `cancelled` on the worker's result** (PRD resolved q75,
/// Decision D160, D171, `docs/distributed.md` §3.2, §3.4).
///
/// `flow.placed_closing`'s body runs for ten minutes on the worker. The cancel
/// is the execution's, and it is the root dispatch's signal: the hub names the
/// dispatch in the worker's next poll's `cancel` list, the worker stops the
/// runner, the instance ends `cancelled` there and runs its finally node — which
/// fails — and the worker's `status: "cancelled"` result, carrying that failure,
/// is what settles the dispatch and so the execution.
#[test]
fn a_cancel_ends_a_placed_flows_execution_cancelled_with_its_workers_result() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let mut worker = mesh.worker("placed-flow-cancel");

    let execution = mesh.start_execution("/placed-closings", &json!({ "path": "release.dmg" }));
    mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    // Inside the instance's body: the runner the worker spawned for the dispatch
    // is running the body's ten-minute command, so the cancel reaches an
    // instance rather than a runner still loading.
    until(
        "the instance's body was running on the worker",
        || worker.transcript(),
        || {
            worker.runners().is_none_or(|running| {
                running.iter().any(|runner| {
                    u32::try_from(*runner)
                        .ok()
                        .and_then(|runner| children_running(runner, "sleep"))
                        .is_some_and(|commands| !commands.is_empty())
                })
            })
        },
    );

    let asked = Instant::now();
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));
    let report = mesh.until(&execution, "was cancelled", |report| {
        report["status"] == "cancelled" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "cancelled",
        "{report:#}\n{}",
        worker.transcript()
    );
    // Settled by the worker's cancelled result, which the execution waited for.
    let rows = mesh.settled_cancelled(&execution, 1, Duration::ZERO);
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to settle a placed flow",
        asked.elapsed()
    );
    assert!(report.get("error").is_none(), "{report:#}");
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the finally node the cancel started on the worker failed, and said so: {report:#}\n\
             {rows:#?}\n{}",
            worker.transcript()
        )
    });
    // The instance's trace came home on the cancelled result: the body node the
    // cancel stopped, and the finally node it started.
    assert_eq!(
        nodes_of(&report["trace"]),
        ["look_long", "close_up"],
        "{report:#}"
    );
    assert_eq!(report["trace"][0]["outcome"], "cancelled", "{report:#}");
    assert!(
        finally_error.starts_with("flow.placed_closing node `close_up` failed: "),
        "{finally_error}"
    );
    let boundary = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT payload FROM effects \
             WHERE execution = '{execution}' AND \"key\" = 'close_up/0#finally/0'"
        ),
    );
    let recorded: Value = serde_json::from_str(boundary[0]["payload"].as_str().unwrap_or_default())
        .expect("the boundary was journaled from the worker");
    assert_eq!(
        recorded["outcome"], "cancelled",
        "the instance ended as the cancel ended it: {recorded:#}"
    );
    assert!(
        worker.running(),
        "the worker stopped with the instance it was asked to cancel: {}",
        worker.transcript()
    );
}

/// **A deadline on a `flow:` node over a placed flow stops the instance on its
/// worker, ends it `failed`, and waits for its finally node there** (grammar
/// §7.9, §9.2; PRD resolved q76, q79, q84; Decisions D159, D171;
/// `docs/distributed.md` §2, §3.2, §6.5).
///
/// `flow.placed_lapsed`'s `bounded` node instantiates `flow.placed_closing`
/// under four seconds, and the instance is a dispatch to `mac`: its body's
/// ten-minute command runs on the worker, and its finally node fails there at
/// once. At four seconds the node's deadline is the instance's stop — the
/// clock `runtime.runSubflow` hands the instance follows the instantiating
/// node's — so the hub stops waiting for the dispatch, supersedes it, and names
/// it on the worker's next `stop` list. The runner is sent `SIGUSR2`, and what a
/// placed flow's instance runs under is that stop (`PlacedSite.stop`): every
/// node of it ends, its command with it, and the instance ends **`failed`** —
/// the outcome nobody's cancel would give it — and runs its finally node on the
/// worker at that outcome.
///
/// **And the node waits for that end**, as a node whose deadline ran out waits
/// for every instance it began (q76's "a timed-out instantiating node waits for
/// its instance's finally"): the hub holds `bounded` open until the worker's
/// result for the stopped dispatch is home. That result is refused `409` — the
/// dispatch is no longer the one the hub awaits — but the finally node's failure
/// it carries is the execution's `finally_error`, on the report and on the
/// lifecycle row, and the boundary is in the journal before the execution
/// settles. A hub that settled the node the moment its budget ran out would
/// close the execution with neither; a runner whose stop reached neither the
/// instance nor the nodes it runs (`runtime.stopEveryNodeOn`) would leave the
/// command running past the deadline, and the instance would end only when the
/// worker's grace killed the runner — with no finally node and no result.
#[test]
fn a_deadline_over_a_placed_flow_stops_its_instance_on_the_worker_and_waits_for_its_finally() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let mut worker = mesh.worker("placed-flow-deadline");
    let started = Instant::now();
    let execution = mesh.start_execution("/placed-lapsings", &json!({ "path": "release.dmg" }));

    // The instance's body, running on the worker under the runner the dispatch
    // spawned — off `/proc`, where there is one to ask.
    let body = worker.runners().map(|_| {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let held = worker.runners().unwrap_or_default();
            let running = held.iter().find_map(|runner| {
                u32::try_from(*runner)
                    .ok()
                    .and_then(|runner| children_running(runner, "sleep"))
                    .and_then(|commands| commands.first().copied())
                    .map(|command| (*runner, command))
            });
            if let Some(running) = running {
                break running;
            }
            assert!(
                Instant::now() < deadline,
                "the placed instance's body never started on the worker: {}",
                worker.transcript()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    });

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running" && report["status"] != "interrupted"
    });
    let settled_after = started.elapsed();
    assert_eq!(
        report["status"],
        "failed",
        "a `flow:` node that ran out of time over a placed flow fails its execution, and \
         cancels nothing: {report:#}\n{}",
        worker.transcript()
    );
    assert!(
        report["error"]
            .as_str()
            .is_some_and(|error| error.contains("timed out")),
        "{report:#}"
    );
    // Settled by the stop's own path, not by the worker's grace running out on
    // a runner that ignored it: four seconds of budget, then the runner's
    // finally node, well inside the five-second grace that would follow.
    assert!(
        settled_after < Duration::from_secs(4 + 5),
        "the execution took {settled_after:?} to settle — as long as the worker's grace, so the \
         instance did not stop on the stop: {}",
        worker.transcript()
    );

    // The instance ended on the worker, and the node waited for it: its
    // boundary came home over `/workers/effects` **before** the execution
    // settled, at the key the `flow:` node's instance takes.
    const BOUNDARY: &str = "bounded/0/close_up/0#finally/0";
    assert!(
        harness::journal_holds(&mesh.project, &[BOUNDARY]),
        "the execution settled before the placed instance its `flow:` node began on the worker \
         had ended: {report:#}\n{}",
        worker.transcript()
    );
    let finally_error = report["finally_error"].as_str().unwrap_or_else(|| {
        panic!(
            "the execution carries no `finally_error`, though the finally node the stop started \
             on the worker failed and its result said so: {report:#}\n{}",
            worker.transcript()
        )
    });
    assert!(
        finally_error.starts_with("flow.placed_closing node `close_up` failed: ")
            && finally_error.contains("exited 3"),
        "the execution's `finally_error` is not the one the worker's result carried: \
         {finally_error}"
    );
    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status, finally_error FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "failed", "finally_error": finally_error }]),
        "the journal's lifecycle row does not carry the worker's finally failure"
    );
    let rows = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, payload FROM effects \
             WHERE execution = '{execution}' AND kind = 'finally'"
        ),
    );
    let rows = rows.as_array().cloned().unwrap_or_default();
    assert_eq!(rows.len(), 1, "one instance ended, one boundary: {rows:?}");
    assert_eq!(rows[0]["k"], BOUNDARY, "{rows:?}");
    let recorded: Value = serde_json::from_str(rows[0]["payload"].as_str().unwrap_or_default())
        .expect("the payload is JSON");
    assert_eq!(
        recorded["outcome"],
        "failed",
        "the placed instance ended as its instantiating node's deadline ends it, and not as a \
         cancel nobody made: {recorded:#}\n{}",
        worker.transcript()
    );

    // One dispatch — the instance, to `mac`, at the `flow:` node's own instance
    // path — ended by the hub's deadline, never settled as a cancel.
    let placed = dispatches_of(&mesh, &execution);
    assert_eq!(placed.len(), 1, "one instance, one dispatch: {placed:#?}");
    assert_eq!(placed[0]["placement"], PLACEMENT, "{placed:#?}");
    assert_eq!(placed[0]["node"], "flow.placed_closing", "{placed:#?}");
    assert_eq!(placed[0]["site"], "bounded/0", "{placed:#?}");
    let dispatches = mesh.dispatch_rows(&execution);
    assert!(
        dispatches.len() == 1
            && dispatches[0]["status"] == "superseded"
            && dispatches[0]["detail"]
                .as_str()
                .is_some_and(|detail| detail.contains("stopped waiting")),
        "{dispatches:#?}"
    );

    // The stop reached the instance's command, and the runner went with the
    // result it posted rather than with the worker's grace.
    if let Some((runner, command)) = body {
        assert!(
            gone_within(command, Duration::from_secs(2)),
            "the placed instance's command outlived the stop that ended its instance: {}",
            std::fs::read_to_string(format!("/proc/{command}/stat")).unwrap_or_default()
        );
        assert!(
            gone_within(runner, Duration::from_secs(10)),
            "the runner outlived the result it posted: {}",
            worker.transcript()
        );
    }
    assert!(worker.running(), "{}", worker.transcript());
}

/// The address a `run` says it serves its mesh at, once it has said so.
///
/// The run writes one `mesh: <url> — …` line on stderr when the first dispatch
/// of a placed flow's instance it makes starts its listener (Decision D171); everything
/// after the url on that line is the `agent-compose worker` command a reader
/// starts next. Waits for the line rather than for the process, which is still
/// running — it is waiting for the worker this answer is used to start.
fn mesh_of(running: &harness::Running) -> String {
    let deadline = Instant::now() + PATIENCE;
    loop {
        let said = running.said();
        if let Some(url) = said.lines().find_map(|line| {
            line.strip_prefix("mesh: ")
                .and_then(|rest| rest.split_whitespace().next())
                .map(str::to_string)
        }) {
            return url;
        }
        assert!(
            Instant::now() < deadline,
            "the run never said where a worker joins it\nstderr: {said}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// One `run`'s dispatch rows, with the columns [`dispatches_of`] reads.
fn run_dispatches(project: &Path, flow: &str) -> Vec<Value> {
    harness::journal_rows(
        project,
        &format!(
            "SELECT d.placement, d.node, d.site, d.wait, d.status, d.session, d.outcome \
             FROM dispatches d JOIN executions e ON e.id = d.execution \
             WHERE e.flow = '{flow}' ORDER BY d.parked_at, d.wait"
        ),
    )
    .as_array()
    .cloned()
    .unwrap_or_default()
}

/// **`agent-compose run` of a placed flow serves the mesh itself, and a worker
/// that joins it runs the instance** (PRD resolved q84 — "a root execution a
/// trigger or `run` starts … runs on a worker claiming the placement"; grammar
/// §14.1 rule 2; Decision D171; `docs/distributed.md` §2).
///
/// A `run` is one process, and the placed instance it starts is still a
/// dispatch a hub makes: onto a board, through the routes a worker joins. So the
/// run is that hub for as long as it lasts. Its first dispatch to `mac` starts a
/// listener for the worker routes and says where on stderr, the instance
/// **parks** there — at the empty instance path, keyed by the execution and the
/// flow's address, held by no session — and the run waits. A worker started
/// against the address it named joins, is handed the dispatch, runs the
/// instance's `agent:` node and its `finally:` node, streams their effects home
/// to the run's journal and posts the instance's outputs, which are the run's
/// answer on stdout. The CLI's own process called no provider: the one model
/// call the provider answered was the worker's.
///
/// Both ways a `run` reaches a placed flow are driven, each with a worker of its
/// own:
///
///  * a `run` of `flow.placed_build` itself — the root dispatch;
///  * a `run` of `flow.placed_retried`, which the run executes, whose `build`
///    node instantiates the placed flow — at that node's instance path — with
///    `--host` and `--port` given, which the mesh line names back.
#[test]
fn a_run_of_a_placed_flow_serves_the_mesh_and_the_worker_that_joins_it_runs_the_instance() {
    let Some(project) = harness::scratch_project("placed-flow-run") else {
        return;
    };
    let _scratch = harness::Scratch::at(project.clone());
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        inspecting("release.dmg", "inspected-on-the-runs-worker"),
        inspecting("hotfix.dmg", "inspected-from-the-runs-node"),
    ]);
    let entrypoint = harness::fixture(FIXTURE);
    let environment = environment(&provider);

    // The placed flow as the run's own root.
    let running = harness::run_target_spawned_with(
        &project,
        &entrypoint,
        TARGET,
        "flow.placed_build",
        &[("path", "release.dmg")],
        &[],
        &environment,
    );
    let hub = mesh_of(&running);
    assert!(
        running.said().contains(&format!(
            "agent-compose worker --hub {hub} --claim {PLACEMENT} --token-env {TOKEN_VARIABLE}"
        )),
        "the run names the command a worker joins it with: {}",
        running.said()
    );
    // Parked on the run's board while nothing claims `mac` (§6.4), and nothing
    // of the instance has run anywhere.
    until(
        "the run parked the instance on its own board",
        || running.said(),
        || {
            run_dispatches(&project, "flow.placed_build")
                .first()
                .is_some_and(|row| row["status"] == "parked")
        },
    );
    assert!(
        provider.requests().is_empty(),
        "the run's own process ran the placed instance's `agent:` node: {:#?}",
        provider.snapshot()
    );
    let worker = Worker::start(&hub, &environment, "placed-flow-run-root", None);
    let run = running.finish();
    run.succeeded();
    assert_eq!(
        run.outputs()["signature"],
        "inspected-on-the-runs-worker",
        "the instance's outputs are the run's answer: {}\n{}",
        run.stderr(),
        worker.transcript()
    );
    assert_eq!(
        run.trace()
            .iter()
            .filter_map(|entry| entry["node"].as_str().map(str::to_string))
            .collect::<Vec<_>>(),
        ["inspect", "settle"],
        "the run's trace is the instance's own, its finally node's entry last"
    );
    let rows = run_dispatches(&project, "flow.placed_build");
    assert_eq!(rows.len(), 1, "one instance, one dispatch: {rows:#?}");
    assert_eq!(rows[0]["placement"], PLACEMENT, "{rows:#?}");
    assert_eq!(rows[0]["node"], "flow.placed_build", "{rows:#?}");
    assert_eq!(
        rows[0]["site"], "",
        "a root instance sits at the empty path: {rows:#?}"
    );
    assert_eq!(rows[0]["wait"], "flow.placed_build/0", "{rows:#?}");
    assert_eq!(
        (rows[0]["status"].as_str(), rows[0]["outcome"].as_str()),
        (Some("settled"), Some("value")),
        "{rows:#?}"
    );
    assert!(
        rows[0]["session"]
            .as_str()
            .is_some_and(|session| session.starts_with("wrk_")),
        "a worker session held the dispatch: {rows:#?}"
    );
    assert!(
        harness::journal_holds(&project, &["inspect/0#model/0", "settle/0#finally/0"]),
        "the instance's effects were streamed home to the run's journal: {}",
        harness::journal_rows(&project, "SELECT \"key\" AS k FROM effects")
    );
    assert_eq!(
        provider.requests().len(),
        1,
        "the instance's one model call was made once, by the worker"
    );
    drop(worker);

    // A `flow:` node over it, in a flow the run executes itself, at the
    // address the command names.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a loopback port")
        .port();
    let running = harness::run_target_spawned_with(
        &project,
        &entrypoint,
        TARGET,
        "flow.placed_retried",
        &[("path", "hotfix.dmg")],
        &["--host", "127.0.0.1", "--port", &port.to_string()],
        &environment,
    );
    let hub = mesh_of(&running);
    assert_eq!(
        hub,
        format!("http://127.0.0.1:{port}"),
        "the run serves its mesh where `--host` and `--port` say: {}",
        running.said()
    );
    let worker = Worker::start(&hub, &environment, "placed-flow-run-node", None);
    let run = running.finish();
    run.succeeded();
    assert_eq!(
        run.outputs()["signature"],
        "inspected-from-the-runs-node",
        "the `flow:` node's instance ran on the worker and its outputs reached the run: {}\n{}",
        run.stderr(),
        worker.transcript()
    );
    let rows = run_dispatches(&project, "flow.placed_retried");
    assert_eq!(rows.len(), 1, "one instance, one dispatch: {rows:#?}");
    assert_eq!(rows[0]["placement"], PLACEMENT, "{rows:#?}");
    assert_eq!(rows[0]["node"], "flow.placed_build", "{rows:#?}");
    assert_eq!(
        rows[0]["site"], "build/0",
        "a `flow:` node's instance is dispatched at the node's own instance path: {rows:#?}"
    );
    assert_eq!(
        (rows[0]["status"].as_str(), rows[0]["outcome"].as_str()),
        (Some("settled"), Some("value")),
        "{rows:#?}"
    );
    assert_eq!(
        provider.requests().len(),
        2,
        "each instance's one model call was made once, by a worker"
    );
}

/// **A placed flow's dispatch parks while nothing claims its placement, and a
/// worker's join wakes it** (`docs/distributed.md` §6, Decision D171).
///
/// The root dispatch is a placement wait like a placed node's: on the board,
/// under the identity the execution and the flow's address give it, `parked`
/// while no session holds it — and the execution `running`, since undispatched
/// is a pause rather than a failure (§6.4). A worker joining is the whole of
/// the wake.
#[test]
fn a_placed_flows_dispatch_parks_with_no_worker_and_a_join_runs_it() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-after-parking"));

    let execution = mesh.start_execution("/placed-builds", &json!({ "path": "release.dmg" }));
    let parked = mesh.until(&execution, "parked on its placement", |report| {
        !report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty)
    });
    let wait = &parked["placement_waits"][0];
    assert_eq!(wait["placement"], PLACEMENT, "{parked:#}");
    assert_eq!(wait["node"], "flow.placed_build", "{parked:#}");
    assert_eq!(wait["wait_id"], "flow.placed_build/0", "{parked:#}");
    assert_eq!(wait["status"], "parked", "{parked:#}");
    assert_eq!(parked["status"], "running", "{parked:#}");

    let _worker = mesh.worker("placed-flow-wake");
    let outputs = mesh.completed(&execution);
    assert_eq!(outputs["signature"], "inspected-after-parking");
}

/// **A hub restarted over a placed flow's dispatch supersedes it, and the
/// instantiating node's retry dispatches the instance again** (§5, §6.3,
/// Decision D171).
///
/// `flow.placed_retried`'s `build` node instantiates `flow.placed_build` under
/// a `retry:`. The worker is inside the instance — its model call held three
/// seconds — when the process behind the hub's name is replaced: the dispatch
/// the dead process issued is superseded at start, the node's retry opens a
/// second attempt at the next ordinal of the same instance path, and the worker
/// that survived the restart re-joins and runs it.
#[test]
fn a_hub_restarted_over_a_placed_flows_dispatch_recovers_it_through_the_retry() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    const SIGNED: &str = "inspected-across-a-restart";
    mesh.provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": SIGNED })).after(Duration::from_secs(3)),
        ),
        Script::new(SONNET, Outcome::structured(json!({ "signature": SIGNED }))),
    ]);
    let mut worker = mesh.worker("placed-flow-restart");

    let execution =
        mesh.start_execution("/placed-retried-builds", &json!({ "path": "release.dmg" }));
    let holding = mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    assert_eq!(
        holding["placement_waits"][0]["wait_id"], "build/0/0",
        "{holding:#}"
    );
    let superseded = holding["placement_waits"][0]["dispatch_id"]
        .as_str()
        .expect("a dispatched wait names the dispatch holding it")
        .to_string();
    until(
        "the instance's model call reached the provider",
        || worker.transcript(),
        || !mesh.provider.requests().is_empty(),
    );

    mesh.restart();

    let retried = mesh.until(&execution, "opened a second attempt", |report| {
        report["placement_waits"][0]["dispatch_id"]
            .as_str()
            .is_some_and(|held| held != superseded)
    });
    assert_eq!(
        retried["placement_waits"][0]["wait_id"], "build/0/1",
        "the attempt after a superseded one is a wait of its own (§6.1): {retried:#}"
    );
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signature"],
        SIGNED,
        "the retry did not reach the worker that survived the restart; it said:\n{}",
        worker.transcript()
    );
    assert!(
        worker.running(),
        "the worker stopped somewhere across the restart: {}",
        worker.transcript()
    );
}

/// **A hub restarted over a placed flow's root dispatch fails the execution
/// once, and nothing runs the instance again** (§2, §5, §6.3, Decision D171).
///
/// `flow.placed_build` started as a root execution: the hub opens the lifecycle
/// row and dispatches the instance at the empty path, keyed by the execution and
/// the flow's address. The worker is inside it — its model call held three
/// seconds — when the process behind the hub's name is replaced. The replacing
/// process supersedes the dispatch the dead one issued, and recovery replays the
/// execution to that dispatch and finds it superseded. A root instance has no
/// instantiating node and so no `retry:` ladder: the execution fails on the
/// lost session, its lifecycle row closes `failed`, and **no second dispatch is
/// issued** — the long step is not run twice. The worker that survived the
/// restart is still running, and whatever its old runner was doing is never
/// asked of the provider again.
#[test]
fn a_hub_restarted_over_a_placed_flows_root_dispatch_fails_the_execution_without_a_second_run() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue(
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "inspected-before-the-restart" }))
                .after(Duration::from_secs(3)),
        )
        .matching("release.dmg"),
    );
    let mut worker = mesh.worker("placed-flow-root-restart");

    let execution = mesh.start_execution("/placed-builds", &json!({ "path": "release.dmg" }));
    let holding = mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    assert_eq!(
        holding["placement_waits"][0]["wait_id"], "flow.placed_build/0",
        "the root instance's wait is keyed by the flow's address: {holding:#}"
    );
    let superseded = holding["placement_waits"][0]["dispatch_id"]
        .as_str()
        .expect("a dispatched wait names the dispatch holding it")
        .to_string();
    until(
        "the instance's model call reached the provider",
        || worker.transcript(),
        || !mesh.provider.requests().is_empty(),
    );

    mesh.restart();

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "failed",
        "a root instance whose session was lost has no retry to recover through, so its \
         execution fails (§2): {report:#}\n{}",
        worker.transcript()
    );
    let error = report["error"].as_str().unwrap_or_default();
    assert!(
        error.contains("DispatchSuperseded") && error.contains("was replaced"),
        "the execution's failure is not the superseded dispatch's: {report:#}"
    );
    let lifecycle = harness::journal_rows(
        &mesh.project,
        &format!("SELECT status FROM executions WHERE id = '{execution}'"),
    );
    assert_eq!(
        lifecycle,
        json!([{ "status": "failed" }]),
        "the lifecycle row the hub opened for the root did not close failed"
    );

    // The worker's old runner ends — stopped on its re-join, or answering a
    // session that is over — and nothing it did after the restart is anyone's.
    until(
        "the runner the restart orphaned is gone",
        || worker.transcript(),
        || worker.runners().is_none_or(|running| running.is_empty()),
    );
    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.iter()
            .map(|row| (row["id"].as_str(), row["status"].as_str()))
            .collect::<Vec<_>>(),
        [(Some(superseded.as_str()), Some("superseded"))],
        "the one root dispatch is the one superseded, and none was issued after it: {rows:#?}\n{}",
        worker.transcript()
    );
    assert_eq!(
        mesh.report(&execution)["status"],
        "failed",
        "the worker's late answer reopened an execution that had failed"
    );
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        1,
        "the instance's model call was made again after the restart: {requests:#?}"
    );
    assert!(
        worker.running(),
        "the worker stopped somewhere across the restart: {}",
        worker.transcript()
    );
}

/// **A placed flow's dispatches count against its placement's `max_running:`**
/// (`docs/distributed.md` §2, Decision D161, D171).
///
/// A one-worker pool whose placement says `max_running: 2`, and a map over the
/// placed flow admitting four: each item's instance is one dispatch, held for a
/// second and a half on the worker, so the session holds two of them at once and
/// never three, and the fan-out completes.
#[test]
fn a_placed_flows_dispatches_count_against_its_placements_max_running() {
    let Some(mesh) = capped("placed-flow-capacity", 2) else {
        return;
    };
    let held = Duration::from_millis(1500);
    mesh.provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "first a.dmg" })),
        )
        .matching("a.dmg"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "item a.dmg" })).after(held),
        )
        .matching("a.dmg"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "item b.dmg" })).after(held),
        )
        .matching("b.dmg"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "item c.dmg" })).after(held),
        )
        .matching("c.dmg"),
    ]);
    let _worker = mesh.worker("placed-flow-capacity");
    let execution = mesh.start_execution(
        "/placed-build-batches",
        &json!({ "paths": ["a.dmg", "b.dmg", "c.dmg"] }),
    );
    let most = most_dispatched_at_once(&mesh, &execution, 2);
    assert_eq!(
        most, 2,
        "one session at a cap of two never held two instances"
    );
    let outputs = mesh.completed(&execution);
    assert_eq!(
        outputs["signatures"].as_array().map(Vec::len),
        Some(3),
        "{outputs:#}"
    );
}

/// **A `coder:` node in a placed flow runs on the worker, with no agent in the
/// way** (PRD resolved q84, Decision D171).
///
/// The report's use case. `flow.placed_building` is a member of `mac` itself,
/// so the trigger that starts it starts a root execution the hub dispatches
/// whole: the coder node's run happens on the worker and its `harness` record
/// comes home, the finally node runs there and its boundary comes home, and the
/// summary the coder produced is the execution's output. No model is called for
/// it at all — the placed agent the coder node used to need, and its extra
/// model call, are gone.
#[test]
fn a_coder_node_in_a_placed_flow_runs_on_the_worker_with_no_agent_in_the_way() {
    let Some(mesh) = Mesh::start_from("mesh-coder-flow", harness::fixture(CODER_FIXTURE), None)
    else {
        return;
    };
    let data_dir = scratch_data_dir("placed-coder-flow");
    stand_in_the_agent_sdk(&data_dir);
    let worker = mesh.worker_into("placed-coder-flow", &data_dir);

    let execution =
        mesh.start_execution("/build-steps", &json!({ "goal": "make the release build" }));
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        report["outputs"]["summary"], "built on the worker for make the release build",
        "the coder's summary is the execution's output: {report:#}"
    );

    let records = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, kind FROM effects \
             WHERE execution = '{execution}' AND kind IN ('harness', 'finally') ORDER BY \"key\""
        ),
    );
    assert_eq!(
        records,
        json!([
            { "k": "build/0#harness/0", "kind": "harness" },
            { "k": "tidy_up/0#finally/0", "kind": "finally" },
        ]),
        "the run and the instance's end came home, keyed at the root: {}",
        worker.transcript()
    );

    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(rows[0]["node"], "flow.placed_building", "{rows:#?}");
    assert_eq!(rows[0]["placement"], PLACEMENT, "{rows:#?}");
    assert!(
        mesh.provider.requests().is_empty(),
        "a placed build step called a model: {:#?}",
        mesh.provider.requests()
    );
}

/// **A placed flow attached by an agent of its own placement runs on the worker
/// already running the agent** (grammar §14.1 rule 4, Decision D171).
///
/// `agent.placed_builder` is a member of `mac`, and its one tool is
/// `flow.placed_building`, a member of `mac` too — the same answer written
/// twice. The call's instance is a placed flow's, so the runtime dispatches it
/// rather than running it in the loop; but the dispatch is issued on the worker
/// that took the agent's node, the one process a worker can hand it to, so that
/// worker runs the instance itself. Nothing about it reaches the hub's board:
/// one dispatch — the agent node's — settles with the node's value, while the
/// coder node's `harness` record and the instance's `finally` boundary come
/// home keyed under the call inside that node, exactly as an unplaced attached
/// flow's do. The model is handed the coder's summary as the tool's result.
#[test]
fn a_placed_flow_attached_by_an_agent_of_its_own_placement_runs_on_the_agents_worker() {
    let Some(mesh) = Mesh::start_from(
        "mesh-coder-same-placement",
        harness::fixture(CODER_FIXTURE),
        None,
    ) else {
        return;
    };
    mesh.provider.enqueue_all(building_through(
        "placed_building",
        "the placed build ran in the agent's loop",
    ));
    let data_dir = scratch_data_dir("placed-coder-same-placement");
    stand_in_the_agent_sdk(&data_dir);
    let worker = mesh.worker_into("placed-coder-same-placement", &data_dir);

    let execution = mesh.start_execution(
        "/placed-builds",
        &json!({ "goal": "make the release build" }),
    );
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "completed" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "completed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        report["outputs"]["summary"], "the placed build ran in the agent's loop",
        "{report:#}"
    );
    assert!(
        report.get("finally_error").is_none_or(Value::is_null),
        "the finally node ran clean on the worker: {report:#}"
    );

    // The coder's run and the instance's end, journaled from the worker under
    // the call inside the placed agent node.
    let records = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k, kind FROM effects \
             WHERE execution = '{execution}' AND kind IN ('harness', 'finally') ORDER BY \"key\""
        ),
    );
    assert_eq!(
        records,
        json!([
            { "k": "build/0/placed_building/0/build/0#harness/0", "kind": "harness" },
            { "k": "build/0/placed_building/0/tidy_up/0#finally/0", "kind": "finally" },
        ]),
        "the run and the instance's end came home under the call: {}",
        worker.transcript()
    );

    // One dispatch, the agent node's: the instance its loop started was run by
    // the worker holding it, not handed back to the hub's board.
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (
                row["placement"].as_str(),
                row["node"].as_str(),
                row["status"].as_str(),
                row["outcome"].as_str()
            ))
            .collect::<Vec<_>>(),
        [(
            Some(PLACEMENT),
            Some("flow.placed_built.build"),
            Some("settled"),
            Some("value")
        )],
        "the placed agent node is the one dispatch, settled with its value: {rows:#?}\n{}",
        worker.transcript()
    );

    // The model was handed what the instance answered, and asked three times.
    let requests = mesh.provider.requests();
    assert_eq!(requests.len(), 3, "{requests:#?}");
    assert!(
        requests.iter().any(|request| request
            .body_text
            .contains("built on the worker for make the release build")),
        "the tool result the model saw was not the instance's outputs: {requests:#?}"
    );
}

/// **A `human:` node inside a placed flow pauses on the hub's board, and the
/// answer dispatches the instance again** (Decision D171, `docs/distributed.md`
/// §3.4, PRD resolved q46).
///
/// `flow.placed_asking` is a member of `mac` and is started as a root
/// execution, so its `ask` node is reached on the worker — the same shape as
/// an attached flow's question inside a placed agent's loop, one level out. The
/// wait is the hub's, so the root dispatch settles *paused* rather than holding
/// the worker while a person thinks; the ordinary resume surface takes the
/// answer; and the instance re-enters dispatch at the root path and replays its
/// `inspect` node out of the effect history rather than paying for the model
/// call again.
#[test]
fn a_human_node_in_a_placed_flow_pauses_on_the_hub_and_its_answer_redispatches_the_instance() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-before-asking"));
    let worker = mesh.worker("placed-flow-asking");

    let execution = mesh.start_execution("/placed-askings", &json!({ "path": "release.dmg" }));
    let wait = mesh.paused(&execution);
    assert_eq!(
        (
            wait["flow"].as_str(),
            wait["node"].as_str(),
            wait["wait_id"].as_str()
        ),
        (Some("flow.placed_asking"), Some("ask"), Some("ask/0")),
        "the wait is not the placed flow's own `human:` node at the root instance's path: \
         {wait:#}\n{}",
        worker.transcript()
    );

    // **Settled, not parked**: the root dispatch ended paused, so the hub holds
    // no placement wait and the worker is free.
    let report = mesh.report(&execution);
    assert_holds_no_placement_wait(
        &report,
        "the hub is still holding a placement wait for a root dispatch a paused result settled",
    );
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(rows.len(), 1, "{rows:#?}");
    assert_eq!(
        (
            rows[0]["node"].as_str(),
            rows[0]["site"].as_str(),
            rows[0]["status"].as_str()
        ),
        (Some("flow.placed_asking"), Some(""), Some("settled")),
        "the paused root instance is not settled on the board: {rows:#?}"
    );

    let id = wait["wait_id"].as_str().expect("the wait names itself");
    let answered = mesh.resume(&execution, id, &json!({ "decision": "approve" }));
    assert_eq!(answered.status, 202, "{}", body_of(&answered));

    let outputs = mesh.completed(&execution);
    assert_eq!(
        (outputs["signature"].as_str(), outputs["approval"].as_str()),
        (Some("inspected-before-asking"), Some("approve")),
        "the instance did not finish after the answer; the worker said:\n{}",
        worker.transcript()
    );

    // The answer sent the **whole instance** back through dispatch: a second
    // root dispatch, to the same placement, settled with the instance's value.
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (
                row["placement"].as_str(),
                row["node"].as_str(),
                row["site"].as_str(),
                row["status"].as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some(PLACEMENT),
                Some("flow.placed_asking"),
                Some(""),
                Some("settled")
            ),
            (
                Some(PLACEMENT),
                Some("flow.placed_asking"),
                Some(""),
                Some("settled")
            ),
        ],
        "the answered pause did not redispatch the instance: {rows:#?}"
    );

    // **Replayed, not re-issued**: one script, one request — the redispatch read
    // the pre-pause model call out of the effect history (§7.2).
    let requests = mesh.provider.requests();
    assert_eq!(
        requests.len(),
        1,
        "the redispatch paid for a model call the journal already held: {requests:#?}"
    );
    // …and the answer is the pause's own effect record, at the wait's own site.
    let human = harness::journal_rows(
        &mesh.project,
        "SELECT key FROM effects WHERE kind = 'human'",
    );
    assert_eq!(
        human,
        json!([{ "key": format!("{id}#human/0") }]),
        "the answer is not journaled once at the pause's own site: {human:#}"
    );
}

/// The cancel route, answered `202` for an execution still open.
fn cancel(mesh: &Mesh, execution: &str) {
    let cancelled = mesh.send(Request::post(format!("/executions/{execution}/cancel")));
    assert_eq!(cancelled.status, 202, "{}", body_of(&cancelled));
}

/// The outcome the finally boundary at `key` recorded in this execution, read
/// off the hub's journal — `None` where no boundary was journaled.
fn boundary_outcome(mesh: &Mesh, execution: &str, key: &str) -> Option<String> {
    let rows = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT payload FROM effects \
             WHERE execution = '{execution}' AND kind = 'finally' AND \"key\" = '{key}'"
        ),
    );
    let payload = rows.as_array()?.first()?["payload"].as_str()?.to_string();
    let recorded: Value = serde_json::from_str(&payload).expect("a boundary's payload is JSON");
    recorded["outcome"].as_str().map(str::to_string)
}

/// **A cancel at a placed flow's question ends the instance on a worker, and
/// its finally node runs there** (PRD resolved q75 ruling e: "releases its
/// waits, runs its `finally:`"; q76, q84; grammar §7.9; Decision D171;
/// `docs/distributed.md` §3.2, §3.4).
///
/// `flow.placed_asking`'s instance runs `inspect` on the worker and reaches
/// `ask`: the root dispatch settles *paused*, and the question waits on the
/// hub's board with no worker running anything of the instance. The cancel
/// releases the question — and finds an instance that has begun with nobody to
/// end it. The hub cannot run `close_out` itself (nothing of a placed flow runs
/// on the hub), so it hands a worker the instance's **end**: a second root
/// dispatch, carrying the effect history and `cancelled: true`. The worker
/// replays `inspect` out of the history, the cancel fires at `ask` — the
/// frontier — and the instance ends `cancelled` there and runs its finally
/// node. The execution settles `cancelled` on that dispatch's cancelled result:
/// its trace is the instance's, `ask` cancelled and `close_out` last, the
/// boundary was journaled from the worker at outcome `cancelled`, no model call
/// was made a second time, and the answer the question waited for is refused
/// naming the status. Without the end dispatch the execution settles with one
/// dispatch row, no `close_out` entry and no boundary at all.
#[test]
fn a_cancel_at_a_placed_flows_question_runs_its_finally_node_on_a_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-before-the-cancel"));
    let mut worker = mesh.worker("placed-flow-cancel-at-question");

    let execution = mesh.start_execution("/placed-askings", &json!({ "path": "release.dmg" }));
    let wait = mesh.paused(&execution);
    let id = wait["wait_id"]
        .as_str()
        .expect("the wait names itself")
        .to_string();
    assert_holds_no_placement_wait(
        &mesh.report(&execution),
        "a paused root dispatch is settled, and no worker holds the instance while it asks",
    );

    let asked = Instant::now();
    cancel(&mesh, &execution);
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "interrupted" && report["status"] != "running"
    });
    assert_eq!(
        report["status"],
        "cancelled",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to end a placed flow at its question",
        asked.elapsed()
    );
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(report.get("finally_error").is_none(), "{report:#}");
    assert!(
        report["interrupts"].as_array().is_none_or(Vec::is_empty),
        "the released question is still published: {report:#}"
    );
    assert!(
        report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a cancelled execution still holds a placement wait: {report:#}"
    );
    assert_eq!(
        nodes_of(&report["trace"]),
        ["inspect", "ask", "close_out"],
        "the instance's end did not come home with its finally node's entry last: {report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(report["trace"][1]["outcome"], "cancelled", "{report:#}");
    assert_eq!(report["trace"][2]["outcome"], "completed", "{report:#}");
    assert_eq!(
        boundary_outcome(&mesh, &execution, "close_out/0#finally/0").as_deref(),
        Some("cancelled"),
        "the finally node did not run at outcome `cancelled`: {}",
        worker.transcript()
    );

    // The end was a dispatch a worker ran: a second root dispatch at the empty
    // path, settled by the worker's cancelled result.
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (
                row["node"].as_str(),
                row["site"].as_str(),
                row["wait"].as_str(),
                row["status"].as_str(),
                row["outcome"].as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some("flow.placed_asking"),
                Some(""),
                Some("flow.placed_asking/0"),
                Some("settled"),
                Some("value")
            ),
            (
                Some("flow.placed_asking"),
                Some(""),
                Some("flow.placed_asking/1"),
                Some("settled"),
                Some("error")
            ),
        ],
        "the cancel did not hand the instance's end to a worker: {rows:#?}"
    );
    assert!(
        rows[1]["session"]
            .as_str()
            .is_some_and(|session| session.starts_with("wrk_")),
        "a worker session held the end: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the end replayed `inspect`'s model call rather than making it again"
    );
    let human = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k FROM effects WHERE execution = '{execution}' AND kind = 'human'"
        ),
    );
    assert_eq!(
        human,
        json!([]),
        "the released question was journaled as answered"
    );
    let refused = mesh.resume(&execution, &id, &json!({ "decision": "approve" }));
    assert_eq!(refused.status, 409, "{}", body_of(&refused));
    assert_eq!(
        refused.json()["status"],
        "cancelled",
        "{}",
        body_of(&refused)
    );
    assert!(
        worker.running(),
        "the worker stopped with the end it ran: {}",
        worker.transcript()
    );
}

/// **A cancel of a placed flow's root dispatch that no worker ever took ends the
/// execution `cancelled` and runs no finally node** (grammar §7.9: an instance
/// "never begun … has no end to run anything at"; PRD resolved q75; Decision
/// D171; `docs/distributed.md` §6.4).
///
/// With no worker claiming `mac`, `flow.placed_asking`'s root dispatch parks,
/// and nothing of the instance runs anywhere. The cancel withdraws the parked
/// row, as it withdraws any parked dispatch, and the execution settles
/// `cancelled` at once: the instance never began, so there is no end to hand a
/// worker — no second dispatch, no boundary, no effect of any kind — where an
/// end dispatch made anyway would have left the execution open, parked for a
/// worker nobody started.
#[test]
fn a_cancel_of_a_placed_flows_untaken_root_dispatch_runs_no_finally_node() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let execution = mesh.start_execution("/placed-askings", &json!({ "path": "release.dmg" }));
    let parked = mesh.until(&execution, "parked on its placement", |report| {
        !report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty)
    });
    assert_eq!(
        parked["placement_waits"][0]["status"], "parked",
        "{parked:#}"
    );

    let asked = Instant::now();
    cancel(&mesh, &execution);
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running"
    });
    assert_eq!(report["status"], "cancelled", "{report:#}");
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to end a parked placed flow",
        asked.elapsed()
    );
    assert!(report.get("error").is_none(), "{report:#}");
    assert!(report.get("finally_error").is_none(), "{report:#}");
    assert!(
        report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a cancelled execution still holds a placement wait: {report:#}"
    );
    assert_eq!(
        nodes_of(&report["trace"]),
        Vec::<String>::new(),
        "{report:#}"
    );

    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.len(),
        1,
        "an end was dispatched for an instance that never began: {rows:#?}"
    );
    assert_eq!(rows[0]["status"], "superseded", "{rows:#?}");
    assert!(
        rows[0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("was cancelled")),
        "the parked row was not withdrawn as the cancel: {rows:#?}"
    );
    let effects = harness::journal_rows(
        &mesh.project,
        &format!("SELECT \"key\" AS k FROM effects WHERE execution = '{execution}'"),
    );
    assert_eq!(
        effects,
        json!([]),
        "something of an instance that never began ran"
    );
    assert!(mesh.provider.requests().is_empty(), "a model call was made");
}

/// Where `flow.placed_reattempted`'s instance stands, effect-wise, once its
/// first attempt was lost: what that attempt's worker streamed home before it
/// went.
const LOST_ATTEMPTS_EFFECT: &str = "build/0/inspect/0#model/0";

/// Drive `flow.placed_reattempted` to the state the two cases below start from,
/// and answer the execution and when it started.
///
/// The `build` node's first attempt is a placed instance a worker runs: its
/// `inspect` call is answered at once and journaled on the hub under `build/0`,
/// and its `reinspect` call is held. The worker is then lost inside it — killed,
/// runners and all, which is a machine that went away (§7.3) — so the liveness
/// sweep supersedes `build/0/0` (§6.3) and the node's `retry:` opens the next
/// attempt: a fresh instance at the **same** instance path (grammar §8.5),
/// whose dispatch `build/0/1` parks with nothing claiming `mac`. That attempt
/// never began, though the journal holds an effect under its path — the lost
/// attempt's.
fn reattempted_after_a_lost_worker(mesh: &Mesh, purpose: &str) -> (String, Instant) {
    mesh.provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "inspected-by-the-lost-attempt" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "signature": "never delivered" }))
                .after(Duration::from_secs(600)),
        ),
    ]);
    let mut worker = mesh.worker(purpose);
    let started = Instant::now();
    let execution = mesh.start_execution("/placed-reattempts", &json!({ "path": "release.dmg" }));
    until(
        "the first attempt's `inspect` call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &[LOST_ATTEMPTS_EFFECT]),
    );
    until(
        "the first attempt's `reinspect` call reached the provider",
        || worker.transcript(),
        || mesh.provider.requests().len() == 2,
    );
    let holding = mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    assert_eq!(
        holding["placement_waits"][0]["wait_id"], "build/0/0",
        "{holding:#}"
    );
    worker.kill();

    let parked = mesh.until(&execution, "parked the retry's attempt", |report| {
        (report["placement_waits"][0]["wait_id"] == "build/0/1"
            && report["placement_waits"][0]["status"] == "parked")
            || report["status"] != "running"
    });
    assert_eq!(
        (
            parked["status"].as_str(),
            parked["placement_waits"][0]["wait_id"].as_str(),
            parked["placement_waits"][0]["status"].as_str(),
        ),
        (Some("running"), Some("build/0/1"), Some("parked")),
        "the retry's attempt did not park after the lost one: {parked:#}"
    );
    (execution, started)
}

/// The two dispatch rows `flow.placed_reattempted`'s `build` node made — the
/// lost attempt's, superseded by the liveness sweep, and the retry's, ended by
/// whatever stopped it while it was parked — and **no third**: an end
/// dispatched for an instance that never began would be `build/0/2`. Answers
/// the retry's row's detail.
fn assert_no_end_for_the_untaken_retry(mesh: &Mesh, execution: &str) -> String {
    let placed = dispatches_of(mesh, execution);
    assert_eq!(
        placed
            .iter()
            .map(|row| (row["wait"].as_str(), row["status"].as_str()))
            .collect::<Vec<_>>(),
        [
            (Some("build/0/0"), Some("superseded")),
            (Some("build/0/1"), Some("superseded")),
        ],
        "an end was dispatched for a retry no worker took, on the strength of the effects the \
         attempt before it made: {placed:#?}"
    );
    let rows = mesh.dispatch_rows(execution);
    assert!(
        rows[0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("made no request")),
        "the lost attempt was not superseded by the liveness sweep: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        2,
        "something of the retry's attempt ran: only the lost attempt's two calls were made"
    );
    rows[1]["detail"].as_str().unwrap_or_default().to_string()
}

/// **A cancel of a placed flow's retry that no worker took ends the execution
/// `cancelled` at once — the effects the attempt before it made do not make it
/// begun** (grammar §7.9, §8.5; PRD resolved q75, q84; Decision D171;
/// `docs/distributed.md` §2, §6.3).
///
/// `flow.placed_reattempted`'s first attempt ran on a worker and streamed a
/// model call home under `build/0` before the worker was lost; its retry is
/// parked at `build/0/1` with no worker claiming `mac`. Each attempt is a
/// fresh instance at the same instance path, so the parked one has not begun:
/// the cancel withdraws its row, as it withdraws any parked dispatch, and the
/// execution settles `cancelled` straight away. Read off the journal's effects
/// at `build/0`, the parked attempt would look begun, and the cancel would park
/// an end for it — `cancelled: true`, which nothing withdraws — leaving the
/// execution open for a worker nobody started, and handing one that did join
/// the lost attempt's history to replay.
#[test]
fn a_cancel_of_a_placed_flows_untaken_retry_owes_no_end_for_the_attempt_lost_before_it() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let (execution, _) = reattempted_after_a_lost_worker(&mesh, "placed-flow-reattempt-cancel");

    let asked = Instant::now();
    cancel(&mesh, &execution);
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running"
    });
    assert_eq!(report["status"], "cancelled", "{report:#}");
    assert!(
        asked.elapsed() < Duration::from_secs(10),
        "the cancel took {:?} to end a retry no worker took",
        asked.elapsed()
    );
    assert!(report.get("finally_error").is_none(), "{report:#}");
    assert!(
        report["placement_waits"]
            .as_array()
            .is_none_or(Vec::is_empty),
        "a cancelled execution still holds a placement wait: {report:#}"
    );

    let detail = assert_no_end_for_the_untaken_retry(&mesh, &execution);
    assert!(
        detail.contains("was cancelled") && !detail.contains("held it"),
        "the retry's parked row was not withdrawn as the cancel's, untaken: {detail}"
    );
}

/// **A deadline over a placed flow's retry that no worker took fails the node
/// on its deadline — the effects the attempt before it made do not make it
/// begun** (grammar §7.9, §8.5, §9.2; Decision D171; `docs/distributed.md` §2,
/// §6.3, §6.5).
///
/// The same lost attempt and the same parked retry as the cancel's case, left
/// for `build`'s twenty-second `timeout:` to end. The parked attempt never
/// began, so the deadline withdraws its row as one no worker took, and the node
/// fails on its budget: the execution settles `failed`, on the timeout, within
/// a few seconds of the budget running out. Read off the journal's effects at
/// `build/0`, the parked attempt would look begun, and the deadline would park
/// a `stopped: true` end for it — which nothing withdraws and the node waits
/// for — so the node's own deadline would stop bounding it, and the execution
/// would wait for a worker indefinitely.
#[test]
fn a_deadline_over_a_placed_flows_untaken_retry_fails_the_node_and_owes_no_end() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    let (execution, started) =
        reattempted_after_a_lost_worker(&mesh, "placed-flow-reattempt-deadline");

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "running" && report["status"] != "interrupted"
    });
    let settled_after = started.elapsed();
    assert_eq!(report["status"], "failed", "{report:#}");
    assert!(
        report["error"]
            .as_str()
            .is_some_and(|error| error.contains("`build` timed out")),
        "the execution did not fail on `build`'s deadline: {report:#}"
    );
    assert!(
        settled_after < Duration::from_secs(20 + 10),
        "the execution took {settled_after:?} to settle on a twenty-second deadline: {report:#}"
    );
    assert!(report.get("finally_error").is_none(), "{report:#}");

    let detail = assert_no_end_for_the_untaken_retry(&mesh, &execution);
    assert!(
        detail.contains("before any worker took it"),
        "the retry's parked row was not withdrawn as one no worker took: {detail}"
    );
}

/// **A hub restarted mid-cancel at a placed flow's question hands the
/// instance's end to a worker, and its finally node runs there** (PRD resolved
/// q75 ruling e, `docs/durability.md` §6.1; Decision D171).
///
/// The journal is left as a `SIGKILL` right after the cancel route recorded its
/// request would leave it: the request on the `cancellations` table, the root
/// dispatch settled paused with no answer recorded. The recovered replay
/// reaches the unanswered question — the frontier, where the cancel fires —
/// and the instance it stops there is a placed flow's that has begun, so the
/// recovery makes the end dispatch the live cancel would have made, and the
/// worker that re-joins the restarted hub runs `close_out` at outcome
/// `cancelled`. Without it the recovered execution settles `cancelled` having
/// never run the finally node.
#[test]
fn a_hub_restarted_mid_cancel_at_a_placed_flows_question_runs_its_finally_node_on_a_worker() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-before-the-restart"));
    let worker = mesh.worker("placed-flow-restart-mid-cancel");
    let execution = mesh.start_execution("/placed-askings", &json!({ "path": "release.dmg" }));
    mesh.paused(&execution);

    mesh.restart_after(|project| {
        let wrote = harness::journal_sql(
            project,
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
    });

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "cancelled" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "cancelled",
        "the recovered execution did not complete its cancel: {report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        boundary_outcome(&mesh, &execution, "close_out/0#finally/0").as_deref(),
        Some("cancelled"),
        "the recovery completed the cancel without running the finally node: {report:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        nodes_of(&report["trace"]),
        ["inspect", "ask", "close_out"],
        "{report:#}"
    );
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (row["wait"].as_str(), row["status"].as_str()))
            .collect::<Vec<_>>(),
        [
            (Some("flow.placed_asking/0"), Some("settled")),
            (Some("flow.placed_asking/1"), Some("settled")),
        ],
        "the recovery did not hand the instance's end to a worker: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the end made `inspect`'s model call a second time"
    );
}

/// **A hub restarted mid-cancel while a worker holds a placed flow's instance
/// leaves the instance's end to that worker, and dispatches no end beside it**
/// (grammar §7.9: a finally node "runs once when it ends"; PRD resolved q75
/// ruling e; Decision D171; `docs/distributed.md` §2, §5).
///
/// `flow.placed_holding`'s instance makes its model call on the worker — the
/// record reaches the hub's journal, so the instance has begun — and then sits
/// in `hold` for ten minutes. The journal is left as a crash right after the
/// cancel route recorded its request would leave it, with the root dispatch
/// still `dispatched` to a session the dead process issued. The recovered
/// replay reaches that row as the cancel's frontier and settles the execution
/// `cancelled` — and, because a worker may still be running the instance, it
/// makes **no end dispatch**: the worker holding it is the one that ends it.
/// Its next request meets the restarted hub's `410`, it stops the runner (§5's
/// stop, never a cancel), and the instance ends there `failed` and runs
/// `settle_up` once, its boundary journaled under the renewed session. An end
/// dispatched beside it would have run `settle_up` a second time, on whichever
/// worker took it — the one row this test counts.
#[test]
fn a_restart_while_a_worker_holds_a_cancelled_placed_flow_leaves_its_end_to_that_worker() {
    let Some(mut mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-before-the-restart"));
    let mut worker = mesh.worker("placed-flow-restart-while-held");

    let execution = mesh.start_execution("/placed-holdings", &json!({ "path": "release.dmg" }));
    until(
        "the instance's model call reached the hub's journal",
        || worker.transcript(),
        || harness::journal_holds(&mesh.project, &["inspect/0#model/0"]),
    );
    mesh.until(&execution, "was taken by the worker", |report| {
        report["placement_waits"][0]["status"] == "dispatched"
    });
    // Inside `hold`: the instance has begun, and a worker is running it.
    until(
        "the instance's long node was running on the worker",
        || worker.transcript(),
        || {
            worker.runners().is_none_or(|running| {
                running.iter().any(|runner| {
                    u32::try_from(*runner)
                        .ok()
                        .and_then(|runner| children_running(runner, "sleep"))
                        .is_some_and(|commands| !commands.is_empty())
                })
            })
        },
    );

    mesh.restart_after(|project| {
        let wrote = harness::journal_sql(
            project,
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
    });

    let report = mesh.until(&execution, "settled", |report| {
        report["status"] == "cancelled" || report["status"] == "failed"
    });
    assert_eq!(
        report["status"],
        "cancelled",
        "the recovered execution did not complete its cancel: {report:#}\n{}",
        worker.transcript()
    );
    assert!(report.get("error").is_none(), "{report:#}");

    // The worker that held the instance ends it, and its finally node runs
    // there: stopped by the `410`, so at outcome `failed` (§5).
    until(
        "the finally node ran on the worker that held the instance",
        || worker.transcript(),
        || boundary_outcome(&mesh, &execution, "settle_up/0#finally/0").is_some(),
    );
    assert_eq!(
        boundary_outcome(&mesh, &execution, "settle_up/0#finally/0").as_deref(),
        Some("failed"),
        "{}",
        worker.transcript()
    );
    until(
        "the runner the restart orphaned is gone",
        || worker.transcript(),
        || worker.runners().is_none_or(|running| running.is_empty()),
    );

    let rows = mesh.dispatch_rows(&execution);
    assert_eq!(
        rows.len(),
        1,
        "the recovery dispatched an end beside the instance a worker was still running, so \
         its finally node would run twice: {rows:#?}\n{}",
        worker.transcript()
    );
    assert_eq!(rows[0]["status"], "superseded", "{rows:#?}");
    assert!(
        rows[0]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("was cancelled while a worker held it")),
        "the held dispatch was not ended as the cancel's, held by a worker: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the instance's model call was made a second time"
    );
    assert!(
        worker.running(),
        "the worker stopped somewhere across the restart: {}",
        worker.transcript()
    );
}

/// **A placed flow's question abandoned by the end around it ends the instance
/// on a worker, `failed`, and its finally node runs there** (grammar §7.9:
/// "one that was waiting on a person is stopped the same way: its pause is
/// abandoned … and it ends failed, its finally node run"; PRD resolved q76;
/// Decision D171; `docs/distributed.md` §3.2).
///
/// `flow.placed_asking_raced` runs a `flow:` node into `flow.placed_asking`
/// beside `doomed`, which fails once the test opens its gate. The placed
/// instance runs `inspect` on the worker and reaches `ask`: its dispatch
/// settles *paused* and the question waits on the hub's board, with no worker
/// running anything of the instance. Then `doomed` fails, the flow around the
/// instance ends, and its end abandons the question. Unplaced, the instance
/// would end `failed` there and run its finally node; the hub can run none of
/// a placed flow, so it hands a worker the instance's **end** — a second
/// dispatch at the instance's path, its effect history and `stopped: true` —
/// and the worker replays `inspect`, stops the instance at `ask`, and runs
/// `close_out` at outcome `failed`. The flow around it settles `failed` after
/// that end, with no model call made twice and no answer journaled. Without
/// the end dispatch the execution settles with one dispatch row and no
/// boundary at all — the finally node run nowhere.
#[test]
fn a_placed_flows_question_abandoned_by_the_end_around_it_runs_its_finally_node_on_a_worker() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider.enqueue(inspecting(
        "release.dmg",
        "inspected-before-the-sibling-failed",
    ));
    let mut worker = mesh.worker("placed-flow-question-abandoned");
    let gate = mesh.project.join("doomed.gate");

    let execution = mesh.start_execution(
        "/placed-asking-races",
        &json!({ "path": "release.dmg", "gate": gate.display().to_string() }),
    );
    let asking = mesh.until(
        &execution,
        "published the placed instance's question",
        |report| {
            report["interrupts"]
                .as_array()
                .is_some_and(|waits| !waits.is_empty())
                || report["status"] == "failed"
        },
    );
    assert_eq!(
        asking["interrupts"][0]["wait_id"],
        "asking/0/ask/0",
        "{asking:#}\n{}",
        worker.transcript()
    );
    assert_holds_no_placement_wait(
        &asking,
        "a paused dispatch is settled, and no worker holds the instance while it asks",
    );

    std::fs::write(&gate, b"open").expect("the gate opens");
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "interrupted" && report["status"] != "running"
    });
    assert_eq!(
        report["status"],
        "failed",
        "{report:#}\n{}",
        worker.transcript()
    );
    assert!(
        report["error"]
            .as_str()
            .is_some_and(|error| error.contains("`doomed`")),
        "the execution failed on its sibling, not on the abandoned question: {report:#}"
    );
    assert!(
        report["interrupts"].as_array().is_none_or(Vec::is_empty),
        "the abandoned question is still published: {report:#}"
    );
    assert_eq!(
        boundary_outcome(&mesh, &execution, "asking/0/close_out/0#finally/0").as_deref(),
        Some("failed"),
        "the instance whose question the end around it abandoned did not run its finally node \
         at outcome `failed`: {report:#}\n{}",
        worker.transcript()
    );

    // The end was a dispatch a worker ran: a second dispatch at the instance's
    // own path, settled by the instance's failure.
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (
                row["node"].as_str(),
                row["site"].as_str(),
                row["wait"].as_str(),
                row["status"].as_str(),
                row["outcome"].as_str()
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some("flow.placed_asking"),
                Some("asking/0"),
                Some("asking/0/0"),
                Some("settled"),
                Some("value")
            ),
            (
                Some("flow.placed_asking"),
                Some("asking/0"),
                Some("asking/0/1"),
                Some("settled"),
                Some("error")
            ),
        ],
        "the abandonment did not hand the instance's end to a worker: {rows:#?}\n{}",
        worker.transcript()
    );
    assert!(
        rows[1]["session"]
            .as_str()
            .is_some_and(|session| session.starts_with("wrk_")),
        "a worker session held the end: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the end replayed `inspect`'s model call rather than making it again"
    );
    let human = harness::journal_rows(
        &mesh.project,
        &format!(
            "SELECT \"key\" AS k FROM effects WHERE execution = '{execution}' AND kind = 'human'"
        ),
    );
    assert_eq!(
        human,
        json!([]),
        "the abandoned question was journaled as answered"
    );
    assert!(
        worker.running(),
        "the worker stopped with the end it ran: {}",
        worker.transcript()
    );
}

/// **A placed flow's redispatch the end around it stops while it is parked is
/// still owed its end on a worker** (grammar §7.9; PRD resolved q76; Decision
/// D171; `docs/distributed.md` §2, §6.5).
///
/// The same race as above, one step later. `flow.placed_asking`'s question
/// comes home from the first worker, which then leaves; the question is
/// answered with no worker to take the redispatch, so it parks — the instance
/// has begun, and nobody is running it. Then `doomed` fails, and the end around
/// the instance stops waiting for it. The hub withdraws the parked redispatch —
/// under a detail of its own, which says no worker took it — and makes the
/// instance's end in its place, `stopped: true`, which parks in turn. A second
/// worker joins and takes the end: it replays `inspect` and the answered
/// question, the instance's body has nothing left past its record, and
/// `close_out` runs there before the flow around it settles `failed`. Without
/// the end the parked row is simply superseded, nothing ever runs `close_out`,
/// and the second worker is handed nothing.
#[test]
fn a_placed_flows_parked_redispatch_the_end_around_it_stops_is_still_owed_its_end() {
    let Some(mesh) = Mesh::start() else {
        return;
    };
    mesh.provider
        .enqueue(inspecting("release.dmg", "inspected-before-the-answer"));
    let gate = mesh.project.join("doomed.gate");
    let first = mesh.worker("placed-flow-redispatch-first");
    let execution = mesh.start_execution(
        "/placed-asking-races",
        &json!({ "path": "release.dmg", "gate": gate.display().to_string() }),
    );
    let asking = mesh.until(
        &execution,
        "published the placed instance's question",
        |report| {
            report["interrupts"]
                .as_array()
                .is_some_and(|waits| !waits.is_empty())
                || report["status"] == "failed"
        },
    );
    let wait = asking["interrupts"][0]["wait_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the question names itself: {asking:#}"))
        .to_string();
    drop(first);

    // Answered with no worker to take the redispatch: it parks.
    let answered = mesh.resume(&execution, &wait, &json!({ "decision": "approve" }));
    assert_eq!(answered.status, 202, "{}", body_of(&answered));
    mesh.until(&execution, "parked the instance's redispatch", |report| {
        report["placement_waits"][0]["status"] == "parked"
    });

    // The end around the instance stops waiting for the parked redispatch, and
    // the instance's end parks in its place.
    std::fs::write(&gate, b"open").expect("the gate opens");
    until(
        "the stopped redispatch was withdrawn and the instance's end parked",
        || format!("{:#?}", dispatches_of(&mesh, &execution)),
        || {
            let rows = dispatches_of(&mesh, &execution);
            rows.len() == 3 && rows[1]["status"] == "superseded" && rows[2]["status"] == "parked"
        },
    );
    let withdrawn = mesh.dispatch_rows(&execution);
    assert!(
        withdrawn[1]["detail"]
            .as_str()
            .is_some_and(|detail| detail.contains("before any worker took it")),
        "the parked redispatch was not withdrawn as one no worker took: {withdrawn:#?}"
    );

    let mut second = mesh.worker("placed-flow-redispatch-end");
    let report = mesh.until(&execution, "settled", |report| {
        report["status"] != "interrupted" && report["status"] != "running"
    });
    assert_eq!(
        report["status"],
        "failed",
        "{report:#}\n{}",
        second.transcript()
    );
    assert!(
        report["error"]
            .as_str()
            .is_some_and(|error| error.contains("`doomed`")),
        "the execution failed on its sibling: {report:#}"
    );
    assert!(
        boundary_outcome(&mesh, &execution, "asking/0/close_out/0#finally/0").is_some(),
        "the instance the end around it stopped while it was parked never ran its finally \
         node: {report:#}\n{}",
        second.transcript()
    );
    let rows = dispatches_of(&mesh, &execution);
    assert_eq!(
        rows.iter()
            .map(|row| (row["wait"].as_str(), row["status"].as_str()))
            .collect::<Vec<_>>(),
        [
            (Some("asking/0/0"), Some("settled")),
            (Some("asking/0/1"), Some("superseded")),
            (Some("asking/0/2"), Some("settled")),
        ],
        "{rows:#?}\n{}",
        second.transcript()
    );
    assert!(
        rows[2]["session"]
            .as_str()
            .is_some_and(|session| session.starts_with("wrk_")),
        "a worker session held the end: {rows:#?}"
    );
    assert_eq!(
        mesh.provider.requests().len(),
        1,
        "the end replayed `inspect`'s model call rather than making it again"
    );
    assert!(second.running(), "{}", second.transcript());
}

/// **A `run` refuses a placed agent's node, and serves no mesh for it**
/// (Decision D171; PRD resolved q84 rules on the instances of a placed flow and
/// on nothing else, so a placed agent's or tool's node under `run` is what it
/// was before a flow could be placed: `PlacementUnreachable`).
///
/// `flow.release`'s `sign` node is `agent.signer`'s, a member of `mac`. A `run`
/// of it fails at that node at once — no `mesh:` line on stderr, no listener,
/// no dispatch row — rather than opening a mesh and parking for a worker
/// nobody started. And a `run` whose mesh is already up refuses the same node
/// the same way: `flow.placed_then_signed`'s `build` instance runs on the
/// worker that joined the run, and its `sign` node is then refused, so whether
/// a placed agent's node is dispatched does not depend on what the run reached
/// before it.
#[test]
fn a_run_refuses_a_placed_agents_node_and_serves_no_mesh_for_it() {
    let Some(project) = harness::scratch_project("placed-agent-run") else {
        return;
    };
    let _scratch = harness::Scratch::at(project.clone());
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(inspecting("hotfix.dmg", "inspected-before-the-refusal"));
    let entrypoint = harness::fixture(FIXTURE);
    let environment = environment(&provider);
    let refused = |node: &str| {
        format!(
            "`{node}` is placed on `{PLACEMENT}`, and this process has no mesh for a worker to \
             join"
        )
    };

    let mut running = harness::run_target_spawned(
        &project,
        &entrypoint,
        TARGET,
        "flow.release",
        &[("path", "release.dmg")],
        &environment,
    );
    let deadline = Instant::now() + PATIENCE;
    while running.still_running() {
        assert!(
            Instant::now() < deadline,
            "the run of a placed agent's node is still going — parked for a worker — rather \
             than refused: {}",
            running.said()
        );
        std::thread::sleep(Duration::from_millis(50));
    }
    let run = running.finish();
    let said = run.failed();
    assert!(
        said.contains(&refused("flow.release.sign")),
        "the run was not refused with `PlacementUnreachable` at the placed agent's node: {said}"
    );
    assert!(
        !said.lines().any(|line| line.starts_with("mesh: ")),
        "the run served a mesh for a placed agent's node: {said}"
    );
    assert_eq!(
        run_dispatches(&project, "flow.release"),
        Vec::<Value>::new(),
        "the run dispatched a placed agent's node"
    );

    // The mesh up for a placed flow's instance first, and the agent's node after.
    let running = harness::run_target_spawned(
        &project,
        &entrypoint,
        TARGET,
        "flow.placed_then_signed",
        &[("path", "hotfix.dmg")],
        &environment,
    );
    let hub = mesh_of(&running);
    let worker = Worker::start(&hub, &environment, "placed-flow-before-agent", None);
    let run = running.finish();
    let said = run.failed();
    assert!(
        said.contains(&refused("flow.placed_then_signed.sign")),
        "a run whose mesh was up dispatched a placed agent's node rather than refusing it: \
         {said}\n{}",
        worker.transcript()
    );
    let rows = run_dispatches(&project, "flow.placed_then_signed");
    assert_eq!(
        rows.iter()
            .map(|row| (row["node"].as_str(), row["status"].as_str()))
            .collect::<Vec<_>>(),
        [(Some("flow.placed_build"), Some("settled"))],
        "the placed flow's instance ran on the worker and nothing else was dispatched: {rows:#?}"
    );
}
