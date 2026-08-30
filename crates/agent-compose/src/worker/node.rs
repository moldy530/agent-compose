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

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};

use serde_json::{Value, json};

use super::wire::{Answer, Hub};
use super::{Sessions, Stop};

/// One dispatch, executed.
///
/// Answers `Ok(())` when the dispatch was settled — or answered `409`, which is
/// a dispatch this worker no longer owns and is equally over (§3.4) — and
/// `Err(Stop)` when the worker itself must wind down.
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
            return sender.result(&json!({
                "dispatch_id": id,
                "error": { "name": "WorkerRunnerUnusable", "message": reason },
            }));
        }
    };

    let stdout = child.stdout.take().expect("stdout is piped");
    let mut result: Option<Value> = None;
    let mut unreadable: Option<String> = None;
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
                if let Err(stop) =
                    sender.effects(&json!({ "dispatch_id": id, "effects": [record] }))
                {
                    abandoned = Some(stop);
                    break;
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

    let settling = match (result, unreadable) {
        (Some(body), None) => body,
        // §3.4's other half: a dispatch that produced no result is an attempt
        // that failed, and the hub is told so rather than left holding an
        // unsettled dispatch until its liveness window closes (§6.3).
        (_, Some(reason)) => json!({
            "dispatch_id": id,
            "error": { "name": "WorkerRunnerUnreadable", "message": reason },
        }),
        (None, None) => {
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
    sender.result(&settling)
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
    fn effects(&mut self, body: &Value) -> Result<(), Stop> {
        let mut backoff = super::Backoff::new();
        loop {
            let session = self.sessions.current()?;
            match self.hub.effects(&session.id, body) {
                Answer::Said(said) if said.status == 204 => return Ok(()),
                Answer::Said(said) if said.status == 410 => {
                    self.sessions.renew(session.generation)?;
                }
                Answer::Said(said) if said.status == 401 => {
                    return Err(Stop::Refused(format!(
                        "this hub refused the join token at `/workers/effects`: {}",
                        said.detail()
                    )));
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
                    if (400..500).contains(&said.status) {
                        return Err(Stop::Refused(format!(
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
    fn result(&mut self, body: &Value) -> Result<(), Stop> {
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
                    return Err(Stop::Refused(format!(
                        "this hub refused the join token at `/workers/result`: {}",
                        said.detail()
                    )));
                }
                Answer::Said(said) => {
                    if (400..500).contains(&said.status) {
                        return Err(Stop::Refused(format!(
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
