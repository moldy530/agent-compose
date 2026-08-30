//! The deploy layer: `deploy/<target>.yml` (grammar 14).
//!
//! Only this layer forks per environment (PRD 5.8's per-target invariant), and
//! it is disjoint from spec files (Decision D3). `event_sources` is reserved
//! grammar: parsed and validated in v0, executed in M3 (grammar 15).
//!
//! `hub:` and `placements:` are **not** reserved, and no longer awaiting a
//! runtime either. They are the live surface of the distributed claims model
//! (PRD resolved q37–q44): a placement is a logical name a worker claims at an
//! authenticated join, and the hub is the process that owns the graph. Every
//! rule about them is enforced by `validate`, a build emits the hub these keys
//! describe, and `agent-compose worker` is the spoke that joins it;
//! `crates/compose-core/tests/placement_surface_landing.rs` is what says so in
//! executable form (grammar 14.1, 14.2, `docs/distributed.md`).

use crate::diag::{Span, Spanned};

use super::common::{Address, EnvRef, Ident, Interpolated};
use super::definition::StoreKind;

/// The `placements:` section (grammar 14.1).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementsSection {
    /// The placements, in declaration order.
    pub placements: Vec<Placement>,
    /// The section's own span.
    pub span: Span,
}

/// One named placement: a claim a worker asserts, and the components it runs
/// (grammar 14.1, Decision D128).
///
/// The name is the whole binding surface. Which machine satisfies it is decided
/// by whoever joins asserting it — capability affinity, not load assignment —
/// so nothing here is an address (PRD resolved q38).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The name a worker claims, from the section key.
    pub name: Spanned<Ident>,
    /// `members:` — the component addresses this placement runs. Non-empty,
    /// `agent.*` and `tool.*` only in v1 (Decision D129).
    pub members: Vec<Spanned<Address>>,
    /// `description:`
    pub description: Option<Spanned<String>>,
    /// The whole entry's span, name and body together. [`Self::name`] carries
    /// the name alone, for the diagnostics that are about it.
    pub span: Span,
}

/// The `hub:` section (grammar 14.2, Decision D130).
///
/// Both keys are optional on their own: a target that declares placements needs
/// the token, and a target that declares none may still want to say where its
/// ingress is.
#[derive(Clone, Debug, PartialEq)]
pub struct HubSection {
    /// `join_token:` — the bearer credential a worker joins with, as an
    /// `${ENV}` reference and never as a literal (grammar 4.3, PRD resolved
    /// q32/q38).
    pub join_token: Option<Spanned<EnvRef>>,
    /// `public_url:` — the absolute base every ingress URL this deployment
    /// hands out derives from (PRD resolved q44 invariant 4).
    pub public_url: Option<Spanned<String>>,
    /// Whether `join_token:` was **written**, whatever became of it.
    ///
    /// [`Self::join_token`] is `None` both for a key nobody wrote and for one
    /// the env-ref rule refused, and the requiredness rule has to tell those
    /// apart: an author who wrote a literal has already been told what is wrong
    /// with it, and adding "this target declares placements and no
    /// `hub.join_token`" would be a second diagnostic for one mistake — the
    /// same reason a deploy file whose `version:` was rejected is not also told
    /// it is missing.
    pub declares_join_token: bool,
    /// The section's own span.
    pub span: Span,
}

/// The `storage_backends:` section (grammar 14.3).
#[derive(Clone, Debug, PartialEq)]
pub struct StorageBackendsSection {
    /// `defaults:` — per-kind fallback backends.
    pub defaults: Vec<BackendDefault>,
    /// `aliases:` — the named slots stores refer to with `backend:`.
    pub aliases: Vec<BackendAlias>,
    /// The section's own span.
    pub span: Span,
}

/// One entry of `storage_backends.defaults`.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendDefault {
    /// The store kind this backend serves.
    pub kind: Spanned<StoreKind>,
    /// Its configuration.
    pub config: BackendConfig,
}

/// One entry of `storage_backends.aliases`.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendAlias {
    /// The alias name, as `store.<s>.backend` spells it.
    pub name: Spanned<Ident>,
    /// Its configuration.
    pub config: BackendConfig,
}

/// A backend configuration (grammar 14.3).
///
/// An open plugin-config object (Decision D50): `provider` is a closed
/// vocabulary and the connection fields of grammar 4.3 must be `${ENV}`
/// value-form references, while every other key is passed through to the
/// storage plugin's own schema.
#[derive(Clone, Debug, PartialEq)]
pub struct BackendConfig {
    /// `provider:` — required.
    pub provider: Option<Spanned<BackendProvider>>,
    /// Connection fields that must be environment references (grammar 4.3).
    pub connection: Vec<ConnectionField>,
    /// Everything else, for the storage plugin's schema to check.
    pub extra: Vec<PluginEntry>,
    /// The mapping's own span.
    pub span: Span,
}

/// One connection field of a backend or event source: a key from grammar 4.3's
/// closed secret list, whose value is an environment reference.
#[derive(Clone, Debug, PartialEq)]
pub struct ConnectionField {
    /// The field name (`url`, `dsn`, `access_key_id`, …).
    pub name: Spanned<String>,
    /// Its value.
    pub value: Spanned<EnvRef>,
}

/// The v0 storage providers (grammar 14.3, Decision D48).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BackendProvider {
    /// `memory` (kv)
    Memory,
    /// `sqlite` (kv)
    Sqlite,
    /// `redis` (kv)
    Redis,
    /// `postgres` (kv)
    Postgres,
    /// `sqlite_vec` (vector)
    SqliteVec,
    /// `chroma` (vector)
    Chroma,
    /// `pgvector` (vector)
    Pgvector,
    /// `qdrant` (vector)
    Qdrant,
    /// `local_fs` (blob)
    LocalFs,
    /// `s3` (blob)
    S3,
    /// `gcs` (blob)
    Gcs,
}

impl BackendProvider {
    /// Every provider, grouped by kind in the order grammar 14.3 lists them.
    pub const ALL: &'static [Self] = &[
        Self::Memory,
        Self::Sqlite,
        Self::Redis,
        Self::Postgres,
        Self::SqliteVec,
        Self::Chroma,
        Self::Pgvector,
        Self::Qdrant,
        Self::LocalFs,
        Self::S3,
        Self::Gcs,
    ];

    /// The keyword that names this provider.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Memory => "memory",
            Self::Sqlite => "sqlite",
            Self::Redis => "redis",
            Self::Postgres => "postgres",
            Self::SqliteVec => "sqlite_vec",
            Self::Chroma => "chroma",
            Self::Pgvector => "pgvector",
            Self::Qdrant => "qdrant",
            Self::LocalFs => "local_fs",
            Self::S3 => "s3",
            Self::Gcs => "gcs",
        }
    }

    /// The store kind this provider serves.
    #[must_use]
    pub const fn kind(self) -> StoreKind {
        match self {
            Self::Memory | Self::Sqlite | Self::Redis | Self::Postgres => StoreKind::Kv,
            Self::SqliteVec | Self::Chroma | Self::Pgvector | Self::Qdrant => StoreKind::Vector,
            Self::LocalFs | Self::S3 | Self::Gcs => StoreKind::Blob,
        }
    }

    /// Whether this backend lives **inside** the process that opens it.
    ///
    /// A heap, a file and a directory: two processes that open "the same" one of
    /// these open two, so what one writes the other cannot read. The rest are
    /// network services addressed by a URL, so a hub and a worker pointed at one
    /// are pointed at the same data — which is the premise
    /// `docs/distributed.md` §1 states as "global-scope stores are already
    /// external backends" and grammar 14.1 turns into a rule about placements.
    #[must_use]
    pub const fn is_process_local(self) -> bool {
        match self {
            Self::Memory | Self::Sqlite | Self::SqliteVec | Self::LocalFs => true,
            Self::Redis
            | Self::Postgres
            | Self::Chroma
            | Self::Pgvector
            | Self::Qdrant
            | Self::S3
            | Self::Gcs => false,
        }
    }
}

/// The `event_sources:` section — reserved grammar (grammar 14.4).
#[derive(Clone, Debug, PartialEq)]
pub struct EventSourcesSection {
    /// The sources, in declaration order.
    pub sources: Vec<EventSource>,
    /// The section's own span.
    pub span: Span,
}

/// One event source: a logical name bound to infrastructure (grammar 14.4).
#[derive(Clone, Debug, PartialEq)]
pub struct EventSource {
    /// The logical name an `event` trigger's `source:` refers to.
    pub name: Spanned<Ident>,
    /// `kind:` — required; the consumer plugin.
    pub kind: Option<Spanned<EventSourceKind>>,
    /// Connection fields that must be environment references (grammar 4.3).
    pub connection: Vec<ConnectionField>,
    /// Everything else, for the consumer plugin's schema to check.
    pub extra: Vec<PluginEntry>,
    /// The whole entry's span, name and body together. [`Self::name`] carries
    /// the name alone, for the diagnostics that are about the name.
    pub span: Span,
}

/// The v0 event-source kinds (grammar 14.4).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventSourceKind {
    /// `redis_streams`
    RedisStreams,
    /// `sqs`
    Sqs,
    /// `nats`
    Nats,
}

impl EventSourceKind {
    /// Every kind, in the order grammar 14.4 lists them.
    pub const ALL: &'static [Self] = &[Self::RedisStreams, Self::Sqs, Self::Nats];

    /// The keyword that names this kind.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RedisStreams => "redis_streams",
            Self::Sqs => "sqs",
            Self::Nats => "nats",
        }
    }
}

/// Field names that must hold an `${ENV}` value-form reference wherever they
/// appear in a `provider.*`, a backend config, or an event source
/// (grammar 4.3, Decision D41).
pub const SECRET_FIELDS: &[&str] = &[
    "api_key",
    "api_secret",
    "token",
    "password",
    "access_key_id",
    "secret_access_key",
    "session_token",
    "credentials_json",
    "url",
    "base_url",
    "endpoint",
    "dsn",
];

/// A value of an open plugin-config object, under a key outside
/// [`SECRET_FIELDS`] (grammar 14.3, 14.4, Decision D50).
///
/// The shape mirrors [`Literal`](super::common::Literal), because a plugin
/// object carries arbitrary YAML for the plugin's own published schema to
/// check. What differs is the string arm: grammar 4.3 puts these values in
/// class 2, so a `${NAME}` token in one is substituted at process start, and
/// [`Interpolated`] is what carries the references unresolved into the IR for
/// `build`/`serve`/`run` to check for presence.
#[derive(Clone, Debug, PartialEq)]
pub enum PluginValue {
    /// `~` / `null`.
    Null,
    /// A boolean.
    Bool(bool),
    /// An integer.
    Int(i64),
    /// A float.
    Float(f64),
    /// A string, with its environment references recorded.
    Text(Interpolated),
    /// A sequence of values.
    Sequence(Vec<Spanned<PluginValue>>),
    /// A mapping of values, in declaration order.
    Mapping(Vec<PluginEntry>),
}

/// One entry of an open plugin-config object.
#[derive(Clone, Debug, PartialEq)]
pub struct PluginEntry {
    /// The key, spanned. Keys name plugin options rather than carrying values,
    /// so they stay in grammar 4.3's class 3: nothing interpolates one.
    pub key: Spanned<String>,
    /// Its value.
    pub value: Spanned<PluginValue>,
}
