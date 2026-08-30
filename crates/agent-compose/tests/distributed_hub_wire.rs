//! The hub, spoken to over the wire (`docs/distributed.md` §3–§7).
//!
//! Every test here drives a **real served app** — `agent-compose serve` over the
//! `placed-nodes` fixture, resolved for its `mesh` target — and speaks the five
//! routes of §3 to it with an ordinary HTTP client. Nothing is stubbed on the hub
//! side: the graph really runs, a placed node really parks, the journal really
//! records the dispatch, and what a worker would send is what these tests send.
//!
//! # Why there is no worker binary in here
//!
//! Because the hub's behaviour is not a property of any particular worker, and
//! writing one into the harness would make it one. §3 fixes a wire; a test that
//! sends exactly what the wire says and asserts exactly what the document says
//! comes back is testing the contract, where a test driving `agent-compose
//! worker` against it would be testing two implementations agreeing with each
//! other. The multi-process suite that drives the real binary is the worker
//! pass's, and it is the *acceptance* gate; this is the conformance one.
//!
//! # The timings
//!
//! §2's poll hold and liveness window are 25s and 90s, which no test can wait
//! out. Both are configurable — §2 requires only that the hold stays shorter
//! than the window — so every hub here is started with a short pair, and
//! [`MESH_TIMINGS`] is the one place they are written.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use mock_provider::{Client, MockProvider, Outcome, Request, Response, Script};
use serde_json::{Value, json};

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

/// The credential every worker in this file presents.
const TOKEN: &str = "a-join-token-nobody-else-has";

/// The variable the fixture's `hub.join_token:` names.
const TOKEN_VARIABLE: &str = "MESH_JOIN_TOKEN";

/// The fixture, and the target that gives it a mesh.
const FIXTURE: &str = "placed-nodes";
const TARGET: &str = "mesh";

/// The model every agent in the fixture routes to.
const SONNET: &str = "claude-sonnet-4-6";

/// A hold and a window short enough for a test and still in §2's relationship.
///
/// The window is five holds wide, which is the shape §2 asks for ("a hold
/// shorter than most intermediary idle timeouts and a liveness window several
/// holds wide") at a scale a test can spend. The sweep runs at a quarter of the
/// window, so a session that stops polling is declared gone inside two seconds.
const MESH_TIMINGS: &[(&str, &str)] = &[
    ("AGENT_COMPOSE_MESH_POLL_HOLD_MS", "300"),
    ("AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS", "1500"),
];

/// How long a test waits for something the hub does on its own.
const PATIENCE: Duration = Duration::from_secs(20);

// ---------------------------------------------------------------------------
// The harness
// ---------------------------------------------------------------------------

/// One served hub, and everything a test needs to speak to it.
struct Hub {
    /// Held so the app outlives the test that started it.
    _served: harness::Served,
    /// Held so the fixture's `${MOCK_BASE_URL}` resolves to something, and
    /// scripted by the one test whose flow runs an agent on the hub.
    provider: MockProvider,
    /// Held so the built project — and its journal — outlive the app.
    _scratch: Option<harness::Scratch>,
    client: Client,
    /// Where it is listening, for a test that needs a client of its own.
    base_url: String,
}

impl Hub {
    /// Send one request, failing the test on a transport error.
    fn send(&self, request: Request) -> Response {
        self.client
            .send(request)
            .unwrap_or_else(|error| panic!("the hub answered nothing: {error}"))
    }

    /// A request already carrying the join token.
    fn authorized(method: &str, path: &str) -> Request {
        Request::new(method, path).header("authorization", format!("Bearer {TOKEN}"))
    }

    /// `POST /workers/join` with a body of this shape.
    fn join(&self, body: &Value) -> Response {
        self.send(Self::authorized("POST", "/workers/join").json(body))
    }

    /// A join that is well formed in every field but the ones `body` overrides.
    fn joining(&self, overrides: &[(&str, Value)]) -> Response {
        let mut body = json!({
            "protocol": 1,
            "compiler": compiler_version(),
            "runtime": "bun 1.2.3",
            "claims": ["mac"],
        });
        for (key, value) in overrides {
            body[*key] = value.clone();
        }
        self.join(&body)
    }

    /// A worker that has fetched the artifact and reported its manifest: the
    /// join §3.1 calls the ordinary steady state, one round trip in.
    fn worker(&self) -> Worker {
        let cold = self.joining(&[]);
        assert_eq!(cold.status, 200, "{}", body_of(&cold));
        let artifact = cold.json()["artifact"]["hash"]
            .as_str()
            .expect("the answer names the artifact")
            .to_string();
        let ready = self.joining(&[
            ("artifact_hash", json!(artifact)),
            ("env_ok", json!(manifest_of("mac"))),
        ]);
        assert_eq!(ready.status, 200, "{}", body_of(&ready));
        Worker {
            session: ready.json()["worker_session"]
                .as_str()
                .expect("the answer issues a session")
                .to_string(),
        }
    }

    /// Start one execution through an `http` trigger, and answer its id.
    fn start(&self, path: &str, body: &Value) -> String {
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
}

/// One worker session, as a test holds it.
struct Worker {
    session: String,
}

impl Worker {
    /// A request carrying both credentials §3 requires after the join.
    fn request(&self, method: &str, path: &str) -> Request {
        Hub::authorized(method, path).header("x-worker-session", &self.session)
    }

    /// `GET /workers/poll`, once.
    fn poll(&self, hub: &Hub) -> Response {
        hub.send(self.request("GET", "/workers/poll"))
    }

    /// Poll until the hub hands over a dispatch, or the patience runs out.
    fn dispatch(&self, hub: &Hub) -> Value {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            let answered = self.poll(hub);
            if answered.status == 200 {
                return answered.json();
            }
            assert_eq!(
                answered.status,
                204,
                "a poll answered something other than a dispatch or an empty hold: {}",
                body_of(&answered)
            );
        }
        panic!("no dispatch reached this worker inside {PATIENCE:?}");
    }

    /// `POST /workers/result`, settling one dispatch with an output.
    fn settle(&self, hub: &Hub, dispatch: &str, output: &Value) -> Response {
        hub.send(
            self.request("POST", "/workers/result")
                .json(&json!({ "dispatch_id": dispatch, "output": output })),
        )
    }

    /// `POST /workers/effects`, handing one journaled effect home.
    fn effects(&self, hub: &Hub, dispatch: &str, effects: &Value) -> Response {
        hub.send(
            self.request("POST", "/workers/effects")
                .json(&json!({ "dispatch_id": dispatch, "effects": effects })),
        )
    }
}

/// The compiler release this build is, which every join has to match (§4.1).
fn compiler_version() -> String {
    compose_core::codegen::COMPILER_VERSION.to_string()
}

/// The variables one placement's manifest names, computed the way the artifact
/// the hub served carries them — which is where §9.1 puts them.
fn manifest_of(placement: &str) -> Vec<String> {
    let ir = artifact();
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

/// The fixture's artifact, resolved for its mesh target.
fn artifact() -> compose_core::Ir {
    let entrypoint = harness::fixture(FIXTURE);
    let resolution = compose_core::resolve_with_target(&entrypoint, TARGET);
    resolution.ir.expect("the fixture resolves")
}

/// A response body, for a failure message.
fn body_of(response: &Response) -> String {
    format!(
        "{} {}",
        response.status,
        String::from_utf8_lossy(&response.body)
    )
}

/// The environment every hub in this file is started with.
fn environment(provider: &MockProvider) -> Vec<(String, String)> {
    let mut held = harness::environment(provider);
    held.push((TOKEN_VARIABLE.to_string(), TOKEN.to_string()));
    for (name, value) in MESH_TIMINGS {
        held.push(((*name).to_string(), (*value).to_string()));
    }
    held
}

/// Serve the fixture under its mesh target, into a scratch directory.
///
/// `None` when the toolchain is not installed, which is the shape every other
/// suite here takes: a machine without Bun skips rather than fails.
fn hub() -> Option<Hub> {
    let scratch = harness::scratch_project("mesh-wire")?;
    let hub = hub_into(&scratch, &[])?;
    Some(Hub {
        _scratch: Some(harness::Scratch::at(scratch)),
        ..hub
    })
}

/// The same, into a directory the caller owns — which a restart needs, because
/// the journal is the directory's (`docs/durability.md` §6.1).
fn hub_into(out: &Path, extra: &[(String, String)]) -> Option<Hub> {
    let provider = MockProvider::start().expect("a loopback port");
    let mut held = environment(&provider);
    held.extend_from_slice(extra);
    let served = harness::serve_target_into(out, &harness::fixture(FIXTURE), TARGET, &held)?;
    let client = Client::new(&served.base_url)
        .expect("the hub's address parses")
        .with_timeout(Duration::from_secs(30));
    let base_url = served.base_url.clone();
    Some(Hub {
        _served: served,
        provider,
        _scratch: None,
        client,
        base_url,
    })
}

/// A scratch directory a restart can serve twice.
fn shared_project(purpose: &str) -> Option<PathBuf> {
    harness::scratch_project(purpose)
}

// ---------------------------------------------------------------------------
// §3.1 — the join, and every refusal it has
// ---------------------------------------------------------------------------

/// The refusal matrix of §3.1, every row, at the status the document gives it.
///
/// Each row is a condition another join would meet identically, which is why
/// §3.1 makes a refused join **terminal**: what is asserted here is the status
/// and what the body may say, because a worker's whole error handling is
/// deciding between "join again" (`410`, and nowhere else) and "stop".
#[test]
fn every_join_refusal_is_the_status_and_the_shape_the_document_gives_it() {
    let Some(hub) = hub() else {
        return;
    };

    // A refused credential is told nothing about why (§3.1's first row).
    let refused = hub.send(
        Request::post("/workers/join")
            .header("authorization", "Bearer not-the-token")
            .json(&json!({ "protocol": 1, "compiler": compiler_version(), "runtime": "bun 1.2", "claims": ["mac"] })),
    );
    assert_eq!(refused.status, 401, "{}", body_of(&refused));
    assert!(
        refused.body.is_empty(),
        "a `401` carried a body: {}",
        body_of(&refused)
    );

    // …and a request with no credential at all is the same refusal.
    let anonymous = hub.send(Request::post("/workers/join").json(&json!({ "protocol": 1 })));
    assert_eq!(anonymous.status, 401);

    // The wire, before anything about the deployment (§10).
    let ahead = hub.joining(&[("protocol", json!(99))]);
    assert_eq!(ahead.status, 409, "{}", body_of(&ahead));
    let said = ahead.json();
    assert_eq!(said["protocol"], json!(1), "{}", body_of(&ahead));
    assert_eq!(said["worker_protocol"], json!(99), "{}", body_of(&ahead));

    // …and a version this hub cannot **compare** is the same refusal, without
    // naming an end that is behind. §3.1 asks the body for "both versions, and
    // which end is behind"; a `protocol` that is absent or is not a number
    // answers the first and not the second, and reporting the hub as the old one
    // over a field a client spelled wrong sends an operator to the wrong machine.
    for spelled in [json!("1"), json!(null)] {
        let unspeakable = hub.joining(&[("protocol", spelled.clone())]);
        assert_eq!(
            unspeakable.status,
            409,
            "`protocol` {spelled}: {}",
            body_of(&unspeakable)
        );
        assert!(
            !body_of(&unspeakable).contains("this hub is behind"),
            "`protocol` {spelled} was answered as this hub being behind: {}",
            body_of(&unspeakable)
        );
    }

    // The handshake triple's two required members, each naming both sides (§4.1).
    let stale = hub.joining(&[("compiler", json!("0.3.9"))]);
    assert_eq!(stale.status, 409, "{}", body_of(&stale));
    let said = body_of(&stale);
    assert!(
        said.contains("0.3.9") && said.contains(&compiler_version()),
        "the refusal names one side only: {said}"
    );

    let wrong_runtime = hub.joining(&[("runtime", json!("node v22.3.0"))]);
    assert_eq!(wrong_runtime.status, 409, "{}", body_of(&wrong_runtime));
    let said = body_of(&wrong_runtime);
    assert!(
        said.contains("node v22.3.0") && said.to_lowercase().contains("bun"),
        "{said}"
    );
    // A worker on a *newer* Bun of the same major is not a mismatch: §4.1
    // compares the name and the major and nothing finer.
    let newer = hub.joining(&[("runtime", json!("bun 1.9.0"))]);
    assert_eq!(newer.status, 200, "{}", body_of(&newer));

    // A claim that names no placement: `400`, naming the claim and the target's
    // own placements — the first refusal that describes the deployment.
    let unknown = hub.joining(&[("claims", json!(["mac", "gpu"]))]);
    assert_eq!(unknown.status, 400, "{}", body_of(&unknown));
    let said = unknown.json();
    assert_eq!(said["claim"], json!("gpu"));
    assert_eq!(said["placements"], json!(["mac"]));

    // …and a join that does not name what it claims at all is refused on the
    // same row. `claims` is REQUIRED of every join, cold start included (§3.1's
    // field list), and reading an absent or mis-typed value as "claims nothing"
    // would answer a *dispatchable* session no work can reach: every hold it
    // takes is answered `204`, the placement's work parks behind a worker both
    // ends believe is healthy, and nothing anywhere says why.
    let unnamed = hub.join(&json!({
        "protocol": 1,
        "compiler": compiler_version(),
        "runtime": "bun 1.2.3",
    }));
    assert_eq!(unnamed.status, 400, "{}", body_of(&unnamed));
    assert_eq!(unnamed.json()["placements"], json!(["mac"]));
    for spelled in [json!("mac"), json!([]), json!(["mac", 7])] {
        let mistyped = hub.joining(&[("claims", spelled.clone())]);
        assert_eq!(
            mistyped.status,
            400,
            "`claims` {spelled}: {}",
            body_of(&mistyped)
        );
    }

    // The manifest, checked against the artifact the worker says it holds, and
    // naming **variables, never values** (§9).
    let hash = hub.joining(&[]).json()["artifact"]["hash"]
        .as_str()
        .expect("the answer names the artifact")
        .to_string();
    let short: Vec<String> = manifest_of("mac")
        .into_iter()
        .filter(|name| name != "KEYCHAIN_PASSWORD")
        .collect();
    let unmet = hub.joining(&[("artifact_hash", json!(hash)), ("env_ok", json!(short))]);
    assert_eq!(unmet.status, 403, "{}", body_of(&unmet));
    assert_eq!(unmet.json()["variables"], json!(["KEYCHAIN_PASSWORD"]));
    assert!(
        !body_of(&unmet).contains(TOKEN),
        "a refusal echoed a credential"
    );

    // `env_ok` absent on a join whose hash **is** the current one: the `400`
    // §3.1 gives that pair, because there is a manifest this worker could have
    // read and did not (§9.2).
    let silent = hub.joining(&[("artifact_hash", json!(hash))]);
    assert_eq!(silent.status, 400, "{}", body_of(&silent));
    assert!(body_of(&silent).contains("env_ok"), "{}", body_of(&silent));

    // …and the other direction is **not** a refusal. A report sent with a stale
    // hash is a report about a manifest this hub is not serving, so §3.1 has the
    // hub ignore it and answer with the current artifact — which is what keeps
    // "a refused join is terminal" satisfiable for a worker that has just been
    // redeployed under: it cannot know its hash is stale until it has asked, and
    // a refusal here would leave it with no body a second join could send.
    let stale = hub.joining(&[
        ("artifact_hash", json!(format!("sha256:{}", "0".repeat(64)))),
        ("env_ok", json!(["KEYCHAIN_PASSWORD"])),
    ]);
    assert_eq!(stale.status, 200, "{}", body_of(&stale));
    assert_eq!(
        stale.json()["artifact"]["hash"],
        json!(hash),
        "the answer does not name the artifact this hub serves: {}",
        body_of(&stale)
    );
}

/// The **order** of §3.1's rows, which is as normative as the rows.
///
/// "A worker that cannot be authenticated is told nothing, a worker whose wire
/// this hub does not speak is told that before anything about the deployment,
/// and the placement and manifest answers — which describe the target — are given
/// only to a worker that has got that far." Each join below is wrong in two ways
/// at once, and what it is told is the earlier one.
#[test]
fn a_join_wrong_in_two_ways_is_refused_by_the_earlier_of_them() {
    let Some(hub) = hub() else {
        return;
    };
    let hash = hub.joining(&[]).json()["artifact"]["hash"]
        .as_str()
        .expect("the answer names the artifact")
        .to_string();

    // Bad credential **and** an unspeakable protocol: `401`, and nothing about
    // the protocol — a refused credential learns nothing about the deployment.
    let both = hub.send(
        Request::post("/workers/join")
            .header("authorization", "Bearer not-the-token")
            .json(&json!({ "protocol": 99, "compiler": "0.0.1", "runtime": "node v1", "claims": ["nowhere"] })),
    );
    assert_eq!(both.status, 401);
    assert!(both.body.is_empty());

    // Protocol **and** compiler: the wire comes first (§3.1, §10).
    let wire_first = hub.joining(&[("protocol", json!(99)), ("compiler", json!("0.3.9"))]);
    assert_eq!(wire_first.status, 409);
    assert!(
        !body_of(&wire_first).contains("0.3.9"),
        "a protocol refusal answered about the release too: {}",
        body_of(&wire_first)
    );

    // Compiler **and** an unknown claim: the triple comes before the target.
    let triple_first = hub.joining(&[("compiler", json!("0.3.9")), ("claims", json!(["nowhere"]))]);
    assert_eq!(triple_first.status, 409);
    assert!(!body_of(&triple_first).contains("nowhere"));

    // An unknown claim **and** an unmet manifest: the claim comes first, so a
    // worker is never told which variables a placement it cannot claim needs.
    let claim_first = hub.joining(&[
        ("claims", json!(["nowhere"])),
        ("artifact_hash", json!(hash)),
        ("env_ok", json!(Vec::<String>::new())),
    ]);
    assert_eq!(claim_first.status, 400);
    assert!(
        !body_of(&claim_first).contains("KEYCHAIN_PASSWORD"),
        "a refusal about a claim named a manifest: {}",
        body_of(&claim_first)
    );
}

/// A provisioning join is answered normally and dispatched nothing (§3.1).
///
/// "The check therefore still catches the machine it exists for … one round trip
/// later and still before any node runs" — which only works if the hub really
/// withholds work from the session in between, since a worker that was dispatched
/// on its first join would run a node before its manifest was ever checked.
#[test]
fn a_provisioning_join_is_answered_and_never_dispatched_to() {
    let Some(hub) = hub() else {
        return;
    };
    let cold = hub.joining(&[]);
    assert_eq!(cold.status, 200, "{}", body_of(&cold));
    let answer = cold.json();
    let hash = answer["artifact"]["hash"]
        .as_str()
        .expect("the answer names the artifact")
        .to_string();
    assert_eq!(answer["protocol"], json!(1));
    assert_eq!(answer["compiler"], json!(compiler_version()));
    assert_eq!(answer["poll_url"], json!("/workers/poll"));
    assert_eq!(
        answer["artifact"]["url"],
        json!(format!("/workers/artifact/{hash}"))
    );
    let provisioning = Worker {
        session: answer["worker_session"]
            .as_str()
            .expect("a session")
            .to_string(),
    };

    // Work on the board, and this session still gets nothing.
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));
    hub.until(&execution, "parked on its placement", |report| {
        report["placement_waits"]
            .as_array()
            .is_some_and(|waits| !waits.is_empty())
    });
    for _ in 0..3 {
        let answered = provisioning.poll(&hub);
        assert_eq!(
            answered.status,
            204,
            "a provisioning session was dispatched to: {}",
            body_of(&answered)
        );
    }

    // …and the same worker, having fetched and reported, is dispatched at once.
    let ready = hub.worker();
    let dispatch = ready.dispatch(&hub);
    assert_eq!(dispatch["execution_id"], json!(execution));
    assert_eq!(dispatch["node"], json!("flow.release.sign"));
}

// ---------------------------------------------------------------------------
// §3.5 — the artifact
// ---------------------------------------------------------------------------

/// The fetch route: hash-addressed, bearer-only, and the two refusals §3.5 gives.
#[test]
fn the_artifact_route_is_hash_addressed_and_takes_the_bearer_alone() {
    let Some(hub) = hub() else {
        return;
    };
    let hash = hub.joining(&[]).json()["artifact"]["hash"]
        .as_str()
        .expect("the answer names the artifact")
        .to_string();
    assert!(
        hash.starts_with("sha256:") && hash.len() == "sha256:".len() + 64,
        "the artifact hash is not `sha256:` and 64 hex digits: {hash}"
    );

    // **No session**, which is the exception §3.5 states and states as one.
    let served = hub.send(Hub::authorized("GET", &format!("/workers/artifact/{hash}")));
    assert_eq!(served.status, 200, "{}", body_of(&served));
    assert_eq!(
        served.headers.get("content-type").map(String::as_str),
        Some("application/gzip")
    );
    assert_eq!(
        served.headers.get("content-length").map(String::as_str),
        Some(served.body.len().to_string().as_str())
    );
    assert_eq!(
        &served.body[..2],
        &[0x1f, 0x8b],
        "the body is not a gzip stream"
    );

    // …and the credential is still required.
    let anonymous = hub.send(Request::get(format!("/workers/artifact/{hash}")));
    assert_eq!(anonymous.status, 401);
    assert!(anonymous.body.is_empty());

    // A hash this hub does not hold names both (§3.5), and one that is not a
    // hash at all says what one looks like.
    let elsewhere = format!("sha256:{}", "0".repeat(64));
    let missing = hub.send(Hub::authorized(
        "GET",
        &format!("/workers/artifact/{elsewhere}"),
    ));
    assert_eq!(missing.status, 404, "{}", body_of(&missing));
    assert_eq!(missing.json()["artifact"], json!(hash));
    assert_eq!(missing.json()["hash"], json!(elsewhere));

    let malformed = hub.send(Hub::authorized("GET", "/workers/artifact/not-a-hash"));
    assert_eq!(malformed.status, 400, "{}", body_of(&malformed));

    // The tarball is exactly what `build` wrote: the emitter's own file list.
    let listed = compose_core::emit(&artifact());
    let entries = tar_entries(&served.body);
    let mut expected: Vec<String> = listed.paths().map(str::to_string).collect();
    expected.sort();
    assert_eq!(
        entries, expected,
        "the served tarball is not the emitted project"
    );
}

/// The paths in a gzipped tar, sorted.
fn tar_entries(gzipped: &[u8]) -> Vec<String> {
    let bytes = gunzip(gzipped);
    let mut found = Vec::new();
    let mut at = 0usize;
    while at + 512 <= bytes.len() {
        let header = &bytes[at..at + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let name = header
            .iter()
            .take(100)
            .take_while(|byte| **byte != 0)
            .map(|byte| *byte as char)
            .collect::<String>();
        let size = usize::from_str_radix(
            std::str::from_utf8(&header[124..135]).expect("an octal size"),
            8,
        )
        .expect("an octal size");
        found.push(name);
        at += 512 + size.div_ceil(512) * 512;
    }
    found.sort();
    found
}

/// Inflate a gzip stream with the toolchain that is already installed.
///
/// A dependency-free decompressor is a page of code nobody should review; `bun`
/// is on the machine because every gate here needs it, and it has one built in.
fn gunzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new("gzip")
        .arg("-dc")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("gzip is on the path");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(bytes)
        .expect("the stream is written");
    let output = child.wait_with_output().expect("gzip runs");
    assert!(output.status.success(), "the served body is not a gzip");
    output.stdout
}

// ---------------------------------------------------------------------------
// §3.2, §3.3, §3.4 — the steady state
// ---------------------------------------------------------------------------

/// The whole loop, once: a placed node parks, a worker takes it, streams an
/// effect home, settles it, and the graph carries on **on the hub**.
///
/// The last clause is what makes this a distribution test rather than a
/// round-trip test: `flow.release`'s second node is a `function:` over an
/// unplaced tool, so one execution runs one node on a worker and the next one
/// here — which is grammar §14.1's "a placement decides which process runs a
/// node", observed rather than asserted.
#[test]
fn a_placed_node_runs_on_a_worker_and_the_rest_of_the_flow_runs_on_the_hub() {
    let Some(hub) = hub() else {
        return;
    };
    let worker = hub.worker();
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));

    let dispatch = worker.dispatch(&hub);
    assert_eq!(dispatch["execution_id"], json!(execution));
    assert_eq!(dispatch["node"], json!("flow.release.sign"));
    assert_eq!(dispatch["instance_path"], json!("sign/0"));
    assert_eq!(dispatch["inputs"], json!({ "path": "dist/app" }));
    assert_eq!(
        dispatch["effect_history"],
        json!([]),
        "a first dispatch has no history to replay"
    );
    let id = dispatch["dispatch_id"]
        .as_str()
        .expect("a dispatch names itself")
        .to_string();
    assert!(id.starts_with("dsp_"), "{id}");

    // §2: the hub MUST NOT answer a session's poll with a dispatch while that
    // session has one it has not settled — however much work is queued.
    let busy = worker.poll(&hub);
    assert_eq!(busy.status, 204, "{}", body_of(&busy));

    // The effects this node issued, handed home as it produced them (§3.3).
    let batch = worker.effects(
        &hub,
        &id,
        &json!([{
            "key": "sign/0#model/0",
            "site": "sign/0",
            "kind": "model",
            "ordinal": 0,
            "request": "{\"model\":\"model.smart\"}",
            "outcome": { "kind": "value", "value": { "text": "signed" } },
        }]),
    );
    assert_eq!(batch.status, 204, "{}", body_of(&batch));
    // …and again, because a batch is safe to re-send after a transport failure:
    // insertion is idempotent by effect key.
    assert_eq!(
        worker
            .effects(
                &hub,
                &id,
                &json!([{
                    "key": "sign/0#model/0",
                    "site": "sign/0",
                    "kind": "model",
                    "ordinal": 0,
                    "request": "{\"model\":\"model.smart\"}",
                    "outcome": { "kind": "value", "value": { "text": "signed" } },
                }]),
            )
            .status,
        204
    );

    let settled = worker.settle(&hub, &id, &json!({ "signature": "signed-by-the-mac" }));
    assert_eq!(settled.status, 204, "{}", body_of(&settled));

    let report = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(
        report["outputs"],
        json!({ "signature": "signed-by-the-mac", "ticket": "notarized" }),
        "the worker's answer and the hub's own node are both in the outputs"
    );

    // A second result for a dispatch a result already settled is accepted and
    // dropped (§3.4's first row).
    assert_eq!(
        worker
            .settle(&hub, &id, &json!({ "signature": "signed-by-the-mac" }))
            .status,
        204
    );
    // …and one for a dispatch this hub never issued is `409`, never `410`.
    let unknown = worker.settle(&hub, "dsp_nothing", &json!({ "signature": "x" }));
    assert_eq!(unknown.status, 409, "{}", body_of(&unknown));
}

/// What a placed node's **stores** did reaches the trace entry the hub writes
/// (PRD 5.8, §4.3).
///
/// The fourth collector a node execution fills, and the only one with no way
/// home of its own: `history`, `models` and `tool_dispatches` are returned by the
/// node function, while a store op answers its *caller* and pushes its record
/// into `context.storeRecords` — an array the worker's process holds a copy of
/// and the hub's node execution never sees. A result that dropped it would give
/// a placed agent's entry `models` and no `stores` where the same agent unplaced
/// reports both, and §4.3's "a placement decides which process runs a node" would
/// have become "a placement decides what a run reports".
#[test]
fn a_placed_nodes_store_records_reach_the_trace_entry_the_hub_writes() {
    let Some(hub) = hub() else {
        return;
    };
    let worker = hub.worker();
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));
    let dispatch = worker.dispatch(&hub);
    let record = json!({
        "store": "store.notes",
        "op": "memory_get",
        "effect": "read",
        "via": "tool",
        "scope": "global",
        "key": "dist/app",
        "answer": { "note": "signed once before" }
    });
    let settled = hub.send(worker.request("POST", "/workers/result").json(&json!({
        "dispatch_id": dispatch["dispatch_id"],
        "output": { "signature": "signed-by-the-mac" },
        "stores": [record.clone()],
    })));
    assert_eq!(settled.status, 204, "{}", body_of(&settled));

    let report = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    let entry = &report["trace"][0];
    assert_eq!(entry["node"], json!("sign"), "{report:#}");
    assert_eq!(
        entry["stores"],
        json!([record]),
        "the store records the worker sent home are not on the node's entry: {report:#}"
    );
}

/// A `function:` node over a placed tool dispatches too, and its answer is held
/// to the tool's `output:` on the way back in (grammar §14.1, §6.1).
///
/// The third way a graph reaches a placed component, and the one grammar §14.1
/// leaves alone — "there its own placement is the whole of the answer" — so it
/// lowers to a different call site from an `agent:` node's, with the contract
/// around it rather than beside it. A worker that answered something the tool's
/// `output:` refuses fails the node, which is what makes the parse a boundary
/// check on a value that crossed the network rather than a formality.
#[test]
fn a_function_node_over_a_placed_tool_dispatches_and_its_answer_is_held_to_the_contract() {
    let Some(hub) = hub() else {
        return;
    };
    let worker = hub.worker();
    let execution = hub.start("/direct-signings", &json!({ "path": "dist/app" }));

    let dispatch = worker.dispatch(&hub);
    assert_eq!(dispatch["node"], json!("flow.direct.sign"));
    assert_eq!(dispatch["instance_path"], json!("sign/0"));
    let id = dispatch["dispatch_id"]
        .as_str()
        .expect("a dispatch id")
        .to_string();

    // An answer the tool's `output:` refuses is the node's failure, not the
    // hub's: `signature` is a string, and this is not one.
    assert_eq!(
        worker.settle(&hub, &id, &json!({ "signature": 17 })).status,
        204,
        "the hub takes the result and lets the node decide about it"
    );
    let failed = hub.until(&execution, "failed", |report| {
        report["status"] == json!("failed")
    });
    let said = failed["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("tool.sign"),
        "the failure does not name what refused the answer: {failed:#}"
    );
}

/// §3.2's four OPTIONAL payload fields: the hub derives them, and a dispatch
/// carries them.
///
/// They are the whole of what keeps "a placement decides which *process* runs a
/// node" (§4.3) from being "a placement decides what a node **does**", and each
/// is a fact a worker cannot compute:
///
///  * `session_key` — grammar §4.1's `execution.session_key`, off the
///    execution's own lifecycle row. Without it a placed component reaching a
///    `scope: session` store would address the empty partition and fail with a
///    diagnostic telling the operator to pass a `--session` the run passed.
///  * `history` — the turns of the shared `messages` channel (grammar §10.4).
///    `flow.conversation` runs an **unplaced** agent first, on the hub, so by
///    the time `sign` is dispatched the channel holds that agent's turns. A
///    payload without them is a node answering from its input object alone.
///  * `item_index` and `policy` — absent here, and asserted absent: §3.2 makes
///    each optional, "a dispatch that has none of them omits all four", and a
///    hub that sent `null` would be sending a value rather than omitting a key.
///
/// The counterpart at the emitter is
/// `compose-core`'s `placement_surface_landing.rs`, which holds the two
/// lowerings — placed and local — to one description; this is the same claim
/// made against a served hub, over the wire, with an execution behind it.
#[test]
fn a_dispatch_carries_the_execution_identity_and_the_conversation_the_node_would_have_read() {
    let Some(hub) = hub() else {
        return;
    };
    // The hub's own agent: `agent.briefer` is unplaced, so this call is made
    // here rather than on a worker, and its answer is what fills `messages`.
    hub.provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "brief": "a release of dist/app" })),
    ));

    let worker = hub.worker();
    let execution = hub.start(
        "/conversations",
        &json!({ "path": "dist/app", "session": "release-42" }),
    );

    let dispatch = worker.dispatch(&hub);
    assert_eq!(dispatch["execution_id"], json!(execution));
    assert_eq!(dispatch["node"], json!("flow.conversation.sign"));
    assert_eq!(
        dispatch["session_key"],
        json!("release-42"),
        "the dispatch does not carry the execution's session key: {dispatch:#}"
    );

    let turns = dispatch["history"]
        .as_array()
        .unwrap_or_else(|| panic!("the dispatch carries no conversation: {dispatch:#}"));
    assert!(
        !turns.is_empty(),
        "the placed node was dispatched with an empty history where the same node unplaced \
         would have seen `agent.briefer`'s turns (grammar §10.4): {dispatch:#}"
    );
    assert!(
        turns
            .iter()
            .any(|turn| turn["text"].as_str().unwrap_or_default().contains("brief")),
        "the turns are not the ones the hub's own agent produced: {dispatch:#}"
    );

    assert_eq!(
        dispatch.get("item_index"),
        None,
        "no `map` encloses this node, so §3.2's key is omitted rather than sent as null: \
         {dispatch:#}"
    );
    assert_eq!(
        dispatch.get("policy"),
        None,
        "nothing instantiated this flow with a `policy:`, so §3.2's key is omitted: {dispatch:#}"
    );

    let id = dispatch["dispatch_id"]
        .as_str()
        .expect("a dispatch id")
        .to_string();
    assert_eq!(
        worker
            .settle(&hub, &id, &json!({ "signature": "signed-after-the-brief" }))
            .status,
        204
    );
    let report = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(
        report["outputs"],
        json!({ "signature": "signed-after-the-brief" })
    );
}

/// A placed node with no worker **parks**, on the board and in the report
/// (§6.1), and a join is what wakes it (§6.2).
#[test]
fn a_placed_node_with_no_worker_parks_and_a_join_is_what_wakes_it() {
    let Some(hub) = hub() else {
        return;
    };
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));

    let parked = hub.until(&execution, "parked", |report| {
        report["placement_waits"]
            .as_array()
            .is_some_and(|waits| !waits.is_empty())
    });
    let wait = &parked["placement_waits"][0];
    assert_eq!(wait["placement"], json!("mac"));
    assert_eq!(wait["node"], json!("flow.release.sign"));
    assert_eq!(wait["instance_path"], json!("sign/0"));
    assert_eq!(wait["status"], json!("parked"));
    assert_eq!(
        wait["wait_id"],
        json!("sign/0/0"),
        "a placement wait's identity is its instance path — which already ends in the node's \
         traversal ordinal (grammar §9.4) — plus an ordinal of its own, so a `retry:` at the \
         same traversal asks for the next dispatch rather than re-using the superseded one (§6.1)"
    );
    assert_eq!(
        parked["status"],
        json!("running"),
        "an execution waiting for a machine is running, not interrupted: nothing here is a \
         question a human can answer"
    );

    // The wake is a join (§6.2), and nothing polls for it.
    let worker = hub.worker();
    let dispatch = worker.dispatch(&hub);
    assert_eq!(dispatch["instance_path"], json!("sign/0"));
    worker.settle(
        &hub,
        dispatch["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "late-but-signed" }),
    );
    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(done["outputs"]["signature"], json!("late-but-signed"));
}

/// A parking fires the `parked` lifecycle webhook, so a subscribed system
/// "learns 'waiting for the Mac' exactly the way it learns 'waiting for a
/// human'" (§6.6, PRD resolved q34).
///
/// The webhook is owed to a **quiescence**, and a placed node awaiting a worker
/// is what makes an execution one: it is a unit of work in flight that cannot
/// advance on its own, which is the same thing a `human` pause is and the reason
/// `runtime.quiescent` had to learn about placement waits at all. The body is
/// the status route's report, so the receiver can see which placement it is
/// waiting on without asking.
#[test]
fn a_placement_wait_fires_the_parked_lifecycle_webhook() {
    let Some(hub) = hub() else {
        return;
    };
    let receiver = harness::Receiver::start().expect("a loopback port");
    let execution = hub.start(
        "/releases",
        &json!({ "path": "dist/app", "callback_url": format!("{}/hook", receiver.base_url) }),
    );

    let parked = receiver.wait_for_event("parked", 1, PATIENCE);
    assert_eq!(parked.len(), 1, "one webhook per parking, not one per wait");
    let body = &parked[0].body;
    assert_eq!(body["execution_id"], json!(execution));
    assert_eq!(body["status"], json!("running"));
    assert_eq!(body["placement_waits"][0]["placement"], json!("mac"));
    assert_eq!(
        body["placement_waits"][0]["node"],
        json!("flow.release.sign")
    );
    assert!(
        body["interrupts"].is_null(),
        "an execution waiting for a machine publishes no question a human could answer: {body:#}"
    );

    // And the parking is announced **once**: a re-park under the same wait id is
    // not a second question, which is what keeps a recovered execution quiet.
    let worker = hub.worker();
    let dispatch = worker.dispatch(&hub);
    worker.settle(
        &hub,
        dispatch["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "signed" }),
    );
    let settled = receiver.wait_for_event("settled", 1, PATIENCE);
    assert_eq!(settled[0].body["status"], json!("completed"));
    assert_eq!(
        receiver.of_event("parked").len(),
        1,
        "the parking was announced more than once"
    );
}

/// A dispatch a worker has **taken** is not a parking, and no webhook says it is
/// (§6.4, §6.6).
///
/// §6.4's table is two rows and this is the line between them: "no worker has
/// taken the node yet" is the pause — the thing a `parked` delivery announces —
/// while "a worker took it" is a node that is *running*, somewhere else. An
/// execution holding one has not stopped advancing on its own, so it is owed no
/// delivery at all, and a report that said otherwise would tell a subscriber that
/// a four-minute build was waiting for a machine that was in fact building.
///
/// `flow.watched` is the shape that can tell the two apart, and its shape is not
/// incidental: LangGraph's superstep barrier holds two *branches* of one graph in
/// step with each other, so a question on one branch cannot open while a node on
/// the other is still running. A **dispatched instance** runs a graph of its own
/// and advances while a sibling instance is parked, so a `map` of two is what
/// reaches a quiescence with one dispatch still out at a worker.
///
/// The liveness window is a long one, because what is being observed is a
/// dispatch a session is **holding**: these workers are a test rather than a
/// process, so they make no request between the poll that took the dispatch and
/// the result that settles it, and the suite's short window would supersede
/// exactly the state this is about (§6.3).
#[test]
fn a_dispatch_a_worker_took_is_not_a_parking() {
    let Some(project) = harness::scratch_project("mesh-watched") else {
        return;
    };
    let held = harness::Scratch::at(project);
    let Some(hub) = hub_into(
        held.path(),
        &[(
            "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS".to_string(),
            "60000".to_string(),
        )],
    ) else {
        return;
    };
    let receiver = harness::Receiver::start().expect("a loopback port");
    let execution = hub.start(
        "/watched-releases",
        &json!({
            "paths": ["dist/one", "dist/two"],
            "callback_url": format!("{}/hook", receiver.base_url),
        }),
    );

    // Both instances park their sign-off, with no worker anywhere: the pause of
    // §6.4's first row, and whatever the parks announced is the delivery count
    // this test measures from.
    let board = hub.until(&execution, "parked both instances", |report| {
        report["placement_waits"]
            .as_array()
            .is_some_and(|waits| waits.len() == 2)
    });
    receiver.wait_for_event("parked", 1, PATIENCE);
    let announced = receiver.of_event("parked").len();
    let ids: Vec<&str> = board["placement_waits"]
        .as_array()
        .expect("two waits")
        .iter()
        .map(|wait| wait["wait_id"].as_str().expect("a wait id"))
        .collect();
    assert_eq!(
        ids,
        ["watch/0/0/sign/0/0", "watch/0/1/sign/0/0"],
        "{board:#}"
    );

    // Two sessions, because §2 hands one session one dispatch at a time and both
    // instances have to be out at once for the question below to open while a
    // node is running.
    let one = hub.worker();
    let first = one.dispatch(&hub);
    let two = hub.worker();
    let second = two.dispatch(&hub);
    let leading = first["instance_path"] == json!("watch/0/0/sign/0");
    let (asking, asker, running, runner) = if leading {
        (&first, &one, &second, &two)
    } else {
        (&second, &two, &first, &one)
    };
    assert_eq!(
        asking["instance_path"],
        json!("watch/0/0/sign/0"),
        "{asking:#}"
    );
    assert_eq!(
        running["instance_path"],
        json!("watch/0/1/sign/0"),
        "{running:#}"
    );

    // Settling one instance's dispatch carries that instance on to its question —
    // while the other instance's dispatch is still out at its worker.
    asker.settle(
        &hub,
        asking["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "signed-first" }),
    );
    let asked = hub.until(&execution, "opened its question", |report| {
        report["interrupts"]
            .as_array()
            .is_some_and(|waits| !waits.is_empty())
    });
    assert_eq!(
        asked["placement_waits"][0]["status"],
        json!("dispatched"),
        "the other instance's node is out at a worker: {asked:#}"
    );
    // Long enough that a delivery this quiescence had taken would have arrived:
    // every other one in this suite lands in milliseconds.
    std::thread::sleep(Duration::from_millis(750));
    assert_eq!(
        receiver.of_event("parked").len(),
        announced,
        "a `parked` webhook was delivered for an execution whose only other work is a node a \
         worker is running (§6.4)"
    );

    // …and when that node's result lands, the execution really has stopped
    // advancing on its own, and the delivery names what it is waiting for.
    runner.settle(
        &hub,
        running["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "signed-second" }),
    );
    let parked = receiver.wait_for_event("parked", announced + 1, PATIENCE);
    let body = &parked[announced].body;
    assert_eq!(body["execution_id"], json!(execution));
    assert!(
        body["placement_waits"].is_null(),
        "the placement waits were gone by then, and the report says so: {body:#}"
    );
    let questions: Vec<&str> = body["interrupts"]
        .as_array()
        .expect("the delivery lists the questions")
        .iter()
        .map(|wait| wait["wait_id"].as_str().expect("a wait id"))
        .collect();
    assert_eq!(
        questions,
        ["watch/0/0/approve/0", "watch/0/1/approve/0"],
        "the delivery names the questions a person can answer: {body:#}"
    );
}

/// A poll already in flight is answered **when the work parks**, not when its
/// hold runs out (§2).
///
/// The hold is what bounds an *empty* answer, and §2 states the cost of the
/// transport as "a small latency floor on dispatch — one round trip after the
/// hold is answered". A hub whose held poll waited out its hold before noticing
/// a dispatch would turn that floor into the hold itself: twenty-five seconds,
/// in production, for work that was ready the instant the poll asked. So this
/// hub is given a hold long enough that waiting one out is unmistakable, and the
/// dispatch has to arrive in a fraction of it.
#[test]
fn a_poll_in_flight_is_answered_when_the_work_parks_rather_than_when_its_hold_ends() {
    let Some(project) = harness::scratch_project("mesh-latency") else {
        return;
    };
    let held = harness::Scratch::at(project);
    let hold = Duration::from_secs(6);
    let Some(hub) = hub_into(
        held.path(),
        &[
            (
                "AGENT_COMPOSE_MESH_POLL_HOLD_MS".to_string(),
                hold.as_millis().to_string(),
            ),
            (
                "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS".to_string(),
                "30000".to_string(),
            ),
        ],
    ) else {
        return;
    };
    let worker = hub.worker();

    // A poll on a thread of its own, with a client of its own: the hold blocks,
    // which is the whole point of it.
    let base = hub.base_url.clone();
    let session = worker.session.clone();
    let polling = std::thread::spawn(move || {
        let client = Client::new(&base)
            .expect("the hub's address parses")
            .with_timeout(Duration::from_secs(30));
        let started = Instant::now();
        let answered = client
            .send(
                Hub::authorized("GET", "/workers/poll").header("x-worker-session", session.clone()),
            )
            .expect("the hub answered");
        (answered, started.elapsed())
    });

    // Long enough that the poll is certainly held, short enough that the hold
    // has most of itself left.
    std::thread::sleep(Duration::from_millis(400));
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));

    let (answered, waited) = polling.join().expect("the polling thread");
    assert_eq!(answered.status, 200, "{}", body_of(&answered));
    assert_eq!(answered.json()["execution_id"], json!(execution));
    assert!(
        waited < hold / 2,
        "the poll waited {waited:?} of a {hold:?} hold for work that parked after 400ms: a \
         dispatch woke nobody, and §2's latency floor became the hold"
    );
}

/// A poll already in flight is answered when **this session's own dispatch
/// settles**, not when its hold runs out (§2).
///
/// The other half of the wake, and the one a queue is drained by. A session is
/// answered `204` for every hold while it holds a dispatch it has not settled
/// (§2), so the instant the result lands that session became eligible for the
/// next item — and the poll that will hand it over is one the hub is already
/// holding. A hub that only ever woke polls when work *arrived* would drain a
/// queue at one item per hold: §2's "small latency floor on dispatch — one round
/// trip after the hold is answered" would become a whole hold per item, which
/// for the default 25s is a `map` of eight spending three minutes idling on a
/// mesh whose nodes answer instantly.
#[test]
fn a_poll_in_flight_is_answered_when_the_session_settles_what_it_was_holding() {
    let Some(project) = harness::scratch_project("mesh-drain") else {
        return;
    };
    let held = harness::Scratch::at(project);
    let hold = Duration::from_secs(6);
    let Some(hub) = hub_into(
        held.path(),
        &[
            (
                "AGENT_COMPOSE_MESH_POLL_HOLD_MS".to_string(),
                hold.as_millis().to_string(),
            ),
            (
                "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS".to_string(),
                "30000".to_string(),
            ),
        ],
    ) else {
        return;
    };
    let worker = hub.worker();
    let execution = hub.start("/batches", &json!({ "paths": ["dist/one", "dist/two"] }));

    // Both items are admitted and parked; this session takes the first and is
    // now busy, so its next poll is held for the whole hold.
    let first = worker.dispatch(&hub);
    let taken = first["dispatch_id"]
        .as_str()
        .expect("a dispatch id")
        .to_string();

    let base = hub.base_url.clone();
    let session = worker.session.clone();
    let polling = std::thread::spawn(move || {
        let client = Client::new(&base)
            .expect("the hub's address parses")
            .with_timeout(Duration::from_secs(30));
        let started = Instant::now();
        let answered = client
            .send(
                Hub::authorized("GET", "/workers/poll").header("x-worker-session", session.clone()),
            )
            .expect("the hub answered");
        (answered, started.elapsed())
    });

    // Long enough that the poll is certainly held, short enough that the hold
    // has most of itself left.
    std::thread::sleep(Duration::from_millis(400));
    let settled = worker.settle(&hub, &taken, &json!({ "signature": "one" }));
    assert_eq!(settled.status, 204, "{}", body_of(&settled));

    let (answered, waited) = polling.join().expect("the polling thread");
    assert_eq!(answered.status, 200, "{}", body_of(&answered));
    let next = answered.json();
    assert_ne!(
        next["dispatch_id"],
        json!(taken),
        "the hold was answered with the dispatch it had already settled: {next:#}"
    );
    assert!(
        waited < hold / 2,
        "the poll waited {waited:?} of a {hold:?} hold after settling what it held at 400ms: a \
         session that became free woke nobody, and §2's latency floor became the hold"
    );

    worker.settle(
        &hub,
        next["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "two" }),
    );
    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(done["outputs"]["signatures"], json!(["one", "two"]));
}

/// A poll whose client hung up takes no dispatch, and stops being a heartbeat.
///
/// A hold ends for two reasons and only one of them means "ask again": the hold
/// expired, or **the connection closed**. A worker that was `SIGKILL`ed, or a
/// poll an intermediary dropped, is the second — and a hub that went round its
/// loop anyway would do two wrong things with one dead socket. It would refresh
/// `session.seen` for a request that has demonstrably ended, extending §6.3's
/// "90 seconds since the last request on a session" by up to a whole hold; and
/// it would `claimDispatch` parked work for a socket nothing can be written to,
/// marking the row `dispatched` against a session that will never settle it. The
/// node then waits out the liveness window before anything supersedes it, and
/// loses an attempt to a dispatch it never received.
///
/// The hold here is deliberately long: the failure needs work to park **inside**
/// the hold the dead poll was in, which is exactly the window the guard covers,
/// and a 300 ms hold leaves no room to arrange it. The window is long too, so a
/// hub that did claim the row is not rescued by the sweep before the second
/// worker asks.
#[test]
fn a_poll_whose_client_hung_up_claims_no_dispatch() {
    let Some(project) = harness::scratch_project("mesh-hangup") else {
        return;
    };
    let held = harness::Scratch::at(project);
    let Some(hub) = hub_into(
        held.path(),
        &[
            (
                "AGENT_COMPOSE_MESH_POLL_HOLD_MS".to_string(),
                "6000".to_string(),
            ),
            (
                "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS".to_string(),
                "30000".to_string(),
            ),
        ],
    ) else {
        return;
    };

    // One session that polls and goes away. The socket is written and then
    // **shut down** rather than left to a client's own budget, because what is
    // being reproduced is a process that stopped existing: an abandoned read
    // leaves the connection open and the hub answering into it, which is a
    // different thing and not the one §6.3 is about.
    let gone = hub.worker();
    let address = hub
        .base_url
        .trim_start_matches("http://")
        .trim_end_matches('/')
        .to_string();
    {
        use std::io::Write;
        let mut socket =
            std::net::TcpStream::connect(&address).expect("the hub accepts a connection");
        write!(
            socket,
            "GET /workers/poll HTTP/1.1\r\nHost: {address}\r\nAuthorization: Bearer {TOKEN}\r\n\
             X-Worker-Session: {}\r\n\r\n",
            gone.session
        )
        .expect("the poll is written");
        socket.flush().expect("the poll is flushed");
        // Long enough that the hub is certainly holding it.
        std::thread::sleep(Duration::from_millis(500));
        socket
            .shutdown(std::net::Shutdown::Both)
            .expect("the socket closes");
    }
    // …and long enough for the hub to notice.
    std::thread::sleep(Duration::from_millis(300));

    // …and the work parks while that hold is still running.
    let execution = hub.start("/releases", &json!({ "path": "dist/app" }));

    // A live worker gets it. If the dead session's loop claimed it instead, this
    // is `204` until the liveness window closes — thirty seconds, which is past
    // this suite's patience, so the failure reads as "no dispatch reached this
    // worker" rather than as a timeout nobody can place.
    let live = hub.worker();
    let dispatch = live.dispatch(&hub);
    assert_eq!(dispatch["execution_id"], json!(execution));
    assert_eq!(dispatch["node"], json!("flow.release.sign"));
}

/// An unknown session is `410` at every session-carrying route, and at those
/// routes only (§3).
#[test]
fn an_unknown_session_is_gone_at_every_route_that_carries_one() {
    let Some(hub) = hub() else {
        return;
    };
    let ghost = Worker {
        session: "wrk_nobody-issued-this".to_string(),
    };
    for (method, path, body) in [
        ("GET", "/workers/poll", Value::Null),
        (
            "POST",
            "/workers/effects",
            json!({ "dispatch_id": "dsp_x", "effects": [] }),
        ),
        (
            "POST",
            "/workers/result",
            json!({ "dispatch_id": "dsp_x", "output": {} }),
        ),
    ] {
        let mut request = ghost.request(method, path);
        if !body.is_null() {
            request = request.json(&body);
        }
        let answered = hub.send(request);
        assert_eq!(
            answered.status,
            410,
            "`{method} {path}` answered a session this hub never issued with something other \
             than `410`: {}",
            body_of(&answered)
        );
    }
}

// ---------------------------------------------------------------------------
// §6.3, §7.2, §7.3 — the mid-node disconnect
// ---------------------------------------------------------------------------

/// A session that stops making requests has its dispatch **superseded**, the
/// attempt fails under the node's own `retry:`, and the retry re-parks — with the
/// effects the first attempt streamed home handed to the second (§6.3, §7.2).
///
/// Everything §7.3 promises about a mid-node disconnect is here: the attempt
/// fails rather than the run, the model call already paid for is in the
/// `effect_history` the redispatch carries rather than issued again, and the
/// revived worker's late result meets `409` (§3.4) rather than re-driving an
/// execution that has moved on.
#[test]
fn a_vanished_session_supersedes_its_dispatch_and_the_retry_carries_the_history() {
    let Some(hub) = hub() else {
        return;
    };
    let first = hub.worker();
    let execution = hub.start("/retried-releases", &json!({ "path": "dist/app" }));

    let dispatch = first.dispatch(&hub);
    let id = dispatch["dispatch_id"]
        .as_str()
        .expect("a dispatch id")
        .to_string();
    // The model call this attempt made and paid for, journaled by the hub.
    // Two of them: the node's own model call, and the tool call its loop made
    // **under** it. A site is "inside" a dispatch by the prefix relation grammar
    // §9.4's paths already carry (§3.2), so both belong to this dispatch's
    // history and the redispatch has to carry both.
    let effect = json!([
        {
            "key": "sign/0#model/0",
            "site": "sign/0",
            "kind": "model",
            "ordinal": 0,
            "request": "{\"model\":\"model.smart\"}",
            "outcome": { "kind": "value", "value": { "text": "half a signature" } },
        },
        {
            "key": "sign/0/tool.sign/0#tool/0",
            "site": "sign/0/tool.sign/0",
            "kind": "tool",
            "ordinal": 0,
            "request": "{\"path\":\"dist/app\"}",
            "outcome": { "kind": "value", "value": { "signature": "half" } },
        },
    ]);
    let handed = first.effects(&hub, &id, &effect);
    assert_eq!(handed.status, 204, "{}", body_of(&handed));

    // **The hub is the single writer, and it writes into the dispatch's own
    // execution and under the dispatch's own site.** A record naming a site
    // outside it is not a record this dispatch could have produced, and taking
    // it would let one session write an effect another node will replay.
    let elsewhere = first.effects(
        &hub,
        &id,
        &json!([{
            "key": "stamp/0#tool/0",
            "site": "stamp/0",
            "kind": "tool",
            "ordinal": 0,
            "request": "{}",
            "outcome": { "kind": "value", "value": {} },
        }]),
    );
    assert_eq!(elsewhere.status, 400, "{}", body_of(&elsewhere));

    // …and then the laptop closes. Nothing else is sent on this session, so the
    // liveness window runs out and the hub declares it gone.
    let second = Worker {
        session: hub.worker().session,
    };
    let redispatch = second.dispatch(&hub);
    assert_ne!(
        redispatch["dispatch_id"], dispatch["dispatch_id"],
        "the retry re-used the superseded dispatch rather than opening one"
    );
    assert_eq!(redispatch["instance_path"], json!("sign/0"));
    assert_eq!(
        redispatch["wait_id"],
        Value::Null,
        "a dispatch payload carries what §3.2 lists and nothing more"
    );
    let history = redispatch["effect_history"]
        .as_array()
        .expect("a redispatch carries the history");
    assert_eq!(
        history.len(),
        2,
        "the redispatch did not carry both effects the first attempt journaled — its own and the \
         one its tool loop made under it (§3.2, §7.2): {redispatch:#}"
    );
    let keys: Vec<&str> = history
        .iter()
        .map(|record| record["key"].as_str().expect("a key"))
        .collect();
    assert_eq!(keys, ["sign/0#model/0", "sign/0/tool.sign/0#tool/0"]);
    assert_eq!(
        history[0]["outcome"]["value"],
        json!({ "text": "half a signature" }),
        "the recorded answer is what the replay consumes instead of calling the model again"
    );

    // The revived worker comes back, and meets both halves of §3 in the order
    // §6.3 fixes. Its first request carries a session the hub has forgotten, so
    // it is `410` and the worker joins again…
    let late = first.settle(&hub, &id, &json!({ "signature": "too-late" }));
    assert_eq!(late.status, 410, "{}", body_of(&late));
    // …and the result it then posts for that dispatch is `409` and discarded:
    // the superseded row, because this hub's own declaration is what superseded
    // it. Never the `204` row, which is a dispatch a **result** ended.
    let revived = hub.worker();
    let dead = revived.settle(&hub, &id, &json!({ "signature": "too-late" }));
    assert_eq!(dead.status, 409, "{}", body_of(&dead));
    assert_eq!(dead.json()["dispatch_id"], json!(id));

    // The second attempt finishes the run.
    second.settle(
        &hub,
        redispatch["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "signed-on-the-retry" }),
    );
    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(done["outputs"]["signature"], json!("signed-on-the-retry"));
}

// ---------------------------------------------------------------------------
// §2 — queueing
// ---------------------------------------------------------------------------

/// A `map` admitting four items onto a placement with one worker runs them
/// **one at a time**, and the rest are undispatched (§2, §6.4).
///
/// "A `max_concurrency: 8` over an `agent.signer` placed on `mac`, with one
/// worker on the Mac, runs one at a time: the map admits eight, the placement
/// delivers one, and the other seven are queued to the placement." The map here
/// admits four; the assertion is that the session is never holding two.
#[test]
fn a_fan_out_onto_a_one_worker_placement_runs_one_item_at_a_time() {
    let Some(hub) = hub() else {
        return;
    };
    let worker = hub.worker();
    let execution = hub.start(
        "/batches",
        &json!({ "paths": ["dist/one", "dist/two", "dist/three"] }),
    );

    let mut taken = Vec::new();
    for _ in 0..3 {
        let dispatch = worker.dispatch(&hub);
        // Every one of them is an item of the same fan-out, and while it is
        // unsettled the session is answered nothing.
        let busy = worker.poll(&hub);
        assert_eq!(
            busy.status,
            204,
            "a session was handed a second unsettled dispatch: {}",
            body_of(&busy)
        );
        let id = dispatch["dispatch_id"]
            .as_str()
            .expect("a dispatch id")
            .to_string();
        taken.push(
            dispatch["instance_path"]
                .as_str()
                .expect("an instance path")
                .to_string(),
        );
        worker.settle(&hub, &id, &json!({ "signature": "signed" }));
    }
    taken.sort();
    assert_eq!(
        taken,
        ["fan/0/0", "fan/0/1", "fan/0/2"]
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        "the three items dispatched under their own instance paths (grammar §9.4)"
    );

    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(
        done["outputs"]["signatures"],
        json!(["signed", "signed", "signed"])
    );
}

/// The status report publishes placement waits **in park order** (§6.2).
///
/// The order a queue drains in is the order the board resumes in — `parked_at`,
/// then the ordinal at the instance path — and a report that sorted its waits as
/// strings would publish a different one: `fan/0/10/0` before `fan/0/2/0`. The
/// journal's own read was given `ORDER BY parked_at ASC, rowid ASC` for exactly
/// this reason (`tests/toolchain/dispatch-board.mjs` holds that half), and a
/// reader watching a queue drain has to see the rows in the order they will be
/// taken in. Twelve items, because nothing under ten can tell the two orders
/// apart.
#[test]
fn the_status_report_publishes_placement_waits_in_park_order() {
    let Some(hub) = hub() else {
        return;
    };
    let paths: Vec<String> = (0..12).map(|index| format!("dist/{index}")).collect();
    let execution = hub.start("/batches", &json!({ "paths": paths }));

    let board = hub.until(&execution, "parked all twelve items", |report| {
        report["placement_waits"]
            .as_array()
            .is_some_and(|waits| waits.len() == 12)
    });
    let published: Vec<&str> = board["placement_waits"]
        .as_array()
        .expect("twelve waits")
        .iter()
        .map(|wait| wait["wait_id"].as_str().expect("a wait id"))
        .collect();
    let expected: Vec<String> = (0..12).map(|index| format!("fan/0/{index}/0")).collect();
    assert_eq!(
        published, expected,
        "the report published its waits in an order that is not the order they will be taken \
         in (§6.2): {board:#}"
    );
}

// ---------------------------------------------------------------------------
// §5 — a hub replaced behind its own name
// ---------------------------------------------------------------------------

/// A restarted hub forgets its sessions, and keeps the board that had **not**
/// been handed out (§5, §8 rule 3).
///
/// The half of §5 that costs nothing: a wait nobody was holding is a row in the
/// journal, and the journal is not the process. The worker of the dead process
/// meets `410`, joins again, and is handed the very dispatch its predecessor
/// parked — same id, same wait identity, because §6.1 makes that identity
/// deterministic rather than a handle.
#[test]
fn a_hub_restarted_under_its_own_name_keeps_the_work_nobody_had_taken() {
    let Some(project) = shared_project("mesh-restart-parked") else {
        return;
    };
    let execution;
    let parked;
    {
        let Some(hub) = hub_into(&project, &[]) else {
            return;
        };
        execution = hub.start("/releases", &json!({ "path": "dist/app" }));
        // Parked, and **not** taken: no worker joins in this process at all.
        let report = hub.until(&execution, "parked", |report| {
            report["placement_waits"]
                .as_array()
                .is_some_and(|waits| !waits.is_empty())
        });
        parked = report["placement_waits"][0]["dispatch_id"]
            .as_str()
            .expect("a dispatch id")
            .to_string();
        // …and the process dies here. `Served`'s drop kills the app and the
        // command that launched it.
    }

    let Some(hub) = hub_into(&project, &[]) else {
        return;
    };
    // The session the dead process issued is one this one never heard of.
    let ghost = Worker {
        session: "wrk_from-the-process-that-died".to_string(),
    };
    assert_eq!(ghost.poll(&hub).status, 410);

    let worker = hub.worker();
    let again = worker.dispatch(&hub);
    assert_eq!(again["execution_id"], json!(execution), "{again:#}");
    assert_eq!(again["instance_path"], json!("sign/0"), "{again:#}");
    assert_eq!(
        again["dispatch_id"],
        json!(parked),
        "a resumed hub opened a second dispatch for a wait its predecessor had parked and \
         nobody had taken"
    );
    worker.settle(
        &hub,
        &parked,
        &json!({ "signature": "signed-after-the-restart" }),
    );
    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(
        done["outputs"]["signature"],
        json!("signed-after-the-restart")
    );
    drop(harness::Scratch::at(project));
}

/// A dispatch the **replaced** process was holding is superseded, and the
/// node's `retry:` is what dispatches it again (§5, §6.3, §7.3).
///
/// §5's own words for the state this leaves behind: "an unsettled dispatch on an
/// ended session is superseded exactly as §6.3 supersedes one, so its node's
/// attempt fails under that node's `retry:`/`on_error:` chain". The sessions of
/// a process that has been replaced are ended by definition — they lived in its
/// memory — so every row it left `dispatched` is one of those.
///
/// **What the alternative would be is why this is asserted at all.** Putting the
/// row back on the board would hand one instance path to a second session while
/// the worker of the dead process is still executing it: that worker knows
/// nothing about the restart, and the new process's "is this session already
/// holding something" is read off rows *it* handed out, of which it has none. Two
/// executions of one node instance would then run at once, the second replaying
/// an `effect_history` that does not hold what the first is issuing — so the
/// model call §7.3 promises is not paid for twice would be paid for twice.
#[test]
fn a_dispatch_a_replaced_hub_was_holding_is_superseded_and_retried() {
    let Some(project) = shared_project("mesh-restart-dispatched") else {
        return;
    };
    let execution;
    let orphan;
    {
        let Some(hub) = hub_into(&project, &[]) else {
            return;
        };
        let worker = hub.worker();
        execution = hub.start("/retried-releases", &json!({ "path": "dist/app" }));
        let dispatch = worker.dispatch(&hub);
        orphan = dispatch["dispatch_id"]
            .as_str()
            .expect("a dispatch id")
            .to_string();
        assert_eq!(dispatch["instance_path"], json!("sign/0"));
        // …and the process dies here, with that dispatch out at a worker.
    }

    let Some(hub) = hub_into(&project, &[]) else {
        return;
    };
    // The retry is a **different** dispatch, at the next wait ordinal of the
    // same instance path: §6.1's identity is the node's path plus an ordinal, so
    // a second attempt asks for a dispatch of its own rather than re-using the
    // one the dead process handed out.
    let worker = hub.worker();
    let retried = worker.dispatch(&hub);
    assert_eq!(retried["execution_id"], json!(execution), "{retried:#}");
    assert_eq!(retried["instance_path"], json!("sign/0"), "{retried:#}");
    assert_ne!(
        retried["dispatch_id"],
        json!(orphan),
        "the resumed hub handed a second session the dispatch its predecessor had already given \
         to a worker that may still be running it"
    );
    let board = hub.report(&execution);
    assert_eq!(
        board["placement_waits"][0]["wait_id"],
        json!("sign/0/1"),
        "the attempt after a superseded one parks at the next ordinal (§6.1): {board:#}"
    );

    // And the worker of the dead process, coming back with the answer it did
    // produce: `409`, discarded, exactly as §3.4 answers a superseded dispatch.
    let revived = hub.worker();
    let late = revived.settle(
        &hub,
        &orphan,
        &json!({ "signature": "from-the-old-process" }),
    );
    assert_eq!(late.status, 409, "{}", body_of(&late));
    assert_eq!(late.json()["dispatch_id"], json!(orphan));

    worker.settle(
        &hub,
        retried["dispatch_id"].as_str().expect("a dispatch id"),
        &json!({ "signature": "signed-on-the-retry" }),
    );
    let done = hub.until(&execution, "completed", |report| {
        report["status"] == json!("completed")
    });
    assert_eq!(done["outputs"]["signature"], json!("signed-on-the-retry"));
    drop(harness::Scratch::at(project));
}

// ---------------------------------------------------------------------------
// The launch checks
// ---------------------------------------------------------------------------

/// A mesh whose join token resolved to nothing refuses to serve.
///
/// The same posture `./serve.ts` takes to a trigger credential set to the empty
/// string, applied to the one that admits a worker to the whole mesh (§9.3): an
/// empty expected token compares equal to the empty token every anonymous caller
/// can send.
#[test]
fn a_hub_whose_join_token_is_empty_refuses_to_serve() {
    let Some(out) = harness::scratch_project("mesh-blank") else {
        return;
    };
    let provider = MockProvider::start().expect("a loopback port");
    let mut held: BTreeMap<String, String> = environment(&provider).into_iter().collect();
    held.insert(TOKEN_VARIABLE.to_string(), String::new());
    let refused = harness::serve_refused_target(
        &out,
        &harness::fixture(FIXTURE),
        TARGET,
        &held.into_iter().collect::<Vec<_>>(),
    );
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains(TOKEN_VARIABLE) && said.contains("empty"),
        "the app started, or refused for another reason: {said}"
    );
    drop(harness::Scratch::at(out));
}

/// A poll hold that is not shorter than the liveness window is refused at launch
/// (§2's one normative relationship between them).
#[test]
fn a_hold_that_outlives_the_liveness_window_is_refused_at_launch() {
    let Some(out) = harness::scratch_project("mesh-timings") else {
        return;
    };
    let provider = MockProvider::start().expect("a loopback port");
    let mut held = environment(&provider);
    held.push((
        "AGENT_COMPOSE_MESH_POLL_HOLD_MS".to_string(),
        "2000".to_string(),
    ));
    held.push((
        "AGENT_COMPOSE_MESH_LIVENESS_WINDOW_MS".to_string(),
        "1000".to_string(),
    ));
    let refused = harness::serve_refused_target(&out, &harness::fixture(FIXTURE), TARGET, &held);
    let said = String::from_utf8_lossy(&refused.stderr);
    assert!(
        said.contains("AGENT_COMPOSE_MESH_POLL_HOLD_MS"),
        "the app started, or refused for another reason: {said}"
    );
    drop(harness::Scratch::at(out));
}
