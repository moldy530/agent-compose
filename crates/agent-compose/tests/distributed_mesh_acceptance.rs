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
    /// Removed when the test ends.
    _scratch: harness::Scratch,
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
            _scratch: harness::Scratch::at(project.clone()),
            project,
            entrypoint,
            _composition: composition,
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
        let data_dir = data_dir.unwrap_or_else(|| scratch_data_dir(purpose));
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .arg("worker")
            .args(["--hub", base_url])
            .args(["--claim", PLACEMENT])
            .args(["--token-env", TOKEN_VARIABLE])
            .arg("--data-dir")
            .arg(&data_dir)
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

    /// Everything this worker has written, for a failure message.
    fn transcript(&self) -> String {
        self.said
            .lock()
            .expect("the buffer is not poisoned")
            .join("\n")
    }

    /// Kill this worker and everything it started — the disconnect of §7.3.
    fn kill(&mut self) {
        end(&mut self.child);
        self.reaped = true;
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
            end(&mut self.child);
        }
        let _ = std::fs::remove_dir_all(&self.data_dir);
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
        .enqueue_all(signing("signed-for-the-collector"));
    let _worker = mesh.worker("exporting");

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
    assert_eq!(body["trace_version"], 4, "{body:#}");

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
