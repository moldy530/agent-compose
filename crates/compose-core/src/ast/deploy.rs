//! The deploy layer: `deploy/<target>.yml` (grammar 14).
//!
//! Only this layer forks per environment (PRD 5.8's per-target invariant), and
//! it is disjoint from spec files (Decision D3). `placements` and
//! `event_sources` are reserved grammar: parsed and validated in v0, executed
//! in M3 (grammar 15).

use crate::diag::{Span, Spanned};

use super::common::{Address, EnvRef, Ident, Interpolated, LiteralEntry};
use super::definition::StoreKind;

/// The `placements:` section (grammar 14.1).
#[derive(Clone, Debug, PartialEq)]
pub struct PlacementsSection {
    /// The placements, in declaration order.
    pub placements: Vec<Placement>,
    /// The section's own span.
    pub span: Span,
}

/// Where a component runs (grammar 14.1, Decision D47).
#[derive(Clone, Debug, PartialEq)]
pub struct Placement {
    /// The component address: `agent.*`, `tool.*`, or `flow.*`.
    pub address: Spanned<Address>,
    /// `runtime:` — required.
    pub runtime: Option<Spanned<Runtime>>,
    /// `network:` — reserved; defaults to `all`.
    pub network: Option<Spanned<Network>>,
    /// `description:`
    pub description: Option<Spanned<String>>,
    /// The entry's own span.
    pub span: Span,
}

/// A placement's runtime (grammar 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    /// Its own instance or container.
    Isolated,
    /// In the calling process.
    Colocated,
}

/// A placement's sandbox network policy — reserved (grammar 14.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Network {
    /// No network access.
    None,
    /// Outbound only.
    Egress,
    /// Unrestricted (the default).
    All,
}

/// The `storage_backends:` section (grammar 14.2).
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

/// A backend configuration (grammar 14.2).
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
    pub extra: Vec<LiteralEntry>,
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

/// The v0 storage providers (grammar 14.2, Decision D48).
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
    /// Every provider, grouped by kind in the order grammar 14.2 lists them.
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
}

/// The `event_sources:` section — reserved grammar (grammar 14.3).
#[derive(Clone, Debug, PartialEq)]
pub struct EventSourcesSection {
    /// The sources, in declaration order.
    pub sources: Vec<EventSource>,
    /// The section's own span.
    pub span: Span,
}

/// One event source: a logical name bound to infrastructure (grammar 14.3).
#[derive(Clone, Debug, PartialEq)]
pub struct EventSource {
    /// The logical name an `event` trigger's `source:` refers to.
    pub name: Spanned<Ident>,
    /// `kind:` — required; the consumer plugin.
    pub kind: Option<Spanned<EventSourceKind>>,
    /// Connection fields that must be environment references (grammar 4.3).
    pub connection: Vec<ConnectionField>,
    /// Everything else, for the consumer plugin's schema to check.
    pub extra: Vec<LiteralEntry>,
    /// The entry's own span.
    pub span: Span,
}

/// The v0 event-source kinds (grammar 14.3).
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
    /// Every kind, in the order grammar 14.3 lists them.
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

/// A value that may embed environment references, used by the open
/// plugin-config objects for keys outside [`SECRET_FIELDS`].
pub type PluginValue = Spanned<Interpolated>;
