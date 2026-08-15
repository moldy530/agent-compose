//! Error policy in the IR (grammar 9).
//!
//! One type serves all three levels the chain declares — a `flow:` node's
//! `policy:` override, a node's own keys, and the composition's `defaults:`
//! (Decisions D20, D60). Resolving them against each other is a later pass's
//! job; the IR records where each was written, and that no level was defaulted
//! on the way through: an absent key is absent in the artifact, so the value a
//! run uses stays derivable from the four levels rather than baked in at one of
//! them.

use serde::Serialize;

use crate::ast::common::{ControlTarget, Duration};
use crate::diag::{Span, Spanned};

/// A `retry`/`timeout`/`on_error` triple (grammar 9.3).
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Policy {
    /// `retry:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub retry: Option<Retry>,
    /// `timeout:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<Duration>>,
    /// `on_error:`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub on_error: Option<OnError>,
}

impl Policy {
    /// Whether none of the three fields was declared.
    #[must_use]
    pub const fn is_empty(&self) -> bool {
        self.retry.is_none() && self.timeout.is_none() && self.on_error.is_none()
    }
}

/// A `retry:` block (grammar 9.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Retry {
    /// Additional attempts after the first, 1..=10.
    pub max: i64,
    /// Initial delay.
    pub backoff: Spanned<Duration>,
    /// Exponential factor, at least 1; absent means the default, `2.0`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub multiplier: Option<f64>,
    /// Cap on a single delay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_backoff: Option<Spanned<Duration>>,
    /// Full jitter; absent means the default, `true`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub jitter: Option<bool>,
    /// The block's own span.
    pub span: Span,
}

/// The `on_error:` strategy applied once retries are exhausted (grammar 9.2,
/// Decision D21).
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "strategy", rename_all = "snake_case")]
pub enum OnError {
    /// Abort the execution with the node's error (the built-in default).
    Fail {
        /// Where the keyword was written.
        span: Span,
    },
    /// Produce no output and write nothing; routing then proceeds through
    /// grammar 7.3 with one substitution (Decision D97).
    Skip {
        /// Where the keyword was written.
        span: Span,
    },
    /// Transfer control to another node of the same flow, or to `end`. A
    /// node's own key only (Decision D103).
    Fallback {
        /// The target, which the flow must declare — or `end`.
        target: Spanned<ControlTarget>,
        /// Where the whole `on_error:` value was written.
        span: Span,
    },
}
