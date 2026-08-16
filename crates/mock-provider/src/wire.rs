//! What a surface hands back to the connection: a status, headers, a body, and
//! how long to wait first.
//!
//! Both provider surfaces answer in the same four parts, so the server's
//! connection loop knows nothing about either API. The one shape that is not a
//! response is [`Answer::Close`], which is how a scripted timeout is delivered:
//! there is no HTTP status for "the provider never answered".

use std::collections::BTreeMap;

use serde_json::Value;

use crate::control::{Delay, HARNESS_HEADER};

/// What to send back.
#[derive(Clone, Debug)]
pub(crate) enum Answer {
    /// A JSON response.
    Respond(Response),
    /// No response at all: wait, then close the connection.
    ///
    /// A client whose request timeout is shorter than the wait sees the timeout
    /// it is being tested for. The close is what keeps a client with no timeout
    /// from hanging until the test suite is killed.
    Close(Delay),
}

/// A JSON response, before it becomes bytes.
#[derive(Clone, Debug)]
pub(crate) struct Response {
    pub(crate) status: u16,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) body: Value,
    pub(crate) delay: Delay,
}

impl Response {
    /// A response with no extra headers and no wait.
    pub(crate) fn new(status: u16, body: Value) -> Self {
        Self {
            status,
            headers: BTreeMap::new(),
            body,
            delay: Delay::none(),
        }
    }

    /// Add a header.
    pub(crate) fn header(mut self, name: &str, value: impl Into<String>) -> Self {
        self.headers.insert(name.to_string(), value.into());
        self
    }

    /// Mark this as an answer the *harness* produced rather than one a script
    /// asked for, and say which kind. See [`HARNESS_HEADER`].
    pub(crate) fn harness(self, kind: &str) -> Self {
        self.header(HARNESS_HEADER, kind)
    }

    /// Wait this long before answering.
    pub(crate) fn after(mut self, delay: Delay) -> Self {
        self.delay = delay;
        self
    }

    /// Finish it.
    pub(crate) fn answer(self) -> Answer {
        Answer::Respond(self)
    }
}

pub(crate) use crate::control::{
    HARNESS_STATUS, REFUSED_INVALID as INVALID, REFUSED_MISMATCH as MISMATCH,
    REFUSED_UNSCRIPTED as UNSCRIPTED,
};
