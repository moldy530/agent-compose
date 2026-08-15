//! Triggers in the IR (grammar 13).
//!
//! The IR's trigger table holds exactly the **declared** triggers. Implicit
//! `manual` invocation — every flow is runnable from the CLI whether or not a
//! trigger names it — contributes no entry here, and every rule that quantifies
//! over triggers quantifies over this table (Decision D64).
//!
//! `schedule` and `event` are reserved grammar: parsed, type-checked, and
//! carried into the artifact, executing as no-ops until M3 (grammar 15).

use serde::Serialize;

use crate::ast::common::{Address, Cel, Duration, Ident};
use crate::ast::trigger::{Respond, TriggerMethod};
use crate::diag::{Span, Spanned};

use super::binding::Bindings;

/// One declared trigger (grammar 13.1).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Trigger {
    /// The trigger name, repeated from the key it sits under so the entry names
    /// itself, with the span of the name.
    pub name: Spanned<Ident>,
    /// `flow:` — the execution's entry module.
    pub flow: Spanned<Address>,
    /// `session_key:` — CEL over `payload`, supplying session identity for
    /// session-scoped stores and history (grammar 11.3). On a `manual` trigger,
    /// absent means the default, `"payload.session"` (grammar 13.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_key: Option<Spanned<Cel>>,
    /// `description:`
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// The trigger object's own span.
    pub span: Span,
    /// The type-specific half.
    #[serde(flatten)]
    pub kind: TriggerKind,
}

/// The four trigger types (grammar 13), tagged by `type`.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
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

/// An `http` trigger (grammar 13.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HttpTrigger {
    /// `path:` — absent means the default, `/triggers/<name>`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<Spanned<String>>,
    /// `method:` — absent means the default, `POST`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub method: Option<TriggerMethod>,
    /// `input:` — flow input field to CEL over `payload`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Bindings>,
    /// `respond:` — absent means the default, `async`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub respond: Option<Respond>,
    /// `timeout:` — the response budget; `sync` only, where absent means the
    /// default, `60s` (Decision D81).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<Spanned<Duration>>,
    /// `callback:` — a completion webhook; `async` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback: Option<Spanned<Cel>>,
}

/// A `schedule` trigger — reserved grammar (grammar 13.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ScheduleTrigger {
    /// `cron:` — a 5-field POSIX cron expression.
    pub cron: Spanned<String>,
    /// `timezone:` — an IANA name; absent means the default, `UTC`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timezone: Option<Spanned<String>>,
    /// `input:` — CEL over `payload.scheduled_at` / `payload.trigger`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Bindings>,
}

/// An `event` trigger — reserved grammar (grammar 13.5).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EventTrigger {
    /// `source:` — a logical name the active target's `event_sources:` binds
    /// to infrastructure (grammar 14.3).
    pub source: Spanned<Ident>,
    /// `input:` — CEL over the event payload.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub input: Option<Bindings>,
    /// `dedupe_key:` — absent means the default, `payload.id`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<Spanned<Cel>>,
}
