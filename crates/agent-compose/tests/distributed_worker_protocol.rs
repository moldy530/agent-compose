//! What `agent-compose worker` does with each status
//! (`docs/distributed.md` §3, §4, §5).
//!
//! The third of the three suites this protocol has, and the only one that can
//! ask this question. `tests/distributed_hub_wire.rs` holds the **hub** to §3 by
//! speaking the wire to it; `tests/distributed_mesh_acceptance.rs` runs a real
//! mesh and asks whether a distributed execution works. Neither can say what a
//! worker does when a hub answers something the hub would never say — and §10.1
//! calls the worker's status handling "the whole of what a worker's error
//! handling has to decide".
//!
//! So the hub here is a **fixture**: a few hundred lines of HTTP that answers
//! whatever a test scripts and records what it was asked. The worker is the real
//! binary, and what is asserted is what it did next.
//!
//! # The artifact these tests serve
//!
//! Not a compiled project: three files, hashed the way the compiler hashes a
//! tree, with a `manifest.json` naming a **node runner of the test's own** —
//! twenty lines of Bun that writes one effect line and one result line. That is
//! the whole of the runner contract `src/worker-node.ts` implements, so a test
//! about `/workers/effects` and `/workers/result` can drive it without a
//! composition, a provider or a graph.
//!
//! It also exercises the reader from the other side: the archive here is written
//! by this file, and the hash is computed by the compiler's own function — so a
//! worker that read the format loosely, or hashed it differently, fails here
//! rather than against the one writer it usually meets.

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

/// The credential every worker in this file presents.
const TOKEN: &str = "a-join-token-nobody-else-has";
const TOKEN_VARIABLE: &str = "MESH_JOIN_TOKEN";
const PLACEMENT: &str = "mac";

/// How long a test waits for the worker to do the next thing.
const PATIENCE: Duration = Duration::from_secs(30);

// ---------------------------------------------------------------------------
// The hub, as a fixture
// ---------------------------------------------------------------------------

/// One answer the fixture hub gives.
#[derive(Clone)]
struct Reply {
    status: u16,
    body: Vec<u8>,
    content_type: &'static str,
}

impl Reply {
    fn json(status: u16, body: &Value) -> Self {
        Self {
            status,
            body: serde_json::to_vec(body).expect("the body serializes"),
            content_type: "application/json",
        }
    }

    const fn empty(status: u16) -> Self {
        Self {
            status,
            body: Vec::new(),
            content_type: "application/json",
        }
    }

    const fn bytes(status: u16, body: Vec<u8>) -> Self {
        Self {
            status,
            body,
            content_type: "application/gzip",
        }
    }
}

/// One request the fixture hub was given.
#[derive(Clone, Debug)]
struct Asked {
    method: String,
    path: String,
    session: Option<String>,
    authorization: Option<String>,
    body: Value,
}

/// A hub that answers what a test scripts and records what it was asked.
struct FixtureHub {
    base_url: String,
    asked: Arc<Mutex<Vec<Asked>>>,
    scripted: Arc<Mutex<HashMap<String, VecDeque<Reply>>>>,
    fallback: Arc<Mutex<HashMap<String, Reply>>>,
    stopping: Arc<AtomicBool>,
}

impl FixtureHub {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("a loopback port");
        let address = listener.local_addr().expect("the listener has an address");
        let asked: Arc<Mutex<Vec<Asked>>> = Arc::new(Mutex::new(Vec::new()));
        let scripted: Arc<Mutex<HashMap<String, VecDeque<Reply>>>> =
            Arc::new(Mutex::new(HashMap::new()));
        let fallback: Arc<Mutex<HashMap<String, Reply>>> = Arc::new(Mutex::new(HashMap::new()));
        let stopping = Arc::new(AtomicBool::new(false));
        let hub = Self {
            base_url: format!("http://{address}"),
            asked: Arc::clone(&asked),
            scripted: Arc::clone(&scripted),
            fallback: Arc::clone(&fallback),
            stopping: Arc::clone(&stopping),
        };
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                if stopping.load(Ordering::SeqCst) {
                    return;
                }
                let Ok(stream) = stream else { continue };
                let asked = Arc::clone(&asked);
                let scripted = Arc::clone(&scripted);
                let fallback = Arc::clone(&fallback);
                std::thread::spawn(move || {
                    serve_one(&stream, &asked, &scripted, &fallback);
                });
            }
        });
        hub
    }

    /// Queue one answer for the next request to `route`.
    fn script(&self, route: &str, reply: Reply) {
        self.scripted
            .lock()
            .expect("the script is not poisoned")
            .entry(route.to_string())
            .or_default()
            .push_back(reply);
    }

    /// What `route` answers once its queue is empty.
    fn always(&self, route: &str, reply: Reply) {
        self.fallback
            .lock()
            .expect("the fallbacks are not poisoned")
            .insert(route.to_string(), reply);
    }

    /// Every request this hub has been given.
    fn asked(&self) -> Vec<Asked> {
        self.asked.lock().expect("the log is not poisoned").clone()
    }

    /// Every request to one route.
    fn asked_at(&self, route: &str) -> Vec<Asked> {
        self.asked()
            .into_iter()
            .filter(|request| request.path.starts_with(route))
            .collect()
    }

    /// Wait until `wanted` answers `true`, or fail describing what was asked.
    fn until(&self, what: &str, wanted: impl Fn(&[Asked]) -> bool) -> Vec<Asked> {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            let asked = self.asked();
            if wanted(&asked) {
                return asked;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!(
            "the worker never {what}; it asked for:\n{:#?}",
            self.asked()
        );
    }
}

impl Drop for FixtureHub {
    fn drop(&mut self) {
        self.stopping.store(true, Ordering::SeqCst);
        // One connection of its own, so the accept loop wakes and sees the flag.
        let _ = TcpStream::connect(self.base_url.trim_start_matches("http://"));
    }
}

/// Read one request, record it, and answer it.
fn serve_one(
    stream: &TcpStream,
    asked: &Mutex<Vec<Asked>>,
    scripted: &Mutex<HashMap<String, VecDeque<Reply>>>,
    fallback: &Mutex<HashMap<String, Reply>>,
) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if reader.read_line(&mut line).is_err() || line.trim().is_empty() {
        return;
    }
    let mut parts = line.split_whitespace();
    let method = parts.next().unwrap_or_default().to_string();
    let path = parts.next().unwrap_or_default().to_string();

    let mut length = 0usize;
    let mut session = None;
    let mut authorization = None;
    loop {
        let mut header = String::new();
        if reader.read_line(&mut header).is_err() {
            return;
        }
        let header = header.trim_end().to_string();
        if header.is_empty() {
            break;
        }
        let Some((name, value)) = header.split_once(':') else {
            continue;
        };
        let value = value.trim().to_string();
        match name.to_ascii_lowercase().as_str() {
            "content-length" => length = value.parse().unwrap_or(0),
            "x-worker-session" => session = Some(value),
            "authorization" => authorization = Some(value),
            _ => {}
        }
    }
    let mut body = vec![0u8; length];
    if length > 0 && reader.read_exact(&mut body).is_err() {
        return;
    }

    let route = route_of(&path);
    asked.lock().expect("the log is not poisoned").push(Asked {
        method,
        path: path.clone(),
        session,
        authorization,
        body: serde_json::from_slice(&body).unwrap_or(Value::Null),
    });

    let reply = scripted
        .lock()
        .expect("the script is not poisoned")
        .get_mut(&route)
        .and_then(VecDeque::pop_front)
        .or_else(|| {
            fallback
                .lock()
                .expect("the fallbacks are not poisoned")
                .get(&route)
                .cloned()
        })
        .unwrap_or_else(|| Reply::empty(204));

    let mut out = stream;
    let head = format!(
        "HTTP/1.1 {} X\r\ncontent-type: {}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n",
        reply.status,
        reply.content_type,
        reply.body.len()
    );
    let _ = out.write_all(head.as_bytes());
    let _ = out.write_all(&reply.body);
    let _ = out.flush();
}

/// Which of the five routes a path is, for the script's key.
fn route_of(path: &str) -> String {
    if path.starts_with("/workers/artifact/") {
        return "/workers/artifact".to_string();
    }
    path.split('?').next().unwrap_or(path).to_string()
}

// ---------------------------------------------------------------------------
// The artifact these tests serve
// ---------------------------------------------------------------------------

/// The node runner this file's artifact carries: the whole contract, in Bun.
const RUNNER: &str = r#"// A node runner for `tests/distributed_worker_protocol.rs`, and nothing else.
//
// One dispatch on standard input; one effect line and one result line on
// standard output. That is the contract `src/worker-node.ts` implements over a
// real composition — see its header — and this is the least of it that a test
// about `/workers/effects` and `/workers/result` needs.
let held = "";
for await (const chunk of process.stdin) held += chunk;
const dispatch = JSON.parse(held);
const line = (value) => process.stdout.write(`${JSON.stringify(value)}\n`);
line({
  type: "effect",
  effect: {
    key: `${dispatch.instance_path}#model/0`,
    site: dispatch.instance_path,
    kind: "model",
    ordinal: 0,
    request: "{}",
    outcome: { kind: "value", value: "an answer the fixture runner made up" },
    refused: false,
    recorded_at: "2026-08-30T00:00:00.000Z",
  },
});
// A node that takes a while, for the one test that needs the worker to still be
// busy when the next poll is answered. Zero — the default — is the shape every
// other test wants: a dispatch that ends as soon as it has said what it did.
const linger = Number(process.env.FIXTURE_RUNNER_LINGER_MS ?? "0");
if (linger > 0) await new Promise((resolve) => setTimeout(resolve, linger));
line({ type: "result", output: { signature: "from the fixture runner" } });
"#;

/// `manifest.json`, as `codegen::worker` writes one.
const MANIFEST: &str = r#"{
  "//": ["This file was generated by agent-compose 0.0.0-dev from `main.yml`."],
  "node_runner": "runner.ts",
  "placements": [
    { "name": "mac", "environment": [] }
  ]
}
"#;

/// The artifact this file's hub serves: its files, and the hash that names it.
fn artifact() -> (String, Vec<u8>) {
    artifact_named("fixture")
}

/// A **second** artifact, so a redeployment has something to redeploy to.
///
/// One byte of `package.json` apart, which is enough: the hash is over the
/// tree's content, so two trees that differ anywhere are two artifacts (§4).
fn redeployed_artifact() -> (String, Vec<u8>) {
    artifact_named("fixture-redeployed")
}

/// One artifact, named so that two of them hash differently.
fn artifact_named(name: &str) -> (String, Vec<u8>) {
    let manifest = format!("{{ \"name\": \"{name}\", \"private\": true }}\n");
    let files: Vec<(&str, &[u8])> = vec![
        ("manifest.json", MANIFEST.as_bytes()),
        ("runner.ts", RUNNER.as_bytes()),
        // Carried so the tree looks like one a build wrote; never read here.
        ("package.json", manifest.as_bytes()),
    ];
    let hash = compose_core::codegen::artifact::hash_of(files.iter().copied());
    let mut blocks = Vec::new();
    for (path, bytes) in &files {
        blocks.extend(tar_entry(path, bytes));
    }
    blocks.extend(std::iter::repeat_n(0u8, 1024));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&blocks).expect("the encoder takes it");
    (hash, encoder.finish().expect("the encoder finishes"))
}

/// One ustar entry, written the way `src/mesh.ts` writes one.
fn tar_entry(name: &str, body: &[u8]) -> Vec<u8> {
    let mut header = vec![0u8; 512];
    header[..name.len()].copy_from_slice(name.as_bytes());
    header[100..108].copy_from_slice(b"0000644\0");
    header[108..116].copy_from_slice(b"0000000\0");
    header[116..124].copy_from_slice(b"0000000\0");
    header[124..136].copy_from_slice(format!("{:011o}\0", body.len()).as_bytes());
    header[136..148].copy_from_slice(b"00000000000\0");
    header[148..156].copy_from_slice(b"        ");
    header[156] = b'0';
    header[257..263].copy_from_slice(b"ustar\0");
    header[263..265].copy_from_slice(b"00");
    let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
    header[148..156].copy_from_slice(format!("{checksum:06o}\0 ").as_bytes());
    let mut block = header;
    block.extend_from_slice(body);
    block.extend(std::iter::repeat_n(0u8, (512 - body.len() % 512) % 512));
    block
}

/// A join answer that accepts and names `hash`.
fn accepted(hash: &str, session: &str) -> Reply {
    Reply::json(
        200,
        &json!({
            "protocol": 1,
            "compiler": compose_core::codegen::COMPILER_VERSION,
            "worker_session": session,
            "artifact": { "hash": hash, "url": format!("/workers/artifact/{hash}") },
            "poll_url": "/workers/poll",
        }),
    )
}

/// One dispatch, as `/workers/poll` answers one (§3.2).
fn dispatch(id: &str) -> Reply {
    Reply::json(
        200,
        &json!({
            "dispatch_id": id,
            "execution_id": "exec_fixture",
            "node": "flow.release.sign",
            "instance_path": "sign/0",
            "inputs": { "path": "release.dmg" },
            "effect_history": [],
        }),
    )
}

/// The same dispatch, carrying at least `bytes` of journaled `effect_history`.
///
/// The records are shaped the way `/workers/effects` writes them, because that
/// is what §7.2 hands back: "the journaled record of effects this node instance
/// already issued", read out of the journal at redispatch.
fn dispatch_carrying(id: &str, bytes: usize) -> Reply {
    // A request large enough that a handful of records is a payload no message
    // limit would admit, and small enough that the archive of them is quick to
    // build: a `request` is the canonical text of a model call, which is where
    // the size of a real history comes from.
    let filler = "x".repeat(256 * 1024);
    let mut history: Vec<Value> = Vec::new();
    while history.len() * filler.len() < bytes {
        let ordinal = history.len();
        history.push(json!({
            "key": format!("sign/0#model/{ordinal}"),
            "site": "sign/0",
            "kind": "model",
            "ordinal": ordinal,
            "request": filler,
            "outcome": { "kind": "value", "value": "a turn this node already took" },
            "refused": false,
            "recorded_at": "2026-08-30T00:00:00.000Z",
        }));
    }
    Reply::json(
        200,
        &json!({
            "dispatch_id": id,
            "execution_id": "exec_fixture",
            "node": "flow.release.sign",
            "instance_path": "sign/0",
            "inputs": { "path": "release.dmg" },
            "effect_history": history,
        }),
    )
}

// ---------------------------------------------------------------------------
// The worker under test
// ---------------------------------------------------------------------------

/// One `agent-compose worker` process, killed when the test ends.
struct Worker {
    child: Child,
    reaped: bool,
    data_dir: PathBuf,
    said: Arc<Mutex<Vec<String>>>,
}

impl Worker {
    fn start(hub: &FixtureHub, purpose: &str) -> Self {
        Self::with_environment(hub, purpose, &[])
    }

    fn with_environment(hub: &FixtureHub, purpose: &str, extra: &[(String, String)]) -> Self {
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let data_dir = std::env::temp_dir().join(format!(
            "agent-compose-worker-protocol-{purpose}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&data_dir);
        std::fs::create_dir_all(&data_dir).expect("a data directory");
        Self::over(hub, data_dir, extra)
    }

    /// A worker over a data directory that is **already there** — one that comes
    /// back holding whatever its predecessor materialised.
    fn resuming(hub: &FixtureHub, data_dir: &std::path::Path) -> Self {
        Self::over(hub, data_dir.to_path_buf(), &[])
    }

    fn over(hub: &FixtureHub, data_dir: PathBuf, extra: &[(String, String)]) -> Self {
        let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
        command
            .arg("worker")
            .args(["--hub", &hub.base_url])
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
        let mut environment: Vec<(String, String)> =
            vec![(TOKEN_VARIABLE.to_string(), TOKEN.to_string())];
        environment.extend_from_slice(extra);
        harness::seal(&mut command, &environment);
        let mut child = command.spawn().expect("the worker starts");
        let said: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let stderr = child.stderr.take().expect("stderr is piped");
        let collecting = Arc::clone(&said);
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().map_while(Result::ok) {
                collecting
                    .lock()
                    .expect("the buffer is not poisoned")
                    .push(line);
            }
        });
        let stdout = child.stdout.take().expect("stdout is piped");
        std::thread::spawn(move || {
            let mut held = Vec::new();
            let mut stdout = stdout;
            let _ = stdout.read_to_end(&mut held);
        });
        Self {
            child,
            reaped: false,
            data_dir,
            said,
        }
    }

    fn transcript(&self) -> String {
        self.said
            .lock()
            .expect("the buffer is not poisoned")
            .join("\n")
    }

    /// Whether this worker is still running, which is what a refusal that cost
    /// only a dispatch must not change.
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
            std::thread::sleep(Duration::from_millis(20));
        }
        let said = self.transcript();
        panic!("the worker never exited; it said:\n{said}");
    }

    /// Kill this worker and its group, and leave its data directory alone.
    ///
    /// What a machine restarted between deployments looks like: the process is
    /// gone and what it materialised is not.
    fn stop(&mut self) {
        #[cfg(unix)]
        if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
            // SAFETY: a child of this process, spawned into its own group and
            // not yet reaped.
            unsafe { libc::kill(-pid, libc::SIGKILL) };
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.reaped = true;
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        if !self.reaped {
            #[cfg(unix)]
            if let Ok(pid) = libc::pid_t::try_from(self.child.id()) {
                // SAFETY: a child of this process, spawned into its own group
                // and not yet reaped.
                unsafe { libc::kill(-pid, libc::SIGKILL) };
            }
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
        let _ = std::fs::remove_dir_all(&self.data_dir);
    }
}

/// Set a hub up to take a worker all the way to polling: the provisioning join,
/// the fetch, and the join that carries the report.
fn provisioning(hub: &FixtureHub) -> String {
    let (hash, tarball) = artifact();
    hub.script("/workers/join", accepted(&hash, "wrk_provisioning"));
    hub.script("/workers/artifact", Reply::bytes(200, tarball));
    hub.script("/workers/join", accepted(&hash, "wrk_ready"));
    // Every join after the second issues a **different** session, which is what
    // lets a test tell a re-join apart from the poll that preceded it: §5 makes
    // a session a handle the hub owns, and two of them are two joins.
    hub.always("/workers/join", accepted(&hash, "wrk_renewed"));
    hub.always("/workers/poll", Reply::empty(204));
    hash
}

// ---------------------------------------------------------------------------
// §3.1, §4 — the provisioning cycle
// ---------------------------------------------------------------------------

/// A cold start omits the hash **and** the report, fetches, and joins again
/// carrying both (§3.1, §4).
///
/// The five steps of §4, read off the requests the hub was given. The first join
/// states "this worker holds no artifact" by leaving the key out — §4.1 makes
/// that an absent key rather than a `null` — and the second is the one §3.1
/// calls the ordinary steady state.
#[test]
fn a_cold_start_joins_twice_and_only_the_second_join_carries_a_report() {
    let hub = FixtureHub::start();
    let hash = provisioning(&hub);
    let worker = Worker::start(&hub, "cold-start");

    hub.until("polled", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });

    let joins = hub.asked_at("/workers/join");
    assert!(joins.len() >= 2, "{joins:#?}");
    assert_eq!(
        joins[0].method, "POST",
        "a join is a `POST` (§3): {joins:#?}"
    );
    let first = &joins[0].body;
    assert_eq!(first["protocol"], 1, "{first:#}");
    assert_eq!(
        first["compiler"],
        compose_core::codegen::COMPILER_VERSION,
        "{first:#}"
    );
    assert!(
        first["runtime"]
            .as_str()
            .is_some_and(|runtime| runtime.starts_with("bun ")),
        "the join names the runtime it executes the artifact under: {first:#}"
    );
    assert_eq!(first["claims"], json!([PLACEMENT]), "{first:#}");
    assert!(
        first.get("artifact_hash").is_none() && first.get("env_ok").is_none(),
        "a cold start omits the key altogether, and sends neither `null` nor a placeholder: \
         {first:#}"
    );

    let second = &joins[1].body;
    assert_eq!(second["artifact_hash"], json!(hash), "{second:#}");
    assert_eq!(
        second["env_ok"],
        json!([]),
        "the report is the manifest's variables this machine has, and this manifest is empty: \
         {second:#}"
    );

    // The fetch sat between them, bearer-authenticated and hash-addressed.
    let fetches = hub.asked_at("/workers/artifact");
    assert_eq!(fetches.len(), 1, "{fetches:#?}");
    assert_eq!(fetches[0].path, format!("/workers/artifact/{hash}"));
    assert_eq!(fetches[0].method, "GET", "the fetch is a `GET` (§3.5)");
    assert_eq!(
        fetches[0].authorization.as_deref(),
        Some(format!("Bearer {TOKEN}").as_str())
    );
    // …and the tree is on disk, hashed before it was written (§4 step 2).
    let tree = worker.data_dir.join("artifacts").join(&hash);
    assert!(
        tree.join("runner.ts").is_file(),
        "the worker did not materialise what it fetched; it said:\n{}",
        worker.transcript()
    );
    // §4 step 4 ran in it, rather than being skipped. These data directories sit
    // under the system temporary directory and resolve no pinned set from an
    // ancestor, so `install` takes the branch that runs the command — which is
    // the step `tests/distributed_mesh_acceptance.rs` deliberately elides by
    // rooting its workers under the installed toolchain. The artifact this
    // fixture serves declares no dependencies, so the install is offline and
    // costs milliseconds; what it checks is that the step happens at all.
    assert!(
        tree.join("node_modules").is_dir(),
        "the worker materialised the artifact and installed nothing into it; it said:\n{}",
        worker.transcript()
    );
}

/// A tarball that does not hash to the name it was asked for is not
/// materialised, and the worker says so (§3.5, §4 step 2).
#[test]
fn an_artifact_that_does_not_match_its_hash_is_refused_before_it_is_written() {
    let hub = FixtureHub::start();
    let (hash, _) = artifact();
    hub.always("/workers/join", accepted(&hash, "wrk_ready"));
    // A well-formed archive of the wrong tree, served under the right name.
    let mut blocks = tar_entry("manifest.json", b"{ \"node_runner\": \"runner.ts\" }\n");
    blocks.extend(std::iter::repeat_n(0u8, 1024));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&blocks).expect("the encoder takes it");
    hub.always(
        "/workers/artifact",
        Reply::bytes(200, encoder.finish().expect("the encoder finishes")),
    );

    let mut worker = Worker::start(&hub, "wrong-hash");
    let status = worker.waited();
    assert_eq!(status.code(), Some(2), "{}", worker.transcript());
    let said = worker.transcript();
    assert!(
        said.contains("does not match the name it is addressed by"),
        "{said}"
    );
    assert!(
        !worker.data_dir.join("artifacts").join(&hash).exists(),
        "the worker materialised a tree whose hash it could not verify"
    );
}

/// `404` at the fetch is answered by **joining again**, not by retrying it
/// (§3.5).
///
/// "A worker meeting `404` for the hash its join returned re-joins rather than
/// retrying the fetch: the join is what re-derives the current hash, and
/// re-deriving it anywhere else would be a second answer to what this deployment
/// is running."
#[test]
fn a_fetch_answered_404_sends_the_worker_back_to_the_join() {
    let hub = FixtureHub::start();
    let (hash, tarball) = artifact();
    hub.always("/workers/join", accepted(&hash, "wrk_ready"));
    hub.script(
        "/workers/artifact",
        Reply::json(
            404,
            &json!({ "hash": "sha256:0", "artifact": hash, "error": "this hub does not hold it" }),
        ),
    );
    hub.always("/workers/artifact", Reply::bytes(200, tarball));
    hub.always("/workers/poll", Reply::empty(204));

    let _worker = Worker::start(&hub, "fetch-404");
    hub.until("polled after the second fetch", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });

    let asked = hub.asked();
    let shape: Vec<&str> = asked
        .iter()
        .map(|request| route_of(&request.path))
        .take(4)
        .map(|route| match route.as_str() {
            "/workers/join" => "join",
            "/workers/artifact" => "fetch",
            "/workers/poll" => "poll",
            other => Box::leak(other.to_string().into_boxed_str()),
        })
        .collect();
    assert_eq!(
        shape,
        ["join", "fetch", "join", "fetch"],
        "a `404` is answered by a join, and the fetch that follows is the one the join named: \
         {asked:#?}"
    );
}

// ---------------------------------------------------------------------------
// §3.1 — a refused join is terminal
// ---------------------------------------------------------------------------

/// Every refusal §3.1 gives the join stops the worker, and none of them is
/// answered by joining again.
///
/// The one rule §10.1 calls load-bearing in both directions: "an implementation
/// that read them the other way round would either fail a mesh a hub restart
/// should have healed, or hammer a hub that has refused it".
#[test]
fn every_join_refusal_stops_the_worker_after_exactly_one_join() {
    for (status, body, expected) in [
        (401u16, json!(null), "401"),
        (
            409,
            json!({ "protocol": 2, "worker_protocol": 1, "error": "this hub speaks protocol 2 and the worker speaks 1" }),
            "protocol 2",
        ),
        (
            409,
            json!({ "compiler": "9.9.9", "error": "refused: this hub was built by agent-compose 9.9.9 and the worker runs 0.0.0-dev" }),
            "9.9.9",
        ),
        (
            400,
            json!({ "claim": "mac", "placements": ["gpu"], "error": "`mac` names no placement of this target: it declares `gpu`" }),
            "names no placement",
        ),
        (
            403,
            json!({ "variables": ["SIGNING_KEY"], "error": "this worker reports none of `SIGNING_KEY`" }),
            "SIGNING_KEY",
        ),
    ] {
        let hub = FixtureHub::start();
        hub.always("/workers/join", Reply::json(status, &body));
        let mut worker = Worker::start(&hub, "refused");
        let code = worker.waited();
        let said = worker.transcript();
        assert_eq!(code.code(), Some(2), "{said}");
        assert!(
            said.contains(expected),
            "the worker did not echo the refusal as it was given ({status}): {said}"
        );
        // One join, and no second: a refused join is terminal (§3.1).
        let joins = hub.asked_at("/workers/join");
        assert_eq!(
            joins.len(),
            1,
            "a worker refused {status} joined again; §3.1 makes that refusal terminal: {joins:#?}"
        );
    }
}

/// A worker holding a **stale** artifact joins once, is answered with the
/// current one, and fetches it (§3.1, §4, §5).
///
/// The redeployment case, from the worker's side. It holds an artifact and
/// cannot know the hub has replaced it, so it sends the hash and the report
/// together — which is what §3.1 requires of the case where the hash *is*
/// current. The hub ignores a report about an artifact it is not serving and
/// answers with the one it is; the worker treats that as `Stop::Redeployed`,
/// fetches, materialises, and joins again carrying the new hash.
///
/// What is asserted is the join count and the shape of each. **Two joins, and
/// neither of them answers a refusal**: §3.1 makes every refusal terminal and
/// §10.1 calls the `410`-versus-refusal decision the whole of a worker's error
/// handling, so a worker that answered a `400` with a second join would be
/// outside both. The hub's ignore-rule is what makes that possible, and this is
/// where the two halves meet.
#[test]
fn a_worker_holding_a_stale_artifact_joins_once_and_is_answered_with_the_current_one() {
    let hub = FixtureHub::start();
    let (hash, tarball) = artifact();
    // Take a worker as far as holding an artifact.
    hub.script("/workers/join", accepted(&hash, "wrk_provisioning"));
    hub.script("/workers/artifact", Reply::bytes(200, tarball));
    hub.script("/workers/join", accepted(&hash, "wrk_ready"));
    hub.always("/workers/poll", Reply::empty(204));
    let mut first = Worker::start(&hub, "stale-holder");
    hub.until("materialised and polled", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });
    let held = first.data_dir.clone();
    // The worker is stopped, but its data directory is not: a second worker over
    // the same directory is a worker that comes back holding an artifact.
    first.reaped = true;
    let _ = first.child.kill();
    let _ = first.child.wait();

    // The redeployment: a hub serving a **different** artifact.
    let hub = FixtureHub::start();
    let (current, redeployed) = redeployed_artifact();
    assert_ne!(current, hash, "the two artifacts hash differently");
    hub.always("/workers/join", accepted(&current, "wrk_after"));
    hub.always("/workers/artifact", Reply::bytes(200, redeployed));
    hub.always("/workers/poll", Reply::empty(204));

    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
    command
        .arg("worker")
        .args(["--hub", &hub.base_url])
        .args(["--claim", PLACEMENT])
        .args(["--token-env", TOKEN_VARIABLE])
        .arg("--data-dir")
        .arg(&held)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    harness::seal(
        &mut command,
        &[(TOKEN_VARIABLE.to_string(), TOKEN.to_string())],
    );
    let mut child = command.spawn().expect("the worker starts");

    hub.until("fetched the current artifact and polled", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });
    let joins = hub.asked_at("/workers/join");
    assert_eq!(
        joins.len(),
        2,
        "a worker that holds a stale artifact joins once to be told so and once to report \
         against what it fetched: {joins:#?}"
    );
    assert_eq!(
        joins[0].body["artifact_hash"],
        json!(hash),
        "the first join does not carry the hash this worker holds (§3.1, §4): {:#}",
        joins[0].body
    );
    assert!(
        joins[0].body.get("env_ok").is_some(),
        "the first join withholds the report §3.1 requires of a current hash, which a worker \
         cannot know it does not have: {:#}",
        joins[0].body
    );
    assert_eq!(
        joins[1].body["artifact_hash"],
        json!(current),
        "the second join does not report against the artifact this hub serves: {:#}",
        joins[1].body
    );
    assert!(
        joins[1].body.get("env_ok").is_some(),
        "the second join carries no report, so the hub has nothing to check (§9.2): {:#}",
        joins[1].body
    );
    let fetches = hub.asked_at("/workers/artifact");
    assert_eq!(
        fetches.len(),
        1,
        "the worker did not fetch the artifact the join named, or fetched it twice: {fetches:#?}"
    );

    #[cfg(unix)]
    if let Ok(pid) = libc::pid_t::try_from(child.id()) {
        // SAFETY: a child of this process, in a group of its own, unreaped.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
    let _ = child.kill();
    let _ = child.wait();
    let _ = std::fs::remove_dir_all(&held);
}

/// A hub rolled **back** to an artifact this worker still holds is not
/// downloaded again (§4 step 3, §3.5).
///
/// The reason a worker's trees are keyed by hash: "so the previous artifact
/// survives a rollback". Surviving is only worth the disk if the worker can
/// answer out of it — a store that kept yesterday's tree and re-downloaded it
/// anyway would have the cost of the property and not the property.
///
/// Three deployments over **one data directory**, which is what one machine
/// across two redeployments is: yesterday's artifact, today's, and yesterday's
/// again. What is asserted is the third: two joins, **no fetch**, and both trees
/// still on the disk — the one in hand and the one it replaced, which is the
/// pair §4 step 3 names.
#[test]
fn a_rollback_to_an_artifact_this_worker_still_holds_is_not_downloaded_again() {
    let (before, yesterday) = artifact();
    let (after, today) = redeployed_artifact();
    assert_ne!(before, after, "the two artifacts hash differently");

    // 1. A cold start onto yesterday's artifact.
    let first_hub = FixtureHub::start();
    first_hub.script("/workers/join", accepted(&before, "wrk_provisioning"));
    first_hub.script("/workers/artifact", Reply::bytes(200, yesterday));
    first_hub.always("/workers/join", accepted(&before, "wrk_ready"));
    first_hub.always("/workers/poll", Reply::empty(204));
    let mut worker = Worker::start(&first_hub, "rollback");
    first_hub.until("materialised yesterday's artifact and polled", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });
    worker.stop();
    let data_dir = worker.data_dir.clone();

    // 2. The redeployment: a hub serving today's, over the same data directory.
    let second_hub = FixtureHub::start();
    second_hub.always("/workers/join", accepted(&after, "wrk_after"));
    second_hub.always("/workers/artifact", Reply::bytes(200, today));
    second_hub.always("/workers/poll", Reply::empty(204));
    let mut second = Worker::resuming(&second_hub, &data_dir);
    second_hub.until("fetched today's artifact and polled", |asked| {
        asked.iter().any(|request| request.path == "/workers/poll")
    });
    assert_eq!(
        second_hub.asked_at("/workers/artifact").len(),
        1,
        "a worker that held neither of this hub's artifacts did not fetch exactly once"
    );
    second.stop();

    // 3. The rollback. This hub's artifact route answers a body no worker should
    //    ever ask it for, so a fetch here is a failing test rather than a
    //    silently slower worker.
    let third_hub = FixtureHub::start();
    third_hub.always("/workers/join", accepted(&before, "wrk_rolled_back"));
    third_hub.always(
        "/workers/artifact",
        Reply::json(
            500,
            &json!({ "error": "this artifact was already on the worker's disk" }),
        ),
    );
    third_hub.always("/workers/poll", Reply::empty(204));
    let mut third = Worker::resuming(&third_hub, &data_dir);
    third_hub.until(
        "joined under the rolled-back artifact and polled",
        |asked| asked.iter().any(|request| request.path == "/workers/poll"),
    );

    assert!(
        third_hub.asked_at("/workers/artifact").is_empty(),
        "the worker re-downloaded an artifact it had materialised two deployments ago, which is \
         what keying its trees by hash was for (§4 step 3); it said:\n{}",
        third.transcript()
    );
    let joins = third_hub.asked_at("/workers/join");
    assert_eq!(
        joins.len(),
        2,
        "a worker takes up a tree it holds through the ordinary cycle: one join to be told the \
         hash, one to report against it: {joins:#?}"
    );
    assert_eq!(
        joins[0].body["artifact_hash"],
        json!(after),
        "the first join does not carry the artifact this worker was executing out of: {:#}",
        joins[0].body
    );
    assert_eq!(
        joins[1].body["artifact_hash"],
        json!(before),
        "the second join does not report against the artifact this hub rolled back to: {:#}",
        joins[1].body
    );
    // Both trees are still here: the one in hand, and the one a roll-forward
    // would ask for next.
    assert!(data_dir.join("artifacts").join(&before).is_dir());
    assert!(data_dir.join("artifacts").join(&after).is_dir());
    assert_eq!(
        std::fs::read_to_string(data_dir.join("held"))
            .expect("the worker records which artifact it holds")
            .trim(),
        before
    );
    third.stop();
}

// ---------------------------------------------------------------------------
// §3.2, §5 — an unknown session is a join, and a bad credential is not
// ---------------------------------------------------------------------------

/// `410` at the poll is answered by joining again and polling under the session
/// that join returns (§3.2, §5).
#[test]
fn a_poll_answered_410_is_answered_by_joining_again() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script(
        "/workers/poll",
        Reply::json(410, &json!({ "error": "gone" })),
    );

    let _worker = Worker::start(&hub, "poll-410");
    hub.until("re-joined and polled again", |asked| {
        let after: Vec<&Asked> = asked
            .iter()
            .skip_while(|request| request.path != "/workers/poll")
            .collect();
        after.iter().any(|request| request.path == "/workers/join")
            && after
                .iter()
                .filter(|request| request.path == "/workers/poll")
                .count()
                >= 2
    });

    let polls = hub.asked_at("/workers/poll");
    assert!(
        polls[0].session.is_some(),
        "every route after the join carries the session (§3): {polls:#?}"
    );
    let renewed = polls
        .iter()
        .find(|poll| poll.session != polls[0].session)
        .unwrap_or_else(|| panic!("the worker polled again under a new session: {polls:#?}"));
    assert_eq!(renewed.session.as_deref(), Some("wrk_renewed"));
}

/// `401` at the poll is **terminal**, and is not answered by joining again
/// (§3.2).
///
/// "The credential is the one the join used, so re-joining cannot improve it."
#[test]
fn a_poll_answered_401_stops_the_worker() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.always("/workers/poll", Reply::empty(401));

    let mut worker = Worker::start(&hub, "poll-401");
    let status = worker.waited();
    assert_eq!(status.code(), Some(2), "{}", worker.transcript());
    assert!(
        worker.transcript().contains("401"),
        "{}",
        worker.transcript()
    );
    // Two joins — the provisioning one and the report — and no third.
    assert_eq!(hub.asked_at("/workers/join").len(), 2, "{:#?}", hub.asked());
}

// ---------------------------------------------------------------------------
// §3.3, §3.4 — a dispatch, its effects, and its result
// ---------------------------------------------------------------------------

/// A dispatch is executed, its effects are streamed home as they happen, and the
/// result settles it (§3.2, §3.3, §3.4).
///
/// The runner is this file's own, so what is asserted is the **worker's** half:
/// what it spawned it with, that the effect went out before the result, and that
/// what it posted is the record and the outcome the runner wrote.
#[test]
fn a_dispatch_is_run_and_its_effects_go_home_before_its_result() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script("/workers/poll", dispatch("dsp_one"));
    hub.always("/workers/effects", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::start(&hub, "dispatch");
    hub.until("settled the dispatch", |asked| {
        asked
            .iter()
            .any(|request| request.path == "/workers/result")
    });

    let asked = hub.asked();
    let effect_at = asked
        .iter()
        .position(|request| request.path == "/workers/effects")
        .unwrap_or_else(|| {
            panic!(
                "the worker sent no effect batch: {asked:#?}\n{}",
                worker.transcript()
            )
        });
    let result_at = asked
        .iter()
        .position(|request| request.path == "/workers/result")
        .expect("the worker settled the dispatch");
    assert!(
        effect_at < result_at,
        "an effect reached the hub after the result it belongs to (§3.3): {asked:#?}"
    );

    let batch = &asked[effect_at].body;
    assert_eq!(batch["dispatch_id"], "dsp_one", "{batch:#}");
    assert_eq!(batch["effects"][0]["key"], "sign/0#model/0", "{batch:#}");
    assert_eq!(
        batch["effects"][0]["outcome"]["value"], "an answer the fixture runner made up",
        "{batch:#}"
    );

    let settled = &asked[result_at].body;
    assert_eq!(settled["dispatch_id"], "dsp_one", "{settled:#}");
    assert_eq!(
        settled["output"]["signature"], "from the fixture runner",
        "{settled:#}"
    );
}

/// A dispatch whose `effect_history` is larger than any message-sized ceiling
/// is read, run and settled (§3.2, §7.2).
///
/// The poll answer is the one body on this wire that **grows with the
/// execution**: §7.2 hands a redispatched node every effect the journal holds at
/// its instance path, and a placed `agent:` with a long tool loop — or one
/// already through two `retry:` attempts — carries the canonical request and the
/// whole of the outcome for each of them.
///
/// A worker that capped its reading at a message's size would lose such a
/// dispatch in the worst available way. The hub has already claimed it to this
/// session, so the answer is not repeated: every later poll is `204` while the
/// session holds an unsettled dispatch (§2), the session goes on polling so
/// nothing supersedes it (§6.3), and the node waits out its whole `timeout:`
/// chain — failing over a payload the worker declined to read, with a diagnostic
/// about a deadline.
#[test]
fn a_dispatch_whose_history_is_larger_than_a_message_is_read_and_settled() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script(
        "/workers/poll",
        dispatch_carrying("dsp_replayed", 12 * 1024 * 1024),
    );
    hub.always("/workers/effects", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::start(&hub, "large-history");
    let asked = hub.until("settled the dispatch", |asked| {
        asked
            .iter()
            .any(|request| request.path == "/workers/result")
    });
    let settled = asked
        .iter()
        .find(|request| request.path == "/workers/result")
        .unwrap_or_else(|| {
            panic!(
                "the worker settled nothing: {asked:#?}\n{}",
                worker.transcript()
            )
        });
    assert_eq!(
        settled.body["dispatch_id"], "dsp_replayed",
        "{:#}",
        settled.body
    );
    assert_eq!(
        settled.body["output"]["signature"], "from the fixture runner",
        "{:#}",
        settled.body
    );
}

/// `409` at `/workers/result` is discarded, and the worker keeps its session
/// and goes on polling (§3.4).
///
/// "`409` says *the hub knows you and does not want this*, and the result is
/// dead. A worker whose re-posted result meets `409` has its answer and stops
/// re-posting."
#[test]
fn a_result_answered_409_is_discarded_and_the_session_is_kept() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script("/workers/poll", dispatch("dsp_superseded"));
    hub.always("/workers/effects", Reply::empty(204));
    hub.always(
        "/workers/result",
        Reply::json(
            409,
            &json!({ "dispatch_id": "dsp_superseded", "error": "superseded" }),
        ),
    );

    let worker = Worker::start(&hub, "result-409");
    hub.until("polled again after the `409`", |asked| {
        let at = asked
            .iter()
            .position(|request| request.path == "/workers/result");
        at.is_some_and(|at| {
            asked[at..]
                .iter()
                .any(|request| request.path == "/workers/poll")
        })
    });

    // One result, and no second: the worker took the `409` as its answer.
    std::thread::sleep(Duration::from_millis(300));
    let results = hub.asked_at("/workers/result");
    assert_eq!(
        results.len(),
        1,
        "the worker re-posted a result the hub had already refused: {results:#?}\n{}",
        worker.transcript()
    );
    // …and it kept the session it had, rather than joining again.
    assert_eq!(hub.asked_at("/workers/join").len(), 2, "{:#?}", hub.asked());
}

/// `410` at `/workers/result` is the opposite: the result is **still owed**, so
/// the worker joins and posts it again (§3.4).
#[test]
fn a_result_answered_410_is_posted_again_under_the_session_the_join_returns() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script("/workers/poll", dispatch("dsp_owed"));
    hub.always("/workers/effects", Reply::empty(204));
    hub.script(
        "/workers/result",
        Reply::json(410, &json!({ "error": "gone" })),
    );
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::start(&hub, "result-410");
    hub.until("posted the result again", |asked| {
        asked
            .iter()
            .filter(|request| request.path == "/workers/result")
            .count()
            >= 2
    });

    let results = hub.asked_at("/workers/result");
    assert_eq!(
        results[0].body["dispatch_id"],
        results[1].body["dispatch_id"],
        "the hub attributes a result by `dispatch_id`, which the new session does not change: \
         {results:#?}\n{}",
        worker.transcript()
    );
    assert_ne!(
        results[0].session, results[1].session,
        "the re-post went under the session the join returned: {results:#?}"
    );
}

/// `410` at `/workers/effects` is the same rule one route over: the batch is
/// **re-sent**, never dropped (§3.3).
///
/// "A worker that discarded the batch instead would hand the redispatch of §7.2
/// an `effect_history` short of the frontier, and the node would re-issue an
/// effect the journal was owed."
#[test]
fn an_effect_batch_answered_410_is_sent_again_and_never_dropped() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script("/workers/poll", dispatch("dsp_effects"));
    hub.script(
        "/workers/effects",
        Reply::json(410, &json!({ "error": "gone" })),
    );
    hub.always("/workers/effects", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::start(&hub, "effects-410");
    hub.until("settled the dispatch", |asked| {
        asked
            .iter()
            .any(|request| request.path == "/workers/result")
    });

    let batches = hub.asked_at("/workers/effects");
    assert_eq!(
        batches.len(),
        2,
        "the batch was not re-sent after the `410`: {batches:#?}\n{}",
        worker.transcript()
    );
    assert_eq!(
        batches[0].body, batches[1].body,
        "the same batch, under a new session: {batches:#?}"
    );
    assert_ne!(batches[0].session, batches[1].session, "{batches:#?}");
}

/// A `4xx` §3.3 does **not** give the route costs this dispatch, and never the
/// worker (§3.1, §3.3, §10.1).
///
/// Every status those two routes list is acted on by name; what is left is a
/// `4xx` from **underneath** the handler — a body limit smaller than the record,
/// an intermediary's own refusal — and the temptation is to read it the way §3.1
/// reads a refused join. §3.1's terminality is about the join, where "a second
/// join would be refused identically" is a statement about this worker's right
/// to be in this mesh at all; a body one hub would not take says nothing of the
/// kind. A worker that ended on it would leave the placement with no worker, and
/// the replacement would reach the same record and end the same way — so one
/// oversized effect would be a node the mesh never runs again.
///
/// So the attempt fails, named, and the process goes on: `413` here, which is
/// exactly what a Fastify route left at its default limit answers, and the two
/// halves asserted are the failure the hub is handed for the dispatch it was
/// owed a result for, and the **next** dispatch running on the same process.
#[test]
fn an_effect_batch_refused_outside_its_table_fails_the_dispatch_and_keeps_the_worker() {
    let hub = FixtureHub::start();
    provisioning(&hub);
    hub.script("/workers/poll", dispatch("dsp_refused"));
    hub.script("/workers/poll", dispatch("dsp_after"));
    hub.script(
        "/workers/effects",
        Reply::json(
            413,
            &json!({ "statusCode": 413, "code": "FST_ERR_CTP_BODY_TOO_LARGE" }),
        ),
    );
    hub.always("/workers/effects", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let mut worker = Worker::start(&hub, "effects-413");
    hub.until("settled both dispatches", |asked| {
        asked
            .iter()
            .filter(|request| request.path == "/workers/result")
            .count()
            >= 2
    });

    let results = hub.asked_at("/workers/result");
    let refused = &results[0].body;
    assert_eq!(refused["dispatch_id"], "dsp_refused", "{refused:#}");
    assert_eq!(
        refused["error"]["name"],
        "WorkerEffectsRefused",
        "the refused batch did not fail its dispatch by name: {refused:#}\n{}",
        worker.transcript()
    );
    assert!(
        refused["error"]["message"]
            .as_str()
            .unwrap_or_default()
            .contains("413"),
        "the failure does not name the status the hub gave: {refused:#}"
    );

    let after = &results[1].body;
    assert_eq!(
        after["dispatch_id"],
        "dsp_after",
        "the placement's next dispatch was not run: {after:#}\n{}",
        worker.transcript()
    );
    assert_eq!(
        after["output"]["signature"], "from the fixture runner",
        "{after:#}"
    );
    assert!(
        worker.running(),
        "one refused effect batch ended the worker, so the placement has none: {}",
        worker.transcript()
    );
}

/// A runner that dies without answering is an attempt that **failed**, and the
/// hub is told so rather than left holding an unsettled dispatch (§3.4, §6.3).
#[test]
fn a_node_runner_that_says_nothing_settles_its_dispatch_as_a_failure() {
    let hub = FixtureHub::start();
    // An artifact whose runner exits without writing a result line.
    let files: Vec<(&str, &[u8])> = vec![
        (
            "manifest.json",
            br#"{ "node_runner": "runner.ts", "placements": [{ "name": "mac", "environment": [] }] }
"#,
        ),
        ("package.json", b"{ \"name\": \"fixture\", \"private\": true }\n"),
        ("runner.ts", b"process.exit(9);\n"),
    ];
    let hash = compose_core::codegen::artifact::hash_of(files.iter().copied());
    let mut blocks = Vec::new();
    for (path, bytes) in &files {
        blocks.extend(tar_entry(path, bytes));
    }
    blocks.extend(std::iter::repeat_n(0u8, 1024));
    let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    encoder.write_all(&blocks).expect("the encoder takes it");
    hub.always("/workers/join", accepted(&hash, "wrk_ready"));
    hub.always(
        "/workers/artifact",
        Reply::bytes(200, encoder.finish().expect("the encoder finishes")),
    );
    hub.script("/workers/poll", dispatch("dsp_silent"));
    hub.always("/workers/poll", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::start(&hub, "silent-runner");
    hub.until("settled the dispatch it could not run", |asked| {
        asked
            .iter()
            .any(|request| request.path == "/workers/result")
    });
    let settled = &hub.asked_at("/workers/result")[0].body;
    assert_eq!(settled["dispatch_id"], "dsp_silent", "{settled:#}");
    assert!(
        settled["error"]["message"]
            .as_str()
            .is_some_and(|said| said.contains("9")),
        "the failure does not name what the runner did: {settled:#}\n{}",
        worker.transcript()
    );
}

/// A dispatch answered while a node is running is **queued**, and the poll goes
/// on (§2, §3.2, §3.4).
///
/// §2 requires the poll to continue while a node runs — "the poll is the
/// heartbeat" — and a session that stops making requests is declared gone at the
/// liveness window, which supersedes whatever it was holding (§6.3). So the one
/// thing a worker must not do with a second dispatch is **wait** for the first.
///
/// A hub hands one over legitimately: §3.4 makes a dispatch the hub superseded
/// on the node's own `timeout:` no longer *unsettled*, so answering the next poll
/// is not a breach of §2's "MUST NOT answer a session's poll with a dispatch
/// while that session has a dispatch it has not settled" — and a worker cannot
/// tell that apart from a hub that did breach it. Both are a real journaled
/// dispatch it is owed a result for, so both are queued and neither is dropped.
///
/// Two halves are asserted, and a blocking recovery fails both: the results
/// arrive, in order, for **both** dispatches; and polls keep arriving while the
/// first node runs. The fixture runner lingers so that "while the first node
/// runs" is a window a test can count requests in.
#[test]
fn a_second_dispatch_is_queued_and_the_poll_never_stops() {
    let hub = FixtureHub::start();
    let (hash, tarball) = artifact();
    hub.always("/workers/join", accepted(&hash, "wrk_ready"));
    hub.always("/workers/artifact", Reply::bytes(200, tarball));
    hub.script("/workers/poll", dispatch("dsp_first"));
    hub.script("/workers/poll", dispatch("dsp_second"));
    hub.always("/workers/poll", Reply::empty(204));
    hub.always("/workers/effects", Reply::empty(204));
    hub.always("/workers/result", Reply::empty(204));

    let worker = Worker::with_environment(
        &hub,
        "queued-dispatch",
        &[("FIXTURE_RUNNER_LINGER_MS".to_string(), "2000".to_string())],
    );

    let asked = hub.until("settled both dispatches", |asked| {
        asked
            .iter()
            .filter(|request| request.path == "/workers/result")
            .count()
            >= 2
    });
    let settled: Vec<&str> = asked
        .iter()
        .filter(|request| request.path == "/workers/result")
        .filter_map(|request| request.body["dispatch_id"].as_str())
        .collect();
    assert_eq!(
        settled,
        ["dsp_first", "dsp_second"],
        "the queued dispatch was dropped, or the two ran out of order:\n{}",
        worker.transcript()
    );

    // The window: from the poll that was answered `dsp_second` to the result
    // that settled `dsp_first`. A worker that waited for the node in hand makes
    // no request at all in it, which at §2's window is a session the hub
    // declares gone — and the second dispatch superseded with it.
    let took_second = asked
        .iter()
        .enumerate()
        .filter(|(_, request)| request.path == "/workers/poll")
        .nth(1)
        .map(|(index, _)| index)
        .expect("the worker polled twice");
    let settled_first = asked
        .iter()
        .position(|request| {
            request.path == "/workers/result" && request.body["dispatch_id"] == "dsp_first"
        })
        .expect("the first dispatch was settled");
    let polls = asked[took_second + 1..settled_first]
        .iter()
        .filter(|request| request.path == "/workers/poll")
        .count();
    assert!(
        polls >= 2,
        "the worker made {polls} polls between taking the second dispatch and settling the \
         first: the poll is the heartbeat, and a thread blocked on a slow runner stops it \
         (docs/distributed.md §2, §3.2):\n{asked:#?}\n{}",
        worker.transcript()
    );
}
