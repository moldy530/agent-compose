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
//! # A child that dies is an attempt that failed
//!
//! A runner that exits without a result line has not answered, and §3.4 takes
//! "its output, or its failure": the failure this posts names the exit status,
//! which is what a `retry:` on the hub then spends an attempt on (§7.3).
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

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use super::wire::{Answer, Hub};
use super::{Sessions, Stop, note};

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
pub(crate) fn execute(
    hub: &Hub,
    sessions: &Sessions,
    tree: &Path,
    bun: &Path,
    runner: &str,
    dispatch: &Value,
) -> Result<(), Stop> {
    let id = dispatch
        .get("dispatch_id")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut sender = Sender {
        hub,
        sessions,
        dispatch: &id,
    };

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
    if let Some(stop) = abandoned {
        // The worker itself is winding down — a redeployment, a refusal — so the
        // dispatch is not settled here: the hub ended it, or is about to.
        return Err(stop);
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
    let mut child = command.spawn().map_err(|error| {
        format!(
            "cannot start `{} {runner}` in `{}`: {error}",
            bun.display(),
            tree.display()
        )
    })?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    let payload = serde_json::to_vec(dispatch).unwrap_or_else(|_| b"{}".to_vec());
    let written = stdin.write_all(&payload).and_then(|()| stdin.flush());
    // Closed, because the runner reads to end of file: a handle left open is a
    // dispatch that never starts.
    drop(stdin);
    if let Err(error) = written {
        let _ = child.kill();
        let _ = child.wait();
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
