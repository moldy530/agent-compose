//! Executing one dispatch: the Bun child, its NDJSON, and what goes back over
//! the wire (`docs/distributed.md` §3.2, §3.3, §3.4).
//!
//! The division of labour is the whole of this module. `src/worker-node.ts`
//! *runs* the node and knows nothing about HTTP; this reads what it says and
//! speaks §3 for it:
//!
//! ```text
//! stdin   the dispatch, verbatim as `/workers/poll` answered it
//! stdout  {"type":"effect", …}  → batched and POSTed to /workers/effects
//!         {"type":"result",  …} → POSTed to /workers/result
//!         {"type":"finally", "running": …} → read here, never sent: whether
//!                                  a `finally:` node is running in the runner
//! stderr  inherited by this process's own, so a node's diagnostics are the
//!         worker's diagnostics
//! ```
//!
//! # Why an effect goes out as it arrives
//!
//! §3.3: "a worker SHOULD send a batch as soon as an effect completes rather
//! than accumulating until the node ends, because an effect that never reached
//! the hub is an effect the replay of §7 cannot skip". A batch is therefore what
//! the reader has in hand when it is not blocked — one line, usually — rather
//! than a window on a timer.
//!
//! # Why a batch answered `410` is re-sent and never dropped
//!
//! Also §3.3, and the reason is the one the document gives: "a worker that
//! discarded the batch instead would hand the redispatch of §7.2 an
//! `effect_history` short of the frontier, and the node would re-issue an effect
//! the journal was owed". So [`Sender`] renews the session and sends the same
//! batch again.
//!
//! # A pause is a result like any other
//!
//! §3.4 gives a dispatch three endings and the third is a **paused** result: a
//! `human:` node reached on a worker settles its dispatch with the wait it
//! opened, and the hub plants that wait on its board (PRD resolved q46). This
//! module needs no rule for it, and that is the point of the division above —
//! the runner decides what a result *is* and this posts whatever it wrote, so a
//! paused line is one more shape of the last line and takes §3.4's own statuses.
//! What the worker does next is what it does after any settlement: go back to
//! polling, free to be dispatched other work while a person thinks.
//!
//! # A child that dies is an attempt that failed
//!
//! A runner that exits without a result line has not answered, and §3.4 takes
//! "its output, its failure, or the pause it stopped at": the failure this posts
//! names the exit status, which is what a `retry:` on the hub then spends an
//! attempt on (§7.3).
//!
//! # A body this hub will not take costs the dispatch, not the worker
//!
//! §3.3 and §3.4 each give their route a closed table, and every status in the
//! two that is not `204` is handled by name here. What is left over is a `4xx`
//! **outside** both tables — a `413` from a hub or an intermediary with a
//! smaller ceiling than this worker's, a `431`, a proxy's own refusal — and it
//! is the one place a refusal is *not* read the way §3.1 reads a refused join.
//! §3.1's terminality is about the join, where "a second join would be refused
//! identically" is a statement about this worker's right to be here at all; a
//! body one hub would not take says nothing of the kind. So [`Unsent`] separates
//! the two: a status this worker cannot act on fails **this dispatch**, under
//! the node's own `retry:`/`on_error:` chain like any other failed attempt, and
//! the placement keeps its worker. Ending the process instead would leave the
//! placement with none — and the replacement would reach the same record and die
//! the same way, so one oversized effect would be a mesh that never runs that
//! node again.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU8, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};

use super::wire::{Answer, Hub};
use super::{Sessions, Stop, note};

/// How long a runner is given to stop on its own after a cancel's `SIGTERM`,
/// or a stop's `SIGUSR2`, before its whole process group is killed (§3.2, PRD
/// resolved q75).
///
/// The runner turns `SIGTERM` into its execution's cancel and `SIGUSR2` into a
/// failure of the placed node ([`Halt`]), and either aborts every signal its
/// activities read — a model call's `fetch`, a spawned command, a harness run
/// — so an ordinary node stops in well under this. The kill is for one that
/// does not: a host function that never reads its signal, or a child that
/// ignores the signal. Inside the ten seconds PRD resolved q75 sets as the
/// bar, with room for the poll that carried the cancel.
///
/// **Not while a `finally:` node is running in the runner**, which the runner
/// reports on its stdout. A cancel ends the instances it reaches, and an
/// instance that ends runs its finally node *whole* — "a cancel stops none of
/// it, and nothing it begins" (grammar §7.9, Decision D160) — under that node's
/// own `timeout:`. So the grace is held for as long as one runs, and runs again
/// from the moment the last of them settles.
pub(crate) const CANCEL_GRACE: Duration = Duration::from_secs(5);

/// How often a held grace looks again at whether the runner's finally work has
/// ended.
const FINALLY_POLL: Duration = Duration::from_millis(50);

/// How the hub ended a dispatch this worker holds (§3.2): the two lists a
/// poll's `200` carries, and the two signals a runner reads them by.
///
/// **They are not one thing**, and the runner is the one that has to know
/// which: a cancel is the execution's, and the instances the placed node began
/// there — an attached `flow.*` — end `cancelled`, their finally nodes reading
/// that outcome; a stop is the hub having stopped waiting — the node's
/// `timeout:` ran out on the hub, its execution ended around it, or the session
/// it was handed under ended ([`Running::orphan`]) — which is nobody's cancel,
/// and those instances end `failed`, as they do on the hub when the node runs
/// out of time there (grammar §7.9, PRD resolved q75, q76).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Halt {
    /// The `cancel` list: the runner is sent `SIGTERM`, and the result is
    /// `status: "cancelled"` (§3.4).
    Cancel,
    /// The `stop` list, or a session a `410` ended: the runner is sent
    /// `SIGUSR2`, and the result is whatever the node ended on — which the hub
    /// refuses `409` and only frees the slot by.
    Stop,
}

impl Halt {
    /// How this halt is recorded on a [`Handle`]: never `0`, which is none.
    const fn mark(self) -> u8 {
        match self {
            Self::Cancel => 1,
            Self::Stop => 2,
        }
    }

    /// The halt a [`Handle`]'s mark records, if any.
    const fn of(mark: u8) -> Option<Self> {
        match mark {
            1 => Some(Self::Cancel),
            2 => Some(Self::Stop),
            _ => None,
        }
    }
}

/// One dispatch this worker is running, as a cancel or a stop reaches it.
#[derive(Default)]
pub(crate) struct Handle {
    /// Which of this worker's sessions it was handed under ([`super::Held`]'s
    /// generation): once a newer one has been joined, the hub has ended the
    /// session this dispatch belonged to, and with it the dispatch
    /// ([`Running::orphan`]).
    generation: u64,
    /// The runner's pid, once it has one; `0` before it is spawned.
    pid: AtomicU32,
    /// How the hub ended this dispatch, if it has ([`Halt::mark`]): the first
    /// word is the one it ends by.
    halted: AtomicU8,
    /// Whether the runner says a `finally:` node is running in it — the work a
    /// cancel's grace does not cut off ([`CANCEL_GRACE`]).
    finally: AtomicBool,
}

impl Handle {
    /// How the hub ended this dispatch, if it has.
    fn halted(&self) -> Option<Halt> {
        Halt::of(self.halted.load(Ordering::SeqCst))
    }
}

/// The dispatches this worker is running, by `dispatch_id` — shared between the
/// poll thread, which reads the `cancel` and `stop` lists, and each dispatch's
/// own thread.
#[derive(Clone, Default)]
pub(crate) struct Running(Arc<Mutex<HashMap<String, Arc<Handle>>>>);

impl Running {
    /// Put one dispatch on the books, answering its handle.
    ///
    /// Called by the **poll thread**, for the dispatch a poll answer hands over,
    /// before that answer's `cancel` and `stop` lists are acted on (§3.2, PRD
    /// resolved q75). A list may name that very dispatch — a cancel or a stop
    /// that reached it while the hub was assembling the answer, and which the
    /// hub names again on every answer until its result arrives — and a name
    /// not yet on the books finds nothing to stop ([`Running::halt`]), so the
    /// stop would wait a poll. So would the next poll's, if the hold waited for
    /// the dispatch's own thread to be scheduled.
    pub(crate) fn hold(&self, id: &str, generation: u64) -> Arc<Handle> {
        let handle = Arc::new(Handle {
            generation,
            ..Handle::default()
        });
        self.0
            .lock()
            .expect("the running table is not poisoned")
            .insert(id.to_string(), Arc::clone(&handle));
        handle
    }

    /// Take one dispatch off the books.
    fn release(&self, id: &str) {
        self.0
            .lock()
            .expect("the running table is not poisoned")
            .remove(id);
    }

    /// Whether `id` is still running here.
    fn holds(&self, id: &str) -> bool {
        self.0
            .lock()
            .expect("the running table is not poisoned")
            .contains_key(id)
    }

    /// Stop every dispatch this worker took under a session **older** than
    /// `generation` (§5, §6.3, PRD resolved q79).
    ///
    /// A `410` is the hub saying it no longer knows the session — it was
    /// restarted, or its liveness sweep gave up on this worker — and either way
    /// it has superseded what that session held: a result for one is refused
    /// `409`. Left running, such a runner is work nothing will read, and —
    /// since the new session holds nothing by the hub's count — a slot the hub
    /// hands new work beside: a worker at `max_running: 1` would be running
    /// two. So each is **stopped** ([`Halt::Stop`]) — never cancelled: nobody
    /// cancelled its execution, and what it began here ends `failed` — by
    /// `SIGUSR2`, the grace — held while a `finally:` node runs — and the group
    /// killed after.
    ///
    /// **A stop even where the hub was cancelling it.** A `410` says the
    /// session ended, not why, so a dispatch whose execution a restarted hub
    /// is completing a cancel of is stopped here like any other — what it
    /// began ends `failed`, not `cancelled` — which is the limitation grammar
    /// D160 and `docs/distributed.md` §5 name, accepted by the owner as one
    /// protocol `3` does not close (PRD resolved q75 as amended 2026-11-06).
    pub(crate) fn orphan(&self, generation: u64) {
        let older: Vec<String> = self
            .0
            .lock()
            .expect("the running table is not poisoned")
            .iter()
            .filter(|(_, handle)| handle.generation < generation)
            .map(|(id, _)| id.clone())
            .collect();
        for id in older {
            self.halt(&id, Halt::Stop);
        }
    }

    /// Wait until nothing taken under a session older than `generation` is
    /// still running here: the slot a dispatch handed to the new session takes
    /// is one the hub counted free, and it is free once the old session's
    /// runners have stopped ([`Running::orphan`]) — not before.
    fn drained_before(&self, generation: u64) {
        let older = || {
            self.0
                .lock()
                .expect("the running table is not poisoned")
                .values()
                .any(|handle| handle.generation < generation)
        };
        while older() {
            std::thread::sleep(FINALLY_POLL);
        }
    }

    /// Act on the hub ending `id` (§3.2) — a name on a poll's `cancel` list
    /// or its `stop` list, or a session a `410` ended ([`Running::orphan`]):
    /// mark it, send its runner the signal [`Halt`] names, and kill its
    /// process group if it is still running after [`CANCEL_GRACE`] — a grace
    /// held while the runner reports a `finally:` node running, and given
    /// again once it reports none. A dispatch this worker does not hold —
    /// already settled, or never handed over — is nothing to stop, and one
    /// already halted keeps the first word it was given.
    pub(crate) fn halt(&self, id: &str, how: Halt) {
        let Some(handle) = self
            .0
            .lock()
            .expect("the running table is not poisoned")
            .get(id)
            .cloned()
        else {
            return;
        };
        if handle
            .halted
            .compare_exchange(0, how.mark(), Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return;
        }
        let pid = handle.pid.load(Ordering::SeqCst);
        if pid != 0 {
            ask(pid, how);
        }
        let running = self.clone();
        let id = id.to_string();
        std::thread::spawn(move || {
            std::thread::sleep(CANCEL_GRACE);
            while running.holds(&id) {
                if !handle.finally.load(Ordering::SeqCst) {
                    kill_group(handle.pid.load(Ordering::SeqCst));
                    return;
                }
                // A finally node is how the instance the cancel ended ends, and
                // runs whole: the grace starts over once it has settled.
                while running.holds(&id) && handle.finally.load(Ordering::SeqCst) {
                    std::thread::sleep(FINALLY_POLL);
                }
                std::thread::sleep(CANCEL_GRACE);
            }
        });
    }
}

/// Ask a runner to stop, the way [`Halt`] says: `SIGTERM`, which it turns into
/// its execution's cancel, or `SIGUSR2`, which it turns into a failure of the
/// placed node and cancels nothing.
#[cfg(unix)]
fn ask(pid: u32, how: Halt) {
    let signal = match how {
        Halt::Cancel => libc::SIGTERM,
        Halt::Stop => libc::SIGUSR2,
    };
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: a pid this worker spawned and has not reaped — the handle's
        // pid is cleared the moment the runner is waited on.
        unsafe { libc::kill(pid, signal) };
    }
}

#[cfg(not(unix))]
fn ask(_pid: u32, _how: Halt) {}

/// Kill a runner and every process in its group: what a cancel does to a
/// runner that did not stop within [`CANCEL_GRACE`].
#[cfg(unix)]
fn kill_group(pid: u32) {
    if pid == 0 {
        return;
    }
    if let Ok(pid) = libc::pid_t::try_from(pid) {
        // SAFETY: the group this worker spawned the runner into, which is its
        // own pid; `ESRCH` for a group already gone is ignored.
        unsafe { libc::kill(-pid, libc::SIGKILL) };
    }
}

#[cfg(not(unix))]
fn kill_group(_pid: u32) {}

/// Why a `POST` this dispatch made did not go through.
///
/// The two are different in exactly one way and it is the only way that matters:
/// how much they cost. See the module docs' last section.
enum Unsent {
    /// This **worker** is winding down — a redeployment, or a credential the hub
    /// refused. Nothing more is sent, on this dispatch or any other.
    Stop(Stop),
    /// This **dispatch** is over: a status §3.3 or §3.4 does not give the route,
    /// which no re-send improves and which says nothing about the worker.
    Rejected(String),
}

impl From<Stop> for Unsent {
    fn from(stop: Stop) -> Self {
        Self::Stop(stop)
    }
}

/// One dispatch, executed.
///
/// Answers `Ok(())` when this worker is done with the dispatch, whichever way it
/// became done: settled, answered `409` — a dispatch this worker no longer owns,
/// and equally over (§3.4) — or left with the hub after a result it would not
/// take ([`settle`]). `Err(Stop)` is the worker itself winding down, and nothing
/// else.
///
/// `handle` is the dispatch on the books, put there by the poll thread before
/// the answer that handed it over had its `cancel` and `stop` lists read
/// ([`Running::hold`]): a cancel or a stop in that same answer has already
/// marked it, and its runner is never started. Its generation is the session
/// that poll was made under. A dispatch
/// handed to a session this worker joined while runners of an older one are
/// still stopping waits for them before its own runner starts
/// ([`Running::drained_before`]) — a wait inside the node's `timeout:` chain,
/// which runs from dispatch (§6.5).
#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch's whole context, passed through once"
)]
pub(crate) fn execute(
    hub: &Hub,
    sessions: &Sessions,
    tree: &Path,
    bun: &Path,
    runner: &str,
    dispatch: &Value,
    running: &Running,
    handle: &Handle,
) -> Result<(), Stop> {
    let id = dispatch
        .get("dispatch_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    running.drained_before(handle.generation);
    let ended = execute_held(hub, sessions, tree, bun, runner, dispatch, &id, handle);
    running.release(&id);
    ended
}

/// [`execute`], with the dispatch on the books a cancel reads.
#[allow(
    clippy::too_many_arguments,
    reason = "one dispatch's whole context, passed through once"
)]
fn execute_held(
    hub: &Hub,
    sessions: &Sessions,
    tree: &Path,
    bun: &Path,
    runner: &str,
    dispatch: &Value,
    id: &str,
    handle: &Handle,
) -> Result<(), Stop> {
    let id = id.to_string();
    let mut sender = Sender {
        hub,
        sessions,
        dispatch: &id,
    };
    // **A dispatch the hub cancelled is settled `cancelled`** (§3.4, PRD
    // resolved q75), whatever its runner managed to say on its way out: the
    // result is the worker's statement that it stopped the work. What a
    // `finally:` node that ran in the runner raised travels beside it — a cancel
    // is what starts most of them, and §3.4 carries `finally_error` on every
    // ending.
    let cancelled = |said: Option<&Value>| {
        let mut body = json!({
            "dispatch_id": id,
            "status": "cancelled",
        });
        if let Some(finally_error) = said
            .and_then(|said| said.get("finally_error"))
            .filter(|held| held.is_string())
        {
            body["finally_error"] = finally_error.clone();
        }
        body
    };
    match handle.halted() {
        Some(Halt::Cancel) => return settle(&mut sender, &cancelled(None)),
        // Stopped before its runner started — named on the `stop` list of the
        // answer that handed it over, or its session ended while it waited for
        // an older one's runners ([`Running::drained_before`]). The hub has
        // already superseded it, so this result is refused `409`; what it is
        // for is the slot the hub keeps counted until a result arrives.
        Some(Halt::Stop) => {
            return settle(
                &mut sender,
                &json!({
                    "dispatch_id": id,
                    "error": {
                        "name": "DispatchStopped",
                        "message": "the hub stopped waiting for this dispatch before its runner started, so none was (docs/distributed.md §3.2)",
                    },
                }),
            );
        }
        None => {}
    }

    let child = spawn(bun, tree, runner, dispatch);
    let mut child = match child {
        Ok(child) => child,
        // The runner could not be started at all, which is this machine's
        // problem and not the graph's — but it is still an attempt that did not
        // answer, and the hub is owed a result for the dispatch it handed over.
        Err(reason) => {
            return settle(
                &mut sender,
                &json!({
                    "dispatch_id": id,
                    "error": { "name": "WorkerRunnerUnusable", "message": reason },
                }),
            );
        }
    };

    handle.pid.store(child.id(), Ordering::SeqCst);
    // A cancel or a stop that arrived between the hold and the spawn found no
    // pid to signal, so it is signalled here.
    if let Some(how) = handle.halted() {
        ask(child.id(), how);
    }
    let stdout = child.stdout.take().expect("stdout is piped");
    let mut result: Option<Value> = None;
    let mut unreadable: Option<String> = None;
    // Set where a batch met a status §3.3 does not give the route: the records
    // are not in the journal and nothing this worker does will put them there,
    // so the attempt fails rather than settling with an output whose effects the
    // replay of §7.2 could not skip.
    let mut rejected: Option<String> = None;
    // Set where this worker stops reading before the runner stops writing, which
    // is the one case the child has to be **killed** rather than waited on: a
    // process writing into a pipe nobody drains blocks for ever, and a `wait`
    // over it would take this thread with it.
    let mut abandoned: Option<Stop> = None;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else {
            break;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(said) = serde_json::from_str::<Value>(&line) else {
            unreadable = Some(format!(
                "the node runner wrote a line that is not JSON: {line}"
            ));
            break;
        };
        match said.get("type").and_then(Value::as_str) {
            Some("effect") => {
                let record = said.get("effect").cloned().unwrap_or(Value::Null);
                match sender.effects(&json!({ "dispatch_id": id, "effects": [record] })) {
                    Ok(()) => {}
                    Err(Unsent::Stop(stop)) => {
                        abandoned = Some(stop);
                        break;
                    }
                    Err(Unsent::Rejected(detail)) => {
                        rejected = Some(detail);
                        break;
                    }
                }
            }
            Some("result") => {
                let mut body = said;
                if let Some(object) = body.as_object_mut() {
                    object.remove("type");
                    object.insert("dispatch_id".to_string(), Value::String(id.clone()));
                }
                result = Some(body);
            }
            // The runner's account of its finally work, for a cancel's grace
            // ([`CANCEL_GRACE`]); the hub is never sent it.
            Some("finally") => {
                let running = said.get("running").and_then(Value::as_bool) == Some(true);
                handle.finally.store(running, Ordering::SeqCst);
            }
            _ => {
                unreadable = Some(format!(
                    "the node runner wrote a line this worker does not understand: {line}"
                ));
                break;
            }
        }
    }
    if result.is_none() {
        // Either the runner ended without answering — in which case this is a
        // no-op — or this worker stopped reading it, and then the kill is what
        // makes the `wait` below return.
        let _ = child.kill();
    }
    let status = child.wait();
    // Reaped, so its pid is no longer this worker's to signal: off the handle
    // a cancel reads — a dispatch stays on the books while its result is
    // posted, and an orphaned session's stop ([`Running::orphan`]) can reach
    // it then — and off the books a stop signal reads.
    handle.pid.store(0, Ordering::SeqCst);
    super::stop::ended(child.id());
    if let Some(stop) = abandoned {
        // The worker itself is winding down — a redeployment, a refusal — so the
        // dispatch is not settled here: the hub ended it, or is about to.
        return Err(stop);
    }

    // A stopped dispatch is settled with whatever its runner ended on — the
    // failure the stop raised in it, or a result it reached first — which the
    // hub refuses `409` and frees the slot by (§3.4).
    if handle.halted() == Some(Halt::Cancel) {
        return settle(&mut sender, &cancelled(result.as_ref()));
    }
    let settling = match (result, unreadable, rejected) {
        // The refusal first, and ahead of a result the runner may already have
        // written: a batch the journal never took is a frontier short of where
        // this attempt actually got to, so settling with the node's own output
        // would hand the execution a value whose effects nothing recorded.
        (_, _, Some(detail)) => json!({
            "dispatch_id": id,
            "error": { "name": "WorkerEffectsRefused", "message": detail },
        }),
        (Some(body), None, None) => body,
        // §3.4's other half: a dispatch that produced no result is an attempt
        // that failed, and the hub is told so rather than left holding an
        // unsettled dispatch until its liveness window closes (§6.3).
        (_, Some(reason), None) => json!({
            "dispatch_id": id,
            "error": { "name": "WorkerRunnerUnreadable", "message": reason },
        }),
        (None, None, None) => {
            let said = match status {
                Ok(status) => status.code().map_or_else(
                    || "was killed by a signal".to_string(),
                    |code| format!("exited {code}"),
                ),
                Err(error) => format!("could not be waited on: {error}"),
            };
            json!({
                "dispatch_id": id,
                "error": {
                    "name": "WorkerRunnerFailed",
                    "message": format!("the node runner {said} without answering (docs/distributed.md §3.4)"),
                },
            })
        }
    };
    settle(&mut sender, &settling)
}

/// Post the result that ends this dispatch, and say what a hub that would not
/// take it costs.
///
/// A `409`, a `410` and a `204` are all `Ok` by the time [`Sender::result`]
/// answers — §3.4's three meanings, each acted on there. What is left is a
/// status that route does not give, and there is no second channel to report it
/// through: the one thing the hub is owed for this dispatch is the very message
/// it just refused. So the dispatch is left unsettled and the worker goes on,
/// which costs that node the rest of its `timeout:` chain (§6.5) — bounded, and
/// invisible unless this says so, which is why it is said. It is the same
/// posture `super::settled` takes to a dispatch thread that panicked, and for
/// the same reason: one dispatch that could not be finished is not a reason to
/// take the placement's worker away.
fn settle(sender: &mut Sender<'_>, body: &Value) -> Result<(), Stop> {
    match sender.result(body) {
        Ok(()) => Ok(()),
        Err(Unsent::Stop(stop)) => Err(stop),
        Err(Unsent::Rejected(detail)) => {
            note(&format!(
                "{detail}. This dispatch is left unsettled and this worker goes on: the hub holds \
                 it until that node's `timeout:` fires (docs/distributed.md §6.5)"
            ));
            Ok(())
        }
    }
}

/// Start `bun <runner>` in the materialised tree, with the dispatch on its
/// standard input.
fn spawn(bun: &Path, tree: &Path, runner: &str, dispatch: &Value) -> Result<Child, String> {
    let mut command = Command::new(bun);
    command
        .arg(runner)
        .current_dir(tree)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        // Inherited: a node's own diagnostics are this worker's, which is what
        // makes `agent-compose worker` a process an operator can read.
        .stderr(Stdio::inherit());
    // A **process group of its own**, so a cancel the runner does not answer in
    // time takes everything left in it with it (§3.2, PRD resolved q75). The
    // commands an `exec:` node spawned are in groups of their own, which the
    // runner stops itself as the cancel reaches them — `SIGTERM`, then `SIGKILL`
    // two seconds later, well inside this worker's grace — and kills on its way
    // out if it exits first (`src/runtime.ts`'s `stopExecGroup`). A group the
    // terminal no longer reaches, so a stop signal to this worker is delivered
    // to it by the worker ([`super::stop`]).
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|error| {
        format!(
            "cannot start `{} {runner}` in `{}`: {error}",
            bun.display(),
            tree.display()
        )
    })?;
    // On the books a stop signal to this worker reads: a group of its own is
    // one the terminal's `Ctrl-C` no longer reaches, so the worker delivers it
    // ([`super::stop`]).
    super::stop::started(child.id());
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let payload = serde_json::to_vec(dispatch).unwrap_or_else(|_| b"{}".to_vec());
    let written = stdin.write_all(&payload).and_then(|()| stdin.flush());
    // Closed, because the runner reads to end of file: a handle left open is a
    // dispatch that never starts.
    drop(stdin);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
        super::stop::ended(child.id());
        return Err(format!(
            "cannot hand the dispatch to the node runner: {error}"
        ));
    }
    Ok(child)
}

/// The two `POST`s a dispatch makes, with §3's status rules on them.
struct Sender<'a> {
    hub: &'a Hub,
    sessions: &'a Sessions,
    dispatch: &'a str,
}

impl Sender<'_> {
    /// `POST /workers/effects` (§3.3): `204` is done, `410` is re-sent under a
    /// new session, and a transport failure is retried under §2's backoff.
    fn effects(&mut self, body: &Value) -> Result<(), Unsent> {
        let mut backoff = super::Backoff::new();
        loop {
            let session = self.sessions.current()?;
            match self.hub.effects(&session.id, body) {
                Answer::Said(said) if said.status == 204 => return Ok(()),
                Answer::Said(said) if said.status == 410 => {
                    self.sessions.renew(session.generation)?;
                }
                Answer::Said(said) if said.status == 401 => {
                    return Err(Unsent::Stop(Stop::Refused(format!(
                        "this hub refused the join token at `/workers/effects`: {}",
                        said.detail()
                    ))));
                }
                // A `409` here is a dispatch the hub cannot attribute, which is
                // the one answer this route gives that is not about the batch:
                // the effects are the execution's and the hub has moved past
                // them, so re-sending would loop for ever.
                Answer::Said(said) if said.status == 409 => {
                    return Ok(());
                }
                Answer::Said(said) => {
                    // A `4xx` this worker cannot act on: the batch is not one
                    // the hub will take however often it is sent, and holding
                    // the node hostage to it would stop the dispatch answering.
                    // It costs the **dispatch** and not this process — see the
                    // module docs — because a body one hub would not take says
                    // nothing about whether this worker belongs here.
                    if (400..500).contains(&said.status) {
                        return Err(Unsent::Rejected(format!(
                            "this hub refused an effect batch of `{}` with {}: {}",
                            self.dispatch,
                            said.status,
                            said.detail()
                        )));
                    }
                    backoff.wait(&format!(
                        "an effect batch answered {}: {}",
                        said.status,
                        said.detail()
                    ));
                }
                Answer::Unreachable(reason) => backoff.wait(&reason),
            }
        }
    }

    /// `POST /workers/result` (§3.4).
    ///
    /// The three answers are three different things and a worker MUST NOT treat
    /// them alike: `204` settles, `410` says the hub does not know this session
    /// and the result is **still owed**, and `409` says the hub knows this worker
    /// and does not want this result — so it is discarded and nothing is
    /// re-posted.
    fn result(&mut self, body: &Value) -> Result<(), Unsent> {
        let mut backoff = super::Backoff::new();
        loop {
            let session = self.sessions.current()?;
            match self.hub.result(&session.id, body) {
                Answer::Said(said) if said.status == 204 => return Ok(()),
                Answer::Said(said) if said.status == 409 => return Ok(()),
                Answer::Said(said) if said.status == 410 => {
                    self.sessions.renew(session.generation)?;
                }
                Answer::Said(said) if said.status == 401 => {
                    return Err(Unsent::Stop(Stop::Refused(format!(
                        "this hub refused the join token at `/workers/result`: {}",
                        said.detail()
                    ))));
                }
                Answer::Said(said) => {
                    // Outside §3.4's four statuses, and so outside what any
                    // re-post can improve. It costs this dispatch — see
                    // [`settle`] — and not the placement's worker.
                    if (400..500).contains(&said.status) {
                        return Err(Unsent::Rejected(format!(
                            "this hub refused the result of `{}` with {}: {}",
                            self.dispatch,
                            said.status,
                            said.detail()
                        )));
                    }
                    backoff.wait(&format!(
                        "a result answered {}: {}",
                        said.status,
                        said.detail()
                    ));
                }
                Answer::Unreachable(reason) => backoff.wait(&reason),
            }
        }
    }
}
