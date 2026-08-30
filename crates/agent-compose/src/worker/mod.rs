//! `agent-compose worker`: the spoke of `docs/distributed.md`.
//!
//! ```text
//! agent-compose worker --hub <url> --claim <name>… --token-env <VAR>
//!                      [--data-dir <path>]
//! ```
//!
//! A **complete protocol client**, and nothing else. It holds no checkout, no
//! YAML, no journal and no scheduler (§1); it joins a hub, is handed the
//! artifact, and runs the nodes placed on the names it claims. Every route it
//! speaks is [`wire`]; every status it acts on is §3's, and the two tables below
//! are the whole of its behaviour.
//!
//! # The provisioning cycle (§3.1, §4)
//!
//! ```text
//! join ──▶ the hub names an artifact
//!            ├── the one this worker holds ──▶ dispatchable, no download
//!            └── another (or this worker holds none)
//!                  └─▶ fetch ─▶ verify ─▶ materialise ─▶ bun install ─▶ join again
//! ```
//!
//! The cycle is entered from three places and from nowhere else: at start, when
//! a join names a hash this worker does not hold, and when a fetch meets §3.5's
//! `404`. A re-join answered with a **different** artifact is a redeployment
//! reaching this worker (§5), and it re-enters here rather than being patched
//! into a running session.
//!
//! A cold start is the same code path at its limit: a worker holding nothing
//! omits `artifact_hash`, the hub answers with the current one, and "this hash
//! is not the one I hold" is trivially true. §3.1 asks for exactly that — "a hub
//! compares an **absent** `artifact_hash` the way it compares a stale one …
//! nothing here needs a rule of its own for the cold start".
//!
//! # The steady state (§2, §3.2)
//!
//! One held `GET` at a time, from the moment it joins until it stops, **and it
//! does not suspend while a node runs**: the poll is the heartbeat, and a
//! four-minute build is four minutes of holds that come back empty. So the poll
//! is this thread and a dispatch is another, which is the whole of the
//! concurrency here — §2 fixes a session's capacity at one, and §13 forbids an
//! implementation from deciding otherwise, so there is no capacity anywhere in
//! this module.
//!
//! One node runs at a time, and that is the capacity. A dispatch answered while
//! one is running is **queued**, never waited on: waiting would stop the poll,
//! and a poll that stops is a worker the hub declares gone (§6.3). The hub hands
//! one over legitimately once it has superseded the running dispatch on that
//! node's `timeout:` — §3.4 makes a superseded dispatch not-unsettled, so §2's
//! "MUST NOT answer a session's poll with a dispatch while that session has a
//! dispatch it has not settled" is not breached and this worker cannot tell that
//! from a hub that breached it. Both are answered the same way, because both are
//! a real journaled dispatch this worker is owed a result for.
//!
//! # What each status means, once
//!
//! | where | status | what this worker does |
//! |---|---|---|
//! | any session route | `410` | join again, and make the request again (§5) |
//! | `/workers/join` | `400`, `401`, `403`, `409` | **stop**, non-zero, echoing the refusal (§3.1) |
//! | `/workers/result` | `409` | discard the result, keep the session, go on polling (§3.4) |
//! | `/workers/artifact` | `404` | join again — over a *hash*, not a session (§3.5) |
//! | anything | no answer at all | [`Backoff`], and try again (§2) |
//!
//! The two rows a reader should not merge are the second and the first: §10.1
//! calls them "the whole of what a worker's error handling has to decide", and
//! reading them the other way round would either hammer a hub that has refused
//! this worker or fail a mesh a hub restart should have healed.
//!
//! # This worker never joins after a refusal
//!
//! Not once, and not for any status: §3.1 makes a refused join terminal because
//! "a second join would be refused identically", and every row of its table is
//! such a condition. The one that looks like an exception is not, and the reason
//! is on the hub's side of the same section: a worker holding an artifact cannot
//! know whether the hub still serves it, so it sends the hash and the report
//! together every time — and a report that turns out to be against a **stale**
//! hash is *ignored* rather than refused. So the join succeeds, the answer names
//! the current artifact, and the provisioning cycle above is entered. There is no
//! body a second join could send that the first did not.
//! `crates/agent-compose/tests/distributed_worker_protocol.rs` holds that.

mod artifact;
mod node;
mod wire;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use wire::{Answer, Hub};

/// The version of `docs/distributed.md` this worker speaks (§10).
///
/// The compiler's own constant rather than a copy: a worker and the hub it is
/// built beside come out of one release, and
/// `compose_core::codegen::mesh::PROTOCOL_VERSION` is already pinned to the
/// number `src/mesh.ts` declares.
const PROTOCOL: u32 = compose_core::codegen::mesh::PROTOCOL_VERSION;

/// The default poll hold this worker waits out (§2), as a client-side bound.
///
/// Not a knob: the hold belongs to the hub, and this is only how long a `GET`
/// may take before the connection is presumed dead. A hub configured shorter
/// answers sooner and this never notices; a hub configured longer is one this
/// worker is not built against.
const POLL_HOLD: Duration = Duration::from_secs(25);

/// What `agent-compose worker` was asked to be.
pub(crate) struct Options {
    /// The hub's base URL — a **name**, never a process (§5 rule 3).
    pub(crate) hub: String,
    /// The placement names this worker claims (§3.1).
    pub(crate) claims: Vec<String>,
    /// The variable the join token is read from.
    ///
    /// Named by the invocation rather than by the artifact, and it has to be:
    /// the artifact is what the *hub* serves after the join, so a worker that
    /// read the variable's name out of it could not have joined to get it.
    pub(crate) token_env: String,
    /// Where this worker keeps the artifacts it materialises.
    pub(crate) data_dir: Option<PathBuf>,
}

/// Why the worker stopped.
#[derive(Clone, Debug)]
pub(crate) enum Stop {
    /// A refusal another attempt would meet identically (§3.1): terminal.
    Refused(String),
    /// The hub serves an artifact this worker does not hold (§4, §5).
    ///
    /// Not a failure: the provisioning cycle is re-entered over this hash, which
    /// is what "redeployment is automatic on the next join" means.
    Redeployed(String),
}

/// Nothing was reported.
const CLEAN: u8 = 0;
/// The command could not run.
const UNUSABLE: u8 = 2;

/// Run the verb.
pub(crate) fn run(options: &Options) -> ExitCode {
    let token = match std::env::var(&options.token_env) {
        Ok(token) if !token.trim().is_empty() => token,
        _ => {
            return fail(&format!(
                "`{}` is the variable this worker was told to read the join token from and it is \
                 not set: the token is the deploy target's `hub.join_token:` and a worker presents \
                 it on every request (docs/distributed.md §3, grammar §14.2)",
                options.token_env
            ));
        }
    };
    if options.claims.is_empty() {
        return fail(
            "a worker claims at least one placement: `--claim <name>`, repeated for each \
             (docs/distributed.md §3.1)",
        );
    }
    let bun = match bun() {
        Ok(bun) => bun,
        Err(reason) => return fail(&reason),
    };
    let runtime = match runtime_line(&bun) {
        Ok(line) => line,
        Err(reason) => return fail(&reason),
    };
    let data_dir = options.data_dir.clone().unwrap_or_else(default_data_dir);
    if let Err(error) = std::fs::create_dir_all(&data_dir) {
        return fail(&format!(
            "cannot use `{}` as this worker's data directory: {error}",
            data_dir.display()
        ));
    }

    let hub = Arc::new(Hub::new(&options.hub, &token));
    // **One backoff for the whole provisioning cycle, not one per pass.** §2's
    // schedule is a doubling interval, and it only doubles if it survives the
    // thing it is pacing: a hub whose join keeps naming a hash its artifact
    // route answers `404` for — a tree edited under a running hub, a build that
    // half-wrote — sends this loop round join-then-fetch indefinitely, and a
    // fresh `Backoff` each pass would peg that at one second for ever. Reset on
    // a provision that worked, so a real redeployment still costs one second.
    let mut backoff = Backoff::new();
    loop {
        match serve(options, &hub, &bun, &data_dir, &runtime) {
            Ok(()) => return ExitCode::from(CLEAN),
            Err(Stop::Refused(detail)) => return fail(&detail),
            Err(Stop::Redeployed(hash)) => {
                // The four filesystem steps of §4, over the hash the join named.
                if let Err(reason) = provision(&hub, &bun, &data_dir, &hash, &mut backoff) {
                    return fail(&reason);
                }
            }
        }
    }
}

/// Fetch, verify, materialise and install one artifact (§4 steps 2–4).
fn provision(
    hub: &Hub,
    bun: &Path,
    data_dir: &Path,
    hash: &str,
    backoff: &mut Backoff,
) -> Result<(), String> {
    let tarball = loop {
        match hub.artifact(hash, None) {
            Answer::Said(said) if said.status == 200 => break said.body,
            // §3.5: a worker meeting `404` for the hash its join returned
            // re-joins rather than retrying the fetch — the join is what
            // re-derives the current hash. Answering `Ok` here is that re-join:
            // the caller's loop starts a new cycle, which begins with one.
            Answer::Said(said) if said.status == 404 => {
                // …under the backoff, and that is the one thing this row adds
                // to what §3.5 says. A hub that answers `404` for the hash its
                // own join keeps naming is broken rather than redeployed, and a
                // re-join with no interval in front of it would be a spin
                // against it. The interval is the *caller's* — see [`run`] — so
                // a hub stuck in that state is met with §2's doubling rather
                // than with one second for ever; a real redeployment costs one
                // second here, once, because the provision that follows resets
                // it.
                backoff.wait(&format!(
                    "the artifact fetch answered 404: {}. Re-joining, because the join is what \
                     re-derives the current hash (docs/distributed.md §3.5)",
                    said.detail()
                ));
                return Ok(());
            }
            Answer::Said(said) if said.status == 401 || said.status == 400 => {
                return Err(format!(
                    "this hub refused the artifact fetch with {}: {}",
                    said.status,
                    said.detail()
                ));
            }
            Answer::Said(said) => backoff.wait(&format!(
                "the artifact fetch answered {}: {}",
                said.status,
                said.detail()
            )),
            Answer::Unreachable(reason) => backoff.wait(&reason),
        }
    };
    let tree = artifact::materialise(data_dir, hash, &tarball)?;
    artifact::install(bun, &tree)?;
    // A cycle that ended in an artifact on disk: whatever the interval had
    // climbed to was about a hub that is now answering.
    backoff.reset();
    Ok(())
}

/// One provisioning-and-steady-state cycle: join, and then poll until something
/// ends it.
fn serve(
    options: &Options,
    hub: &Arc<Hub>,
    bun: &Path,
    data_dir: &Path,
    runtime: &str,
) -> Result<(), Stop> {
    let held = artifact::held(data_dir);
    let (hash, manifest) = match &held {
        Some((hash, tree)) => {
            let manifest = artifact::Manifest::read(tree).map_err(Stop::Refused)?;
            (Some(hash.clone()), Some(manifest))
        }
        None => (None, None),
    };
    let env_ok = manifest
        .as_ref()
        .map(|manifest| manifest.env_ok(&options.claims));

    let sessions = Sessions::new(
        Arc::clone(hub),
        Joining {
            claims: options.claims.clone(),
            runtime: runtime.to_string(),
            artifact: hash,
            env_ok,
        },
    );
    // The join happens here, and a worker holding nothing — or holding an
    // artifact this hub no longer serves — leaves through `Stop::Redeployed`
    // before a poll is ever made.
    let mut session = sessions.current()?;

    let (tree, runner) = match (held, manifest) {
        (Some((_, tree)), Some(manifest)) => (tree, manifest.runner),
        // Unreachable: a session issued against an artifact this worker does not
        // hold is exactly what `Stop::Redeployed` is, and `current()` above
        // answered one or the other.
        _ => {
            return Err(Stop::Redeployed(String::new()));
        }
    };

    let sessions = Arc::new(sessions);
    let mut backoff = Backoff::new();
    std::thread::scope(|scope| -> Result<(), Stop> {
        // At most one **running**, because §2 hands a session one dispatch at a
        // time and §13 forbids this implementation from deciding otherwise.
        let mut running: Option<std::thread::ScopedJoinHandle<'_, Result<(), Stop>>> = None;
        // …and whatever the hub handed over anyway, in the order it arrived.
        // Taken at the top of the next turn of this loop, which is **before**
        // the next poll, so a dispatch answered to a free session starts with no
        // latency added and one answered to a busy one starts the moment the
        // node in hand ends. See the `200` arm.
        let mut waiting: std::collections::VecDeque<Value> = std::collections::VecDeque::new();
        loop {
            if running
                .as_ref()
                .is_some_and(std::thread::ScopedJoinHandle::is_finished)
            {
                let held = running.take().expect("the handle was just seen");
                settled(held.join())?;
            }
            if running.is_none()
                && let Some(dispatch) = waiting.pop_front()
            {
                let sessions = Arc::clone(&sessions);
                let tree = tree.clone();
                let runner = runner.clone();
                let hub = Arc::clone(hub);
                running =
                    Some(scope.spawn(move || {
                        node::execute(&hub, &sessions, &tree, bun, &runner, &dispatch)
                    }));
            }
            match hub.poll(&session.id, POLL_HOLD) {
                Answer::Said(said) if said.status == 200 => {
                    backoff.reset();
                    // **Queued, and the poll goes on.** A dispatch answered to a
                    // session that has not settled the one it holds is ordinarily
                    // a hub in breach of §2 — but it is also what a conformant hub
                    // does once §3.4 has superseded the running dispatch on the
                    // node's `timeout:`, because a superseded dispatch is no
                    // longer *unsettled* and this worker has no way to know that
                    // happened. Either way it is a real dispatch, journaled, and
                    // this worker is owed a result for it, so it is queued rather
                    // than dropped.
                    //
                    // What this must not do is **wait** for the node in hand. The
                    // poll is the heartbeat (§3.2) and §2 requires it to continue
                    // while a node runs: a thread blocked on a slow runner makes
                    // no request for the whole of it, so at ninety seconds the hub
                    // declares this session gone and supersedes the second
                    // dispatch too — a second attempt burned for a condition that
                    // was the hub's own dispatch decision.
                    waiting.push_back(said.json());
                }
                Answer::Said(said) if said.status == 204 => backoff.reset(),
                Answer::Said(said) if said.status == 410 => {
                    backoff.reset();
                    session = sessions.renew(session.generation)?;
                }
                Answer::Said(said) if said.status == 401 => {
                    return Err(Stop::Refused(format!(
                        "this hub refused the join token at `/workers/poll`: {}. The credential is \
                         the one the join used, so joining again cannot improve it \
                         (docs/distributed.md §3.2)",
                        said.detail()
                    )));
                }
                // Anything else is a hub this worker cannot read, which §2 puts
                // under the backoff with a socket that closed.
                Answer::Said(said) => backoff.wait(&format!(
                    "a poll answered {}: {}",
                    said.status,
                    said.detail()
                )),
                Answer::Unreachable(reason) => backoff.wait(&reason),
            }
            // A session the *node* thread renewed is the one this poll must use.
            session = sessions.current()?;
        }
    })
}

/// What a finished dispatch thread answered, with a panic **said** rather than
/// swallowed.
///
/// A thread that panicked settled nothing, so the hub is left holding an
/// unsettled dispatch until that node's `timeout:` chain fires from dispatch
/// (§6.5) — bounded, but not free, and invisible unless this says so. It is not
/// a reason to stop the worker: a panic is a bug in one dispatch, and the
/// placement's other work is still this process's to do.
fn settled(joined: std::thread::Result<Result<(), Stop>>) -> Result<(), Stop> {
    match joined {
        Ok(answered) => answered,
        Err(_) => {
            note(
                "a dispatch this worker was running ended in a panic and settled nothing: the \
                 hub is holding it until that node's `timeout:` fires (docs/distributed.md §6.5). \
                 This is a bug — please report it",
            );
            Ok(())
        }
    }
}

/// What every join of one cycle carries (§3.1).
struct Joining {
    claims: Vec<String>,
    runtime: String,
    /// The hash this worker holds, or `None` for a cold start.
    artifact: Option<String>,
    /// The report, present exactly when [`Joining::artifact`] is.
    env_ok: Option<Vec<String>>,
}

/// One session this worker is holding.
#[derive(Clone)]
pub(crate) struct Held {
    pub(crate) id: String,
    /// How many sessions this worker has had, so two threads meeting `410` at
    /// once re-join once between them.
    pub(crate) generation: u64,
}

/// The session, and the joining that replaces it (§5).
///
/// Sessions are advisory and disposable — "a `410` is answered by joining again
/// and retrying the request that met it" — and this is the one place a join
/// happens, so the poll thread and the thread running a dispatch cannot each
/// make one. A thread that meets `410` hands in the generation it was using; if
/// somebody has already re-joined past it, it is given the new session and makes
/// no request of its own.
pub(crate) struct Sessions {
    hub: Arc<Hub>,
    joining: Joining,
    /// The session, under a lock a join is taken out inside.
    held: Mutex<Option<Held>>,
    /// How many joins this worker has completed, read without the lock.
    generation: AtomicU64,
    /// Set once, when something ends this cycle for every thread at once.
    stopped: Mutex<Option<Stop>>,
}

impl Sessions {
    fn new(hub: Arc<Hub>, joining: Joining) -> Self {
        Self {
            hub,
            joining,
            held: Mutex::new(None),
            generation: AtomicU64::new(0),
            stopped: Mutex::new(None),
        }
    }

    /// The session this worker is holding, joining if it holds none.
    pub(crate) fn current(&self) -> Result<Held, Stop> {
        if let Some(stop) = self.stopped() {
            return Err(stop);
        }
        let mut held = self.held.lock().expect("the session lock is not poisoned");
        if let Some(session) = held.as_ref() {
            return Ok(session.clone());
        }
        self.join(&mut held)
    }

    /// Replace the session `seen` came from, or answer the one that replaced it.
    pub(crate) fn renew(&self, seen: u64) -> Result<Held, Stop> {
        if let Some(stop) = self.stopped() {
            return Err(stop);
        }
        let mut held = self.held.lock().expect("the session lock is not poisoned");
        if let Some(session) = held.as_ref()
            && session.generation > seen
        {
            return Ok(session.clone());
        }
        *held = None;
        self.join(&mut held)
    }

    fn stopped(&self) -> Option<Stop> {
        self.stopped
            .lock()
            .expect("the stop lock is not poisoned")
            .clone()
    }

    fn stop(&self, stop: Stop) -> Stop {
        let mut held = self.stopped.lock().expect("the stop lock is not poisoned");
        held.get_or_insert(stop).clone()
    }

    /// `POST /workers/join`, with §3.1's refusals on it.
    ///
    /// **Every refusal is terminal, and there is no second join anywhere in
    /// here.** §3.1 says so and §10.1 calls the `410`-versus-refusal decision
    /// "the whole of what a worker's error handling has to decide"; what makes
    /// that satisfiable for a worker holding an artifact is the hub's side of the
    /// same section — a report sent with a hash that turned out to be stale is
    /// **ignored** rather than refused, so the one refusal a different body could
    /// have cleared does not exist.
    fn join(&self, held: &mut Option<Held>) -> Result<Held, Stop> {
        let mut backoff = Backoff::new();
        loop {
            let body = self.body();
            match self.hub.join(&body) {
                Answer::Said(said) if said.status == 200 => {
                    let answered = said.json();
                    let serving = answered
                        .get("artifact")
                        .and_then(|artifact| artifact.get("hash"))
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_string();
                    // §4 step 1, and §5's redeployment rule read from this side:
                    // a session issued against an artifact this worker does not
                    // hold is one it may not execute out of.
                    if Some(&serving) != self.joining.artifact.as_ref() {
                        return Err(self.stop(Stop::Redeployed(serving)));
                    }
                    let Some(id) = answered.get("worker_session").and_then(Value::as_str) else {
                        return Err(self.stop(Stop::Refused(
                            "this hub accepted the join and issued no `worker_session`, so there \
                             is no session to poll under (docs/distributed.md §3.1)"
                                .to_string(),
                        )));
                    };
                    let session = Held {
                        id: id.to_string(),
                        generation: self.generation.fetch_add(1, Ordering::SeqCst) + 1,
                    };
                    *held = Some(session.clone());
                    return Ok(session);
                }
                Answer::Said(said) if (400..500).contains(&said.status) => {
                    return Err(self.stop(Stop::Refused(format!(
                        "this hub refused the join with {}: {}. A refused join is terminal — \
                         another would be refused identically (docs/distributed.md §3.1)",
                        said.status,
                        said.detail()
                    ))));
                }
                Answer::Said(said) => backoff.wait(&format!(
                    "a join answered {}: {}",
                    said.status,
                    said.detail()
                )),
                Answer::Unreachable(reason) => backoff.wait(&reason),
            }
        }
    }

    /// The join body (§3.1).
    ///
    /// One shape, sent every time: the hash and the report where this worker
    /// holds an artifact, and **neither key at all** where it does not. §3.1
    /// fixes the absence as an absent key and not a `null`, and §4.1 says the
    /// absence states something — this worker holds no artifact the hub's
    /// manifest is about.
    ///
    /// A worker holding one cannot know whether the hub still serves it, so it
    /// sends what §3.1 requires of the case where it does and lets the hub
    /// decide: a report that turns out to be against a stale artifact is ignored
    /// there, and the answer carries the current one.
    fn body(&self) -> Value {
        let mut body = json!({
            "protocol": PROTOCOL,
            "compiler": compose_core::codegen::COMPILER_VERSION,
            "runtime": self.joining.runtime,
            "claims": self.joining.claims,
        });
        if let (Some(hash), Some(env_ok), Some(object)) = (
            self.joining.artifact.as_ref(),
            self.joining.env_ok.as_ref(),
            body.as_object_mut(),
        ) {
            object.insert("artifact_hash".to_string(), json!(hash));
            object.insert("env_ok".to_string(), json!(env_ok));
        }
        body
    }
}

/// §2's bounded exponential backoff: one second, doubling, capped at thirty.
///
/// It covers **transport failures and nothing else** — "a join that never
/// *completed* … which says nothing about whether this worker belongs here". A
/// refusal is terminal and never reaches this.
pub(crate) struct Backoff {
    next: Duration,
}

impl Backoff {
    pub(crate) const fn new() -> Self {
        Self {
            next: Duration::from_secs(1),
        }
    }

    /// Say why, wait out the current interval, and double it.
    ///
    /// The reason goes to standard error, because a worker retrying against a
    /// hub that is not answering is the one state where silence and health look
    /// alike from outside — and the whole of what an operator can do about it is
    /// read what the socket said.
    pub(crate) fn wait(&mut self, reason: &str) {
        note(&format!(
            "the hub did not answer ({reason}); trying again in {:?}",
            self.next
        ));
        std::thread::sleep(self.next);
        self.next = (self.next * 2).min(Duration::from_secs(30));
    }

    /// Back to one second: something answered.
    pub(crate) const fn reset(&mut self) {
        self.next = Duration::from_secs(1);
    }
}

/// The Bun this worker executes the artifact under (§4.2).
///
/// **Bun only.** That is the scoped exception §4.2 makes to PRD resolved q18's
/// Node fallback, and the scope is the point: the fallback is about a generated
/// project somebody runs by hand, and this is the surface *around* the artifact.
/// A worker on Node is refused at join by the handshake triple (§4.1), so
/// finding Node here and using it would only move the refusal later.
fn bun() -> Result<PathBuf, String> {
    let mut candidates = vec![PathBuf::from("bun")];
    if let Some(installed) = std::env::var_os("BUN_INSTALL") {
        candidates.push(PathBuf::from(installed).join("bin/bun"));
    }
    if let Some(home) = std::env::var_os("HOME") {
        candidates.push(PathBuf::from(home).join(".bun/bin/bun"));
    }
    for candidate in candidates {
        if version(&candidate).is_some() {
            return Ok(candidate);
        }
    }
    Err(
        "no `bun` was found: a worker executes the generated artifact under Bun \
         (docs/distributed.md §4.2), which is a scoped exception to the Node fallback a \
         generated project otherwise runs under. Install Bun, or run this placement on a \
         machine that has it"
            .to_string(),
    )
}

/// `"<name> <version>"`, as §3.1 wants `runtime` written.
fn runtime_line(bun: &Path) -> Result<String, String> {
    version(bun).map_or_else(
        || {
            Err(format!(
                "`{} --version` answered nothing, so this worker cannot say which runtime it \
                 executes the artifact under (docs/distributed.md §3.1)",
                bun.display()
            ))
        },
        |version| Ok(format!("bun {version}")),
    )
}

/// What a program answers `--version` with, trimmed.
fn version(program: &Path) -> Option<String> {
    let produced = Command::new(program)
        .arg("--version")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !produced.status.success() {
        return None;
    }
    let said = String::from_utf8_lossy(&produced.stdout).trim().to_string();
    (!said.is_empty()).then_some(said)
}

/// Where a worker keeps its artifacts when nothing says otherwise.
fn default_data_dir() -> PathBuf {
    std::env::var_os("HOME").map_or_else(
        || PathBuf::from(".agent-compose/worker"),
        |home| PathBuf::from(home).join(".agent-compose/worker"),
    )
}

/// Say something on standard error, best effort.
///
/// A worker's standard **output** is left alone: nothing here writes an answer
/// to it, and a node's own diagnostics are inherited on to this process's
/// standard error (see [`node`]).
fn note(said: &str) {
    use std::io::Write;
    let mut stream = std::io::stderr();
    let _ = writeln!(stream, "{said}");
    let _ = stream.flush();
}

/// Report that the worker could not run, or would not go on, and exit `2`.
fn fail(reason: &str) -> ExitCode {
    let mut stream = std::io::stderr();
    use std::io::Write;
    let _ = writeln!(stream, "error: {reason}");
    let _ = stream.flush();
    ExitCode::from(UNUSABLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// §2's schedule, exactly: one second, doubling, capped at thirty.
    #[test]
    fn the_backoff_starts_at_a_second_doubles_and_stops_at_thirty() {
        let mut backoff = Backoff::new();
        assert_eq!(backoff.next, Duration::from_secs(1));
        for expected in [2u64, 4, 8, 16, 30, 30] {
            backoff.next = (backoff.next * 2).min(Duration::from_secs(30));
            assert_eq!(backoff.next, Duration::from_secs(expected));
        }
        backoff.reset();
        assert_eq!(backoff.next, Duration::from_secs(1));
    }
}
