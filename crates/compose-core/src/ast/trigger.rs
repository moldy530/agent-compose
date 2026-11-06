//! Triggers: what causes an execution to exist (grammar 13).
//!
//! All four types are parsed in full, and all four execute: `manual` from the
//! CLI, `http` through the routes `serve` mounts, and — PRD resolved q78,
//! Decision D162 — `schedule` from the scheduler `serve` runs and `event` from
//! the consumer of the source it names.

use crate::diag::{Span, Spanned};

use super::binding::Bindings;
use super::common::{Address, Cel, Duration, EnvRef, Ident};

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
    /// `concurrency:` — the intake groups this trigger's executions are
    /// admitted under (grammar 13.1, PRD resolved q77, Decision D163). Legal on
    /// `http`, `schedule` and `event` triggers; the parser refuses it on a
    /// `manual` one (`concurrency-on-manual`) and leaves this `None`.
    pub concurrency: Option<Concurrency>,
    /// The type-specific half. `None` when `type:` was missing or unknown, in
    /// which case the rest of the entry is not parsed — which keys are legal
    /// depends on the type.
    pub kind: Option<TriggerKind>,
    /// The trigger object's own span.
    pub span: Span,
}

/// The four trigger types (grammar 13).
///
/// The variants differ in size because the constructs do — an `http` trigger
/// carries a route, a response mode and two authentication blocks, a `manual`
/// one carries nothing. Each is built once per trigger and matched by reference
/// from then on, so boxing would add indirection to every consumer and buy
/// nothing (the same reading [`DefinitionBody`](super::definition::DefinitionBody)
/// takes).
#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug, PartialEq)]
pub enum TriggerKind {
    /// CLI/SDK invocation. Declares no `input:` bindings (Decision D44).
    Manual,
    /// The graph as an HTTP endpoint (grammar 13.3).
    Http(HttpTrigger),
    /// A cron schedule, which `serve` fires (grammar 13.4).
    Schedule(ScheduleTrigger),
    /// An event-source consumer, which `serve` runs (grammar 13.5).
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
    /// `auth:` — how an inbound call is authenticated; exactly one scheme.
    /// Absent means the route is open, and so are the resume and status routes
    /// of every execution this trigger starts (grammar 13.3, PRD resolved q32).
    pub auth: Option<Spanned<AuthScheme>>,
    /// `callback_auth:` — how a delivered callback identifies itself to its
    /// receiver; at least one of the two schemes, and both are legal together
    /// (grammar 13.3, PRD resolved q33).
    pub callback_auth: Option<Spanned<CallbackAuth>>,
    /// `callback_allow:` — the URL patterns a callback may point at; mandatory
    /// wherever `callback_auth:` is declared (grammar 13.3, PRD resolved q33).
    pub callback_allow: Option<CallbackAllow>,
    /// `dedupe_key:` — CEL over the request payload, to a string; no default,
    /// because an HTTP body has no id of its own: absent means no dedupe
    /// (grammar 13.3, PRD resolved q77 ruling a).
    pub dedupe_key: Option<Spanned<Cel>>,
    /// `dedupe_window:` — how long a key is held; defaults to
    /// [`DEFAULT_DEDUPE_WINDOW`]. Legal only beside a `dedupe_key:`.
    pub dedupe_window: Option<Spanned<Duration>>,
}

/// The window a dedupe key is held for when a trigger declares none: `24h`
/// (grammar 13.3, 13.5, PRD resolved q77 ruling a).
pub const DEFAULT_DEDUPE_WINDOW: &str = "24h";

/// The window [`DEFAULT_DEDUPE_WINDOW`] spells, in milliseconds.
pub const DEFAULT_DEDUPE_WINDOW_MS: u64 = 24 * 60 * 60 * 1000;

/// The `concurrency:` block (grammar 13.1, PRD resolved q77 ruling b, c).
///
/// A **group** is a lease held by its running executions: the arrivals whose
/// `key:` answers one value share one, `limit:` of them may hold it at once, and
/// `max_running:` bounds the trigger's executions across every group. What an
/// arrival that finds no free slot does is `on_conflict:`'s; `debounce:`
/// coalesces the arrivals for one key into one execution.
#[derive(Clone, Debug, PartialEq)]
pub struct Concurrency {
    /// `key:` — CEL over the kind's payload, to a string; absent means one
    /// group for the whole trigger.
    pub key: Option<Spanned<Cel>>,
    /// `limit:` — an integer of at least `1`; defaults to `1`.
    pub limit: Option<Spanned<i64>>,
    /// `on_conflict:` — defaults to [`OnConflict::Queue`].
    pub on_conflict: Option<Spanned<OnConflict>>,
    /// `debounce:` — a literal duration; absent means no coalescing.
    pub debounce: Option<Spanned<Duration>>,
    /// `max_running:` — **the trigger's** total across all its groups, an
    /// integer of at least `1`. Not a placement's `max_running:` (grammar 14.1,
    /// PRD resolved q79), which bounds a worker session's dispatches.
    pub max_running: Option<Spanned<i64>>,
    /// The block's own span.
    pub span: Span,
}

impl Concurrency {
    /// The keys the block may declare, in the order grammar 13.1 lists them.
    pub const KEYS: &'static [&'static str] =
        &["key", "limit", "on_conflict", "debounce", "max_running"];
}

/// What an arrival that finds no free slot in its group does (grammar 13.1,
/// PRD resolved q77 ruling b).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OnConflict {
    /// `queue` — the default: an intake row, started first-in first-out when a
    /// slot frees.
    Queue,
    /// `drop` — answered with the running execution's id, and nothing starts.
    Drop,
    /// `supersede` — the group's running executions are cancelled, and this one
    /// starts.
    Supersede,
}

impl OnConflict {
    /// Every mode, in the order grammar 13.1 lists them.
    pub const ALL: &'static [Self] = &[Self::Queue, Self::Drop, Self::Supersede];

    /// The keyword that names this mode.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Queue => "queue",
            Self::Drop => "drop",
            Self::Supersede => "supersede",
        }
    }
}

/// The inbound authentication schemes (grammar 13.3, PRD resolved q32).
///
/// One per `auth:` block: a request carries one credential, so the two are
/// alternatives rather than a set — the parser refuses a block declaring both,
/// as it refuses a `tool.*` declaring two implementation bindings (Decision
/// D25).
#[derive(Clone, Debug, PartialEq)]
pub enum AuthScheme {
    /// `bearer:` — a static secret compared against a named header.
    Bearer(BearerAuth),
    /// `hmac:` — a signature over the raw request body.
    Hmac(HmacAuth),
}

impl AuthScheme {
    /// The keys an `auth:` block may declare, in the order grammar 13.3 lists
    /// them.
    pub const KEYS: &'static [&'static str] = &["bearer", "hmac"];
}

/// A static-token scheme, inbound or outbound (grammar 13.3).
///
/// One shape serves both directions: inbound it names the header a caller's
/// token arrives in, outbound the header a delivery carries it in, and the
/// defaults are the same pair because it is the same convention.
#[derive(Clone, Debug, PartialEq)]
pub struct BearerAuth {
    /// `token:` — required; an `${ENV}` reference, never a literal.
    pub token: Option<Spanned<EnvRef>>,
    /// `header:` — defaults to [`BearerAuth::DEFAULT_HEADER`].
    pub header: Option<Spanned<String>>,
    /// `prefix:` — defaults to [`BearerAuth::DEFAULT_PREFIX`].
    pub prefix: Option<Spanned<String>>,
}

impl BearerAuth {
    /// The header a `bearer` scheme reads, or writes, when none is declared.
    pub const DEFAULT_HEADER: &'static str = "Authorization";
    /// The prefix that header's value carries when none is declared. The
    /// trailing space is part of it: the header reads `Bearer <token>`.
    pub const DEFAULT_PREFIX: &'static str = "Bearer ";
}

/// The inbound `hmac:` scheme: a signature over the raw request body
/// (grammar 13.3, PRD resolved q32).
#[derive(Clone, Debug, PartialEq)]
pub struct HmacAuth {
    /// `secret:` — required; an `${ENV}` reference, never a literal.
    pub secret: Option<Spanned<EnvRef>>,
    /// `header:` — defaults to [`HmacAuth::DEFAULT_HEADER`].
    pub header: Option<Spanned<String>>,
    /// `algorithm:` — defaults to [`HmacAlgorithm::DEFAULT`].
    pub algorithm: Option<Spanned<HmacAlgorithm>>,
    /// `encoding:` — defaults to [`SignatureEncoding::DEFAULT`].
    pub encoding: Option<Spanned<SignatureEncoding>>,
    /// `prefix:` — defaults to [`HmacAuth::DEFAULT_PREFIX`], the empty string.
    pub prefix: Option<Spanned<String>>,
}

impl HmacAuth {
    /// The header a signature arrives in when none is declared.
    pub const DEFAULT_HEADER: &'static str = "X-Signature";
    /// What precedes the signature in that header when nothing is declared:
    /// nothing. A vendor that writes `sha256=<hex>` declares `prefix:`.
    pub const DEFAULT_PREFIX: &'static str = "";
}

/// The digests an inbound `hmac:` scheme may verify with (grammar 13.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HmacAlgorithm {
    /// `sha1` — the legacy half of the GitHub-shaped family.
    Sha1,
    /// `sha256` — the default.
    Sha256,
    /// `sha512`
    Sha512,
}

impl HmacAlgorithm {
    /// Every algorithm, in the order grammar 13.3 lists them.
    pub const ALL: &'static [Self] = &[Self::Sha1, Self::Sha256, Self::Sha512];

    /// What an `hmac:` scheme verifies with when it declares nothing.
    pub const DEFAULT: Self = Self::Sha256;

    /// The keyword that names this algorithm.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sha1 => "sha1",
            Self::Sha256 => "sha256",
            Self::Sha512 => "sha512",
        }
    }
}

/// How a signature is written on the wire (grammar 13.3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SignatureEncoding {
    /// `hex` — the default.
    Hex,
    /// `base64`
    Base64,
}

impl SignatureEncoding {
    /// Every encoding, in the order grammar 13.3 lists them.
    pub const ALL: &'static [Self] = &[Self::Hex, Self::Base64];

    /// How a signature is encoded when the scheme declares nothing.
    pub const DEFAULT: Self = Self::Hex;

    /// The keyword that names this encoding.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Hex => "hex",
            Self::Base64 => "base64",
        }
    }
}

/// The outbound `callback_auth:` block (grammar 13.3, PRD resolved q33).
///
/// At least one scheme, and **both together are legal** — the asymmetry with
/// inbound `auth:` is the resolved question's own: a receiver that verifies a
/// signature and a receiver that checks a token are two receivers, and one
/// trigger may deliver to a receiver that does both.
#[derive(Clone, Debug, PartialEq)]
pub struct CallbackAuth {
    /// `bearer:` — a static token on every delivery.
    pub bearer: Option<BearerAuth>,
    /// `hmac:` — a signature over the delivered body.
    pub hmac: Option<CallbackHmac>,
}

impl CallbackAuth {
    /// The keys a `callback_auth:` block may declare, in the order grammar 13.3
    /// lists them.
    pub const KEYS: &'static [&'static str] = &["bearer", "hmac"];

    /// The namespace a delivery's own headers live in: `X-AgentCompose-Event`,
    /// `-Delivery`, `-Ordinal`, `-Timestamp`, and — signed — `-Signature`.
    ///
    /// Reserved against a `bearer:` scheme's `header:`, and only outbound: the
    /// names are normative for the receiver (grammar 13.3, Decision D127), so a
    /// token asked for under one of them collides with a value the delivery
    /// already writes.
    pub const DELIVERY_HEADER_PREFIX: &'static str = "X-AgentCompose-";

    /// The rest of what a delivery writes, outside its own namespace: the three
    /// fields a POST of a JSON body carries by construction.
    ///
    /// Reserved for the same reason and by the same rule (grammar 13.3,
    /// Decision D127). `Content-Type` is `application/json` on every delivery,
    /// `Content-Length` frames the body, and `Host` is the receiver the
    /// allowlist admitted — so a token asked for under one of them replaces a
    /// value the request cannot go without, or arrives joined to it, and the
    /// receiver answers 415, fails to decode the report, or never sees the
    /// request at all.
    pub const TRANSPORT_HEADERS: &'static [&'static str] =
        &["Content-Type", "Content-Length", "Host"];
}

/// The outbound `hmac:` scheme: one key, because the algorithm and the encoding
/// are the wire contract's rather than the author's (grammar 13.3).
#[derive(Clone, Debug, PartialEq)]
pub struct CallbackHmac {
    /// `secret:` — required; an `${ENV}` reference, never a literal.
    pub secret: Option<Spanned<EnvRef>>,
}

/// `callback_allow:` — the URL patterns a callback may point at
/// (grammar 13.3, PRD resolved q33).
#[derive(Clone, Debug, PartialEq)]
pub struct CallbackAllow {
    /// The patterns, in declaration order.
    pub patterns: Vec<Spanned<String>>,
    /// The list's own span.
    pub span: Span,
}

/// A `schedule` trigger, which `serve` fires (grammar 13.4, PRD resolved q78).
#[derive(Clone, Debug, PartialEq)]
pub struct ScheduleTrigger {
    /// `cron:` — required; a 5-field POSIX cron expression.
    pub cron: Option<Spanned<String>>,
    /// `timezone:` — an IANA name; defaults to `UTC`.
    pub timezone: Option<Spanned<String>>,
    /// `input:` — CEL over `payload.scheduled_at` / `payload.trigger`.
    pub input: Option<Bindings>,
}

/// An `event` trigger, fed by its source's consumer under `serve` (grammar 13.5).
#[derive(Clone, Debug, PartialEq)]
pub struct EventTrigger {
    /// `source:` — required; a logical name bound by the deploy layer.
    pub source: Option<Spanned<Ident>>,
    /// `input:` — CEL over the event payload.
    pub input: Option<Bindings>,
    /// `dedupe_key:` — defaults to `payload.id`.
    pub dedupe_key: Option<Spanned<Cel>>,
    /// `dedupe_window:` — how long a key is held; defaults to
    /// [`DEFAULT_DEDUPE_WINDOW`] (grammar 13.5, PRD resolved q77 ruling a).
    pub dedupe_window: Option<Spanned<Duration>>,
    /// `auth:` — the scheme the routes about this trigger's work enforce: the
    /// status, resume and cancel routes of every execution it starts, and its
    /// source's `GET /sources/:name` (grammar 13.5, PRD resolved q32 one level
    /// up, Decision D162). The same block an `http` trigger's `auth:` is.
    pub auth: Option<Spanned<AuthScheme>>,
}
