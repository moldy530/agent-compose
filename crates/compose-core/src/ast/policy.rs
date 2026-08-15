//! Error policy (grammar 9): `retry`, `timeout`, `on_error`.
//!
//! The same three fields appear at three levels of the resolution chain — a
//! `flow:` node's `policy:` override, a node's own keys, and the composition's
//! `defaults:` — so one type covers all three (Decision D20, D60). Resolving
//! them against each other is the validator's job; the parser records where
//! each was written.

use crate::diag::{Span, Spanned};

use super::common::{ControlTarget, Duration};

/// A `retry:` block (grammar 9.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Retry {
    /// Additional attempts after the first, 1..=10.
    pub max: Option<Spanned<i64>>,
    /// Initial delay.
    pub backoff: Option<Spanned<Duration>>,
    /// Exponential factor, at least 1; defaults to 2.0.
    pub multiplier: Option<Spanned<f64>>,
    /// Cap on a single delay.
    pub max_backoff: Option<Spanned<Duration>>,
    /// Full jitter; defaults to true.
    pub jitter: Option<Spanned<bool>>,
    /// The block's own span.
    pub span: Span,
}

/// The `on_error:` strategy applied once retries are exhausted (grammar 9.2,
/// Decision D21).
#[derive(Clone, Debug, PartialEq)]
pub enum OnError {
    /// Abort the execution with the node's error (the built-in default).
    Fail,
    /// Produce no output and write nothing; unconditional and `else` edges
    /// still fire.
    Skip,
    /// Transfer control to another node of the same flow, or to `end`.
    Fallback(Spanned<ControlTarget>),
}

/// A `retry`/`timeout`/`on_error` triple (grammar 9.3).
///
/// Used for the `defaults:` section, for a `flow:` node's `policy:` override,
/// and — flattened onto the node object — for a node's own policy.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PolicyBlock {
    /// `retry:`.
    pub retry: Option<Spanned<Retry>>,
    /// `timeout:`.
    pub timeout: Option<Spanned<Duration>>,
    /// `on_error:`.
    pub on_error: Option<Spanned<OnError>>,
}

impl PolicyBlock {
    /// Whether none of the three fields was declared.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.retry.is_none() && self.timeout.is_none() && self.on_error.is_none()
    }
}
