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

use crate::ast::common::{Address, Cel, Duration, EnvRef, Ident};
use crate::ast::trigger::{HmacAlgorithm, Respond, SignatureEncoding, TriggerMethod};
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
///
/// The variants differ in size for the reason the AST's do, and are read the
/// same way — built once per trigger, matched by reference from then on — so
/// boxing the `http` one would add indirection to every consumer and buy
/// nothing (see [`ast::trigger::TriggerKind`](crate::ast::trigger::TriggerKind)).
#[allow(clippy::large_enum_variant)]
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
    /// `auth:` — how an inbound call is authenticated, and with it the resume
    /// and status routes of every execution this trigger starts. Absent means
    /// those three routes are open (grammar 13.3, PRD resolved q32).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<InboundAuth>,
    /// `callback_auth:` — how a delivery identifies itself to its receiver.
    /// Absent is the documented test posture: the deployment signs nothing and
    /// may POST anywhere (grammar 13.3, PRD resolved q33).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_auth: Option<CallbackAuth>,
    /// `callback_allow:` — the URL patterns a callback may point at; present
    /// wherever [`Self::callback_auth`] is (grammar 13.3, PRD resolved q33).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub callback_allow: Option<Vec<Spanned<String>>>,
}

/// The inbound `auth:` of an `http` trigger, **defaults applied**
/// (grammar 13.3, PRD resolved q32).
///
/// Unlike `path:`, `method:` and `respond:`, which record what the author wrote
/// and leave the default to whoever reads them, a declared scheme lands here
/// complete: every parameter that decides whether a credential verifies —
/// the header it arrives in, the digest, the encoding, the prefix — carries the
/// value the trigger actually enforces. A verifier is the wrong place to
/// re-derive a default, because getting one wrong there does not fail the build,
/// it accepts the wrong request.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "scheme", rename_all = "snake_case")]
pub enum InboundAuth {
    /// `bearer:` — a static secret compared, in constant time, against a named
    /// header.
    Bearer(BearerAuth),
    /// `hmac:` — a signature over the raw request body.
    Hmac(HmacAuth),
}

/// A static-token scheme, inbound or outbound (grammar 13.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BearerAuth {
    /// `token:` — the expected credential, an environment reference that
    /// survives unresolved into the artifact (grammar 4.3).
    pub token: Spanned<EnvRef>,
    /// `header:` — resolved; the default is `Authorization`.
    ///
    /// Carries the author's capitalisation, and is **matched
    /// case-insensitively** on the way in: header names are case-insensitive by
    /// definition and HTTP/2 lowercases every one on the wire, so a verifier
    /// comparing the spelling would refuse every genuine call (grammar 13.3).
    /// Outbound the name is written as it stands.
    pub header: String,
    /// `prefix:` — resolved; the default is `Bearer `, trailing space included.
    pub prefix: String,
}

/// The inbound `hmac:` scheme, resolved (grammar 13.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct HmacAuth {
    /// `secret:` — the signing key, an environment reference that survives
    /// unresolved into the artifact (grammar 4.3).
    pub secret: Spanned<EnvRef>,
    /// `header:` — resolved; the default is `X-Signature`. Matched
    /// case-insensitively, for the reason [`BearerAuth::header`] is.
    pub header: String,
    /// `algorithm:` — resolved; the default is `sha256`.
    pub algorithm: HmacAlgorithm,
    /// `encoding:` — resolved; the default is `hex`.
    pub encoding: SignatureEncoding,
    /// `prefix:` — resolved; the default is the empty string.
    pub prefix: String,
}

/// The outbound `callback_auth:`, resolved (grammar 13.3, PRD resolved q33).
///
/// At least one of the two is present, and both together are legal.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CallbackAuth {
    /// `bearer:` — a static token on every delivery.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bearer: Option<BearerAuth>,
    /// `hmac:` — a signature over the delivered body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hmac: Option<CallbackHmac>,
}

/// The outbound `hmac:` scheme (grammar 13.3).
///
/// One field, because outbound signing is not configurable: the delivery is
/// signed with HMAC-SHA256, written in hex, and carried as
/// `X-AgentCompose-Signature: sha256=<hex>`. Those are the wire contract's
/// rather than the author's, so they are stated once in grammar 13.3 instead of
/// repeated per trigger here.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct CallbackHmac {
    /// `secret:` — the signing key, an environment reference that survives
    /// unresolved into the artifact (grammar 4.3).
    pub secret: Spanned<EnvRef>,
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
