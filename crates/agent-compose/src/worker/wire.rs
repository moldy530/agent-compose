//! The five routes of `docs/distributed.md` §3, as a worker speaks them.
//!
//! One thin layer over the HTTP client, and it exists for one reason: **every
//! status this protocol gives a route is a decision the worker has to make, and
//! none of them is an exception.** So nothing here answers with a `Result` whose
//! error is "the request failed" — it answers with what the hub *said*, and
//! [`super`] is where §3's table is turned into behaviour.
//!
//! The one thing that is decided here is the difference §3.1 draws between a
//! **refusal** and a **transport failure**: a connection that was refused, a
//! socket that closed, a `5xx` — "which is a transport failure and says nothing
//! about whether this worker belongs here" — is [`Answer::Unreachable`], and the
//! bounded backoff of §2 covers it. Everything the hub answered with a status is
//! an [`Answer::Said`], however unwelcome.

use std::time::Duration;

use serde_json::Value;

/// What one request to the hub produced.
pub(crate) enum Answer {
    /// The hub answered. The status is what §3 decides on.
    Said(Said),
    /// Nothing answered: a connection refused, a socket closed, a timeout.
    ///
    /// §3.1's other case, and the only one the backoff of §2 is for.
    Unreachable(String),
}

/// One answer from the hub.
pub(crate) struct Said {
    pub(crate) status: u16,
    pub(crate) body: Vec<u8>,
}

impl Said {
    /// The body as JSON, or `Value::Null` where it is not.
    pub(crate) fn json(&self) -> Value {
        serde_json::from_slice(&self.body).unwrap_or(Value::Null)
    }

    /// The body as a sentence a person can read, for a refusal this worker
    /// echoes on its way out (§3.1).
    pub(crate) fn detail(&self) -> String {
        let said = self.json();
        if let Some(error) = said.get("error").and_then(Value::as_str) {
            return error.to_string();
        }
        let text = String::from_utf8_lossy(&self.body).trim().to_string();
        if text.is_empty() {
            format!("the hub answered {} and said nothing", self.status)
        } else {
            text
        }
    }
}

/// A worker's client for one hub.
pub(crate) struct Hub {
    agent: ureq::Agent,
    base: String,
    token: String,
}

impl Hub {
    /// A client for `base`, presenting `token` on every request (§3).
    pub(crate) fn new(base: &str, token: &str) -> Self {
        // No global timeout: a poll is *meant* to be held (§2), and the hold
        // is the hub's to end. What is bounded is the connect and the wait for
        // response **headers**, which is what a hub that has gone away looks
        // like — and the poll overrides the second, because 25 seconds of
        // silence is what a hold with no work for this session is.
        let config = ureq::Agent::config_builder()
            // Every status is this protocol's to decide, so none of them is an
            // error here: §3 gives `400`, `401`, `403`, `409` and `410` five
            // different behaviours, and a client that raised on all of them
            // would flatten the one distinction the whole of §5 rests on.
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .user_agent(format!(
                "agent-compose/{}",
                compose_core::codegen::COMPILER_VERSION
            ))
            .build();
        Self {
            agent: config.into(),
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    /// `POST /workers/join` (§3.1).
    pub(crate) fn join(&self, body: &Value) -> Answer {
        self.post("/workers/join", None, body, Duration::from_secs(60))
    }

    /// `GET /workers/poll` (§3.2), held for up to `hold`.
    ///
    /// The receive timeout is the hold plus [`POLL_MARGIN`]: a client that gave
    /// up at exactly the hold would race the `204` the hub is about to send, and
    /// every such race is a re-join for nothing. `hold` is what the *caller*
    /// believes this hub holds for, which §2 makes a number a hub may configure
    /// — so it is a parameter rather than a constant here, and [`super::Hold`]
    /// is what learns it.
    pub(crate) fn poll(&self, session: &str, hold: Duration) -> Answer {
        let request = self
            .agent
            .get(self.url("/workers/poll"))
            .config()
            .timeout_recv_response(Some(hold + POLL_MARGIN))
            .build()
            .header("authorization", format!("Bearer {}", self.token))
            .header("x-worker-session", session);
        answer(request.call())
    }

    /// `POST /workers/effects` (§3.3).
    pub(crate) fn effects(&self, session: &str, body: &Value) -> Answer {
        self.post(
            "/workers/effects",
            Some(session),
            body,
            Duration::from_secs(60),
        )
    }

    /// `POST /workers/result` (§3.4).
    pub(crate) fn result(&self, session: &str, body: &Value) -> Answer {
        self.post(
            "/workers/result",
            Some(session),
            body,
            Duration::from_secs(60),
        )
    }

    /// `GET /workers/artifact/{hash}` (§3.5).
    ///
    /// The session is presented where the worker has one, which §3.5 asks for
    /// and does not require: "a worker SHOULD present its session when it has
    /// one; a hub MUST NOT require it". The transfer is the one request here
    /// with a body worth waiting on, so it is given room.
    pub(crate) fn artifact(&self, hash: &str, session: Option<&str>) -> Answer {
        let mut request = self
            .agent
            .get(self.url(&format!("/workers/artifact/{hash}")))
            .config()
            .timeout_recv_response(Some(Duration::from_secs(300)))
            .build()
            .header("authorization", format!("Bearer {}", self.token));
        if let Some(session) = session {
            request = request.header("x-worker-session", session);
        }
        answer(request.call())
    }

    fn post(&self, path: &str, session: Option<&str>, body: &Value, wait: Duration) -> Answer {
        let mut request = self
            .agent
            .post(self.url(path))
            .config()
            .timeout_recv_response(Some(wait))
            .build()
            .header("authorization", format!("Bearer {}", self.token))
            .header("content-type", "application/json");
        if let Some(session) = session {
            request = request.header("x-worker-session", session);
        }
        answer(request.send(serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string())))
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }
}

/// How much longer than the hold a poll waits for the answer to arrive.
///
/// See [`Hub::poll`], and [`super::Hold`] for the hold itself.
pub(crate) const POLL_MARGIN: Duration = Duration::from_secs(30);

/// How large an answer this worker will read into memory.
///
/// One ceiling for every route, and it is the **artifact's** size rather than a
/// message's, because two of the five bodies grow without a bound of their own:
///
/// * the artifact (§3.5), which is read whole before it is verified (§4 step 2),
///   so it is in memory either way;
/// * a poll's `effect_history` (§3.2, §7.2), which is every effect the journal
///   holds at this node instance — a long tool loop, or a node already through
///   two `retry:` attempts, and each record carries the canonical request and
///   the whole of the outcome.
///
/// The second is why a message-sized limit is the wrong one. A poll answer this
/// worker cannot read is a dispatch the hub has already **claimed** to this
/// session: it never reaches [`super`]'s `200` arm, so it is never executed and
/// never settled, and the node waits out its whole `timeout:` chain over a
/// payload the worker declined rather than over anything that went wrong. A
/// ceiling is still here rather than none, because a hub answering with an
/// endless stream would otherwise be a worker that ran out of memory — and where
/// it is reached the diagnostic names it, so what happened is readable.
const ANSWER_LIMIT: u64 = 256 * 1024 * 1024;

fn answer(sent: Result<ureq::http::Response<ureq::Body>, ureq::Error>) -> Answer {
    match sent {
        Ok(response) => {
            let status = response.status().as_u16();
            let mut body = response.into_body();
            match body.with_config().limit(ANSWER_LIMIT).read_to_vec() {
                Ok(bytes) => Answer::Said(Said {
                    status,
                    body: bytes,
                }),
                // A body that did not arrive is a transfer that failed, whatever
                // the status line said: the answer is incomplete, and this
                // protocol has no partial reading of one.
                Err(error) => Answer::Unreachable(format!(
                    "the hub answered {status} and the body did not arrive (this worker reads at \
                     most {} MiB of one): {error}",
                    ANSWER_LIMIT / (1024 * 1024)
                )),
            }
        }
        Err(error) => Answer::Unreachable(error.to_string()),
    }
}
