//! Triggers: what causes an execution to exist (grammar 13).
//!
//! All four types are parsed in full. `manual` and `http` are active in v0;
//! `schedule` and `event` are reserved grammar — parsed, type-checked, and
//! carried into the IR, executing as no-ops until M3 (grammar 15, D46).

use crate::diag::{Span, Spanned};

use super::binding::Bindings;
use super::common::{Address, Cel, Duration, Ident};

/// The `triggers:` section.
#[derive(Clone, Debug, PartialEq)]
pub struct TriggersSection {
    /// The declared triggers, in declaration order.
    pub triggers: Vec<Trigger>,
    /// The section's own span.
    pub span: Span,
}

/// One trigger (grammar 13.1).
#[derive(Clone, Debug, PartialEq)]
pub struct Trigger {
    /// The trigger name.
    pub name: Spanned<Ident>,
    /// `flow:` — the execution's entry module.
    pub flow: Option<Spanned<Address>>,
    /// `session_key:` — CEL over `payload`, supplying session identity.
    pub session_key: Option<Spanned<Cel>>,
    /// `description:`
    pub description: Option<Spanned<String>>,
    /// The type-specific half. `None` when `type:` was missing or unknown, in
    /// which case the rest of the entry is not parsed — which keys are legal
    /// depends on the type.
    pub kind: Option<TriggerKind>,
    /// The trigger object's own span.
    pub span: Span,
}

/// The four trigger types (grammar 13).
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerKind {
    /// CLI/SDK invocation. Declares no `input:` bindings (Decision D44).
    Manual,
    /// The graph as an HTTP endpoint (grammar 13.3).
    Http(HttpTrigger),
    /// A cron schedule — reserved grammar (grammar 13.4).
    Schedule(ScheduleTrigger),
    /// An event-source consumer — reserved grammar (grammar 13.5).
    Event(EventTrigger),
}

impl TriggerKind {
    /// The `type:` keyword that selects this kind.
    #[must_use]
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Manual => "manual",
            Self::Http(_) => "http",
            Self::Schedule(_) => "schedule",
            Self::Event(_) => "event",
        }
    }
}

/// Every trigger type keyword, in the order grammar 13.1 lists them.
pub const TRIGGER_TYPES: &[&str] = &["manual", "http", "schedule", "event"];

/// The methods an `http` trigger's start route may use (grammar 13.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TriggerMethod {
    /// `POST` (the default).
    Post,
    /// `PUT`
    Put,
    /// `GET`
    Get,
}

impl TriggerMethod {
    /// Every method, in the order grammar 13.3 lists them.
    pub const ALL: &'static [Self] = &[Self::Post, Self::Put, Self::Get];

    /// The method's spelling.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Post => "POST",
            Self::Put => "PUT",
            Self::Get => "GET",
        }
    }
}

/// How an `http` trigger responds (grammar 13.3, Decision D45).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Respond {
    /// Block and return the flow's outputs; requires an interrupt-free flow.
    Sync,
    /// Return an execution id immediately (the default).
    Async,
}

impl Respond {
    /// The keyword that names this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sync => "sync",
            Self::Async => "async",
        }
    }
}

/// An `http` trigger (grammar 13.3).
#[derive(Clone, Debug, PartialEq)]
pub struct HttpTrigger {
    /// `path:` — starts with `/`, no whitespace; defaults to
    /// `/triggers/<name>`.
    pub path: Option<Spanned<String>>,
    /// `method:` — defaults to `POST`.
    pub method: Option<Spanned<TriggerMethod>>,
    /// `input:` — flow input field to CEL over `payload`.
    pub input: Option<Bindings>,
    /// `respond:` — defaults to `async`.
    pub respond: Option<Spanned<Respond>>,
    /// `timeout:` — defaults to `60s`; meaningful for `respond: sync`.
    pub timeout: Option<Spanned<Duration>>,
    /// `callback:` — a completion webhook; `async` only.
    pub callback: Option<Spanned<Cel>>,
}

/// A `schedule` trigger — reserved grammar (grammar 13.4).
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleTrigger {
    /// `cron:` — required; a 5-field POSIX cron expression.
    pub cron: Option<Spanned<String>>,
    /// `timezone:` — an IANA name; defaults to `UTC`.
    pub timezone: Option<Spanned<String>>,
    /// `input:` — CEL over `payload.scheduled_at` / `payload.trigger`.
    pub input: Option<Bindings>,
}

/// An `event` trigger — reserved grammar (grammar 13.5).
#[derive(Clone, Debug, PartialEq)]
pub struct EventTrigger {
    /// `source:` — required; a logical name bound by the deploy layer.
    pub source: Option<Spanned<Ident>>,
    /// `input:` — CEL over the event payload.
    pub input: Option<Bindings>,
    /// `dedupe_key:` — defaults to `payload.id`.
    pub dedupe_key: Option<Spanned<Cel>>,
}
