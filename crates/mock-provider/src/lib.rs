//! A scripted stand-in for the LLM providers `agent-compose` talks to.
//!
//! This is the M1 acceptance harness (PRD §7 M1, CLAUDE.md *Validation
//! strategy*): a local server implementing the provider wire surface, with a
//! control plane that enqueues scripted outcomes per model, so compiled graphs
//! execute end to end in CI — triggers, routing, cycles, fan-out joins, store
//! ops, and model failover — with **no API keys and no network**.
//!
//! Two things make it an acceptance rig rather than a stub:
//!
//! 1. **It is strict.** A request a real provider would refuse is refused here,
//!    in the provider's own error shape, and the refusal is recorded with a
//!    per-field verdict. The server's job is catching bad codegen output, so
//!    leniency is a bug — a mock that accepts a malformed tool loop lets a graph
//!    pass CI and fail on the first live call.
//! 2. **It is deterministic.** No randomness, no wall clock, ids derived from
//!    the request's own arrival sequence, and a per-model FIFO of scripted
//!    outcomes. An unscripted call is refused loudly rather than answered with a
//!    default, because a default is how a test starts passing for the wrong
//!    reason.
//!
//! # The two surfaces
//!
//! Grammar 12.1's six provider kinds funnel into two HTTP protocols:
//!
//! | kind | surface |
//! |---|---|
//! | `anthropic` | [`anthropic`]: `POST /v1/messages` |
//! | `openai`, `openai_compatible` | [`openai`]: `POST /v1/chat/completions` |
//! | `azure_openai` | [`openai`], reached at the Azure routes |
//! | `bedrock`, `vertex` | **out of scope in v0** — reached through cloud SDKs, not an HTTP endpoint the spec points at (grammar 12.1); see `WIRE-NOTES.md` |
//!
//! # Using it from a test
//!
//! ```no_run
//! use mock_provider::{MockProvider, Outcome, Script};
//! use serde_json::json;
//!
//! let provider = MockProvider::start().expect("the harness binds a port");
//! provider.enqueue(Script::new(
//!     "claude-sonnet-4-6",
//!     Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
//! ));
//!
//! // …point the compiled graph's provider `base_url` at `provider.base_url()`
//! // and run it…
//!
//! let calls = provider.requests();
//! assert!(calls.iter().all(mock_provider::RecordedRequest::is_valid));
//! assert!(provider.snapshot().is_drained(), "every scripted answer was used");
//! ```
//!
//! # The control plane
//!
//! The same store is reachable over HTTP, which is how a TypeScript harness will
//! drive it once one exists:
//!
//! | route | what it does |
//! |---|---|
//! | `POST /_mock/enqueue` | one [`Script`], or a list of them |
//! | `GET /_mock/requests` | every recorded request, in arrival order |
//! | `GET /_mock/state` | queue depths and counts |
//! | `POST /_mock/reset` | discard both, answering with what was discarded |
//!
//! # What is *not* here
//!
//! Streaming, embeddings, the Files/Batch APIs, and the Bedrock and Vertex SDK
//! surfaces. A request for any of them is refused by name rather than answered
//! approximately: an acceptance harness that guesses is worse than one that
//! says it does not know.

pub mod client;
mod control;
mod strict;
mod wire;

mod anthropic;
mod openai;
mod server;

use std::io;
use std::net::SocketAddr;
use std::sync::Arc;

pub use client::{Client, Request, Response};
pub use control::{
    CREATED, Delay, Failure, HARNESS_HEADER, HARNESS_STATUS, Outcome, REFUSED_INVALID,
    REFUSED_MISMATCH, REFUSED_UNSCRIPTED, RecordedRequest, Reply, ReplyBody, Script, Snapshot,
    Store, StructuredOutput, Surface, ToolCall, Usage, ValidationFailure, Verdict,
};
pub use server::serve;

/// The loopback address the harness binds by default: an ephemeral port, so any
/// number of tests can run at once.
const EPHEMERAL: ([u8; 4], u16) = ([127, 0, 0, 1], 0);

/// A running mock provider, and the handle a test drives it with.
///
/// Blocking on purpose. The acceptance suite it serves shells out to
/// `agent-compose build` and to `node`, so the work around it is blocking work;
/// a handle that demanded an async test macro would spread `async` through a
/// suite that has no other use for it. The runtime is inside — call it from an
/// ordinary `#[test]`, and not from inside another runtime's thread.
///
/// Dropping it stops the server.
pub struct MockProvider {
    address: SocketAddr,
    store: Arc<Store>,
    runtime: Option<tokio::runtime::Runtime>,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
}

impl MockProvider {
    /// Bind an ephemeral loopback port and start serving.
    ///
    /// # Errors
    ///
    /// If the runtime cannot be built or the port cannot be bound.
    pub fn start() -> io::Result<Self> {
        Self::start_on(SocketAddr::from(EPHEMERAL))
    }

    /// Bind a chosen address and start serving.
    ///
    /// # Errors
    ///
    /// If the runtime cannot be built or the address cannot be bound.
    pub fn start_on(address: SocketAddr) -> io::Result<Self> {
        // Two workers: one to accept while another is inside a scripted delay.
        // A fan-out test's concurrency lives in the connections, not here.
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_io()
            .enable_time()
            .build()?;
        let listener =
            runtime.block_on(async move { tokio::net::TcpListener::bind(address).await })?;
        let address = server::address(&listener)?;
        let store = Arc::new(Store::new());
        let (shutdown, shutdown_signal) = tokio::sync::oneshot::channel();
        runtime.spawn(serve(listener, Arc::clone(&store), shutdown_signal));
        Ok(Self {
            address,
            store,
            runtime: Some(runtime),
            shutdown: Some(shutdown),
        })
    }

    /// Where it is listening.
    #[must_use]
    pub fn address(&self) -> SocketAddr {
        self.address
    }

    /// The base URL a provider definition's `base_url:` should resolve to.
    ///
    /// This is how the harness reaches a compiled graph: every acceptance
    /// fixture declares `base_url: ${…}` on its providers (grammar 12.1), and
    /// the run is given this string for that variable — the spec never changes
    /// between a real run and a scripted one.
    #[must_use]
    pub fn base_url(&self) -> String {
        format!("http://{}", self.address)
    }

    /// A client bound to this server, sharing its runtime.
    #[must_use]
    pub fn client(&self) -> Client {
        Client::attached(
            self.address,
            self.runtime
                .as_ref()
                .expect("the runtime outlives every handle method")
                .handle()
                .clone(),
        )
    }

    /// Script one outcome.
    pub fn enqueue(&self, script: Script) {
        self.store.enqueue(script);
    }

    /// Script several, in order.
    pub fn enqueue_all(&self, scripts: impl IntoIterator<Item = Script>) {
        for script in scripts {
            self.enqueue(script);
        }
    }

    /// Every request the server has seen, in arrival order.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.store.requests()
    }

    /// The requests that failed validation — the ones a codegen bug produced.
    #[must_use]
    pub fn invalid(&self) -> Vec<RecordedRequest> {
        self.store
            .requests()
            .into_iter()
            .filter(|request| !request.is_valid())
            .collect()
    }

    /// Queue depths and counts.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        self.store.snapshot()
    }

    /// Forget every request and discard every queued outcome, answering with
    /// what was discarded.
    pub fn reset(&self) -> Snapshot {
        self.store.reset()
    }

    /// The store itself, for a caller that wants to hand it to something else.
    #[must_use]
    pub fn store(&self) -> Arc<Store> {
        Arc::clone(&self.store)
    }
}

impl Drop for MockProvider {
    fn drop(&mut self) {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        // Shut down without waiting: the accept loop and any connection inside a
        // scripted delay are ordinary tasks, and a test that has finished with
        // the server should not block on either of them.
        if let Some(runtime) = self.runtime.take() {
            runtime.shutdown_background();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// The handle binds a real port, answers on it, and stops when it is
    /// dropped — the three things every test in the suite depends on.
    #[test]
    fn the_handle_serves_and_stops() {
        let provider = MockProvider::start().expect("a port");
        assert!(provider.address().ip().is_loopback());
        assert_eq!(
            provider.base_url(),
            format!("http://{}", provider.address())
        );

        let client = provider.client();
        let state = client
            .get("/_mock/state")
            .expect("the control plane answers");
        assert_eq!(state.status, 200);
        assert_eq!(state.json()["requests"], 0);

        let address = provider.address();
        drop(provider);

        // The runtime is gone, so a fresh client of its own gets a refusal
        // rather than an answer. (A retry loop would be flakier than one shot:
        // `shutdown_background` closes the listener before it returns.)
        let orphan = Client::new(format!("http://{address}")).expect("a client");
        assert!(
            orphan.get("/_mock/state").is_err(),
            "the server stopped with its handle"
        );
    }

    /// The in-process API and the HTTP control plane are the same store: a test
    /// may script in Rust and read back over the wire, or the other way round.
    #[test]
    fn the_two_control_surfaces_share_one_store() {
        let provider = MockProvider::start().expect("a port");
        provider.enqueue(Script::new("model.fast", Outcome::text("in process")));

        let client = provider.client();
        let state = client
            .get("/_mock/state")
            .expect("the control plane answers");
        assert_eq!(state.json()["queues"]["model.fast"], 1);

        client
            .post_json(
                "/_mock/enqueue",
                &json!({
                    "model": "model.smart",
                    "outcome": { "reply": { "body": { "text": "over http" } } },
                }),
            )
            .expect("the control plane accepts a script");
        assert_eq!(provider.snapshot().queues["model.smart"], 1);

        let discarded = provider.reset();
        assert_eq!(discarded.queues.len(), 2);
        assert!(provider.snapshot().is_drained());
    }
}
