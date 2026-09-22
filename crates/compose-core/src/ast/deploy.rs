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
use super::trigger::CallbackAuth;

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

/// The `trace_sink:` section (grammar 14.5, PRD resolved q50, q51).
///
/// One address every settled execution's trace is shipped to, on the delivery
/// machinery a `callback:` already uses. A deploy-layer key rather than a spec
/// one because *where the traces go* is a property of an environment, which is
/// the whole of what this layer forks for.
#[derive(Clone, Debug, PartialEq)]
pub struct TraceSinkSection {
    /// `url:` — required; an absolute `http`/`https` address, shape-checked the
    /// way [`HubSection::public_url`] is and a class-3 string for the same
    /// reason (grammar 4.3, Decision D92).
    pub url: Option<Spanned<String>>,
    /// `format:` — what is POSTed. Absent means
    /// [`TraceSinkFormat::DEFAULT`].
    pub format: Option<Spanned<TraceSinkFormat>>,
    /// `auth:` — how a delivery identifies itself to the sink. The **outbound**
    /// signing shape of grammar 13.3, reused rather than re-invented: a sink
    /// delivery is a delivery, signed by the same headers a callback is
    /// (PRD resolved q50).
    pub auth: Option<Spanned<CallbackAuth>>,
    /// The section's own span.
    pub span: Span,
}

/// What a sink delivery carries (grammar 14.5, PRD resolved q51).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TraceSinkFormat {
    /// `envelope` — the trace envelope itself, the object the trace file holds
    /// (`docs/trace.md` §2). The default.
    Envelope,
    /// `otlp` — an OTLP/JSON `ExportTraceServiceRequest`, hand-emitted from the
    /// same envelope (PRD resolved q51).
    Otlp,
}

impl TraceSinkFormat {
    /// Every format, in the order grammar 14.5 lists them.
    pub const ALL: &'static [Self] = &[Self::Envelope, Self::Otlp];

    /// What a sink ships when it declares no `format:`.
    ///
    /// The envelope, deliberately: it is the trace this project already
    /// documents and the one a reader can diff against the trace file, so the
    /// OTLP mapping is a choice an author makes rather than a translation they
    /// are opted into.
    pub const DEFAULT: Self = Self::Envelope;

    /// The keyword that names this format.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Envelope => "envelope",
            Self::Otlp => "otlp",
        }
    }
}

/// The `package_registry:` section (grammar 14.6, PRD resolved q59).
///
/// Where a generated project's **installer** resolves packages from. A
/// deploy-layer key for the reason the journal and the trace sink are ones (PRD
/// resolved q27, q50): which registry a machine may reach is a placement fact,
/// and placement facts belong in deploy files. The composition says nothing.
///
/// It is the one deploy-layer section whose effect is not on a running process
/// at all — `build` writes it out as `bunfig.toml` and `.npmrc`, and the two
/// installers read those before any of this project's code exists.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageRegistrySection {
    /// `url:` — required; the registry every package resolves from unless a
    /// scope names another. An absolute `http`/`https` address, shape-checked
    /// the way [`HubSection::public_url`] and [`TraceSinkSection::url`] are, and
    /// a class-3 string for the same reason (grammar 4.3, Decision D92).
    pub url: Option<Spanned<String>>,
    /// `token:` — the credential the installer presents, as an `${ENV}`
    /// reference and never a literal (grammar 4.3, PRD resolved q32). Absent
    /// where the mirror wants none, which is the ordinary read-through case.
    pub token: Option<Spanned<EnvRef>>,
    /// `scopes:` — per-scope overrides, in declaration order. Corporate setups
    /// routinely split a private scope off a read-through mirror, which is why
    /// the slot has this second half at all.
    pub scopes: Vec<PackageRegistryScope>,
    /// The section's own span.
    pub span: Span,
}

/// One entry of `package_registry.scopes` (grammar 14.6).
///
/// The key is an npm **scope**, written with its `@` — `"@corp"` — because that
/// is how a scope is spelled everywhere else a reader meets one: in a package
/// name, in `.npmrc`'s `@corp:registry=`, and in Bun's `[install.scopes]` table.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageRegistryScope {
    /// The scope, from the section key, `@` included.
    pub name: Spanned<String>,
    /// `url:` — required, the same shape rule the section's own takes.
    pub url: Option<Spanned<String>>,
    /// `token:` — optional, the same env-ref rule the section's own takes.
    pub token: Option<Spanned<EnvRef>>,
    /// The whole entry's span, name and body together. [`Self::name`] carries
    /// the name alone, for the diagnostics that are about it.
    pub span: Span,
}

/// The `journal:` section (grammar 14.7, PRD resolved q62).
///
/// **One journal per target**, which is what makes this a single backend config
/// rather than `storage_backends:`' aliases-and-defaults shape: a store is a
/// slot a composition names and a journal is not named anywhere at all — the
/// composition says nothing (PRD resolved q27), so there is one of these and no
/// key selects between two.
///
/// The block is OPTIONAL and its absence is [`JournalProvider::DEFAULT`] stated
/// rather than implied: a target that declares nothing gets a SQLite file beside
/// the project, which is "durable by default, zero configuration" everywhere and
/// not only on a laptop (PRD resolved q27, q62).
#[derive(Clone, Debug, PartialEq)]
pub struct JournalSection {
    /// `provider:` — required. Which of the three backends binds.
    pub provider: Option<Spanned<JournalProvider>>,
    /// `url:` — the connection this journal dials, as an `${ENV}` reference and
    /// never a literal (grammar 4.3, PRD resolved q32). Required by a provider
    /// that dials out, refused by the one that does not.
    pub url: Option<Spanned<EnvRef>>,
    /// Whether `url:` was **written**, whatever became of it.
    ///
    /// [`Self::url`] is `None` both for a key nobody wrote and for one the
    /// env-ref rule refused, and the requiredness rule has to tell those apart,
    /// for the reason [`HubSection::declares_join_token`] does: an author who
    /// wrote a literal has already been told what is wrong with it, and adding
    /// "this provider dials out and declares no `url:`" would be a second
    /// diagnostic for one mistake.
    pub declares_url: bool,
    /// The section's own span.
    pub span: Span,
}

/// The journal backends a target binds (grammar 14.7, PRD resolved q62).
///
/// One interface, three implementations: the record vocabulary, the keys, the
/// frontier and the recovery verbs of [`docs/durability.md`] do not move, and a
/// runtime cannot tell which it got (PRD resolved q27).
///
/// [`docs/durability.md`]: https://docs.rs/ "docs/durability.md"
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalProvider {
    /// `sqlite` — one file beside the project's stores, opened in this process.
    Sqlite,
    /// `postgres` — a Postgres server, dialled with the `pg` driver.
    Postgres,
    /// `mysql` — a MySQL server, dialled with the `mysql2` driver.
    Mysql,
}

impl JournalProvider {
    /// Every provider, in the order grammar 14.7 lists them.
    pub const ALL: &'static [Self] = &[Self::Sqlite, Self::Postgres, Self::Mysql];

    /// What a target binds when it declares no `journal:` block at all.
    ///
    /// SQLite, everywhere — not as a local-only concession but as the default a
    /// named target keeps until it says otherwise, which is what makes "durable
    /// by default, zero configuration" true of every target (PRD resolved q27,
    /// q62).
    pub const DEFAULT: Self = Self::Sqlite;

    /// The keyword that names this provider.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Sqlite => "sqlite",
            Self::Postgres => "postgres",
            Self::Mysql => "mysql",
        }
    }

    /// Whether the process reaching this journal **opens it itself**, rather
    /// than dialling a service that holds it.
    ///
    /// The journal's analogue of [`BackendProvider::opens_in_process`], stated
    /// on the vocabulary that names the providers for the same reason: it is a
    /// property of the backend rather than of a target. Unlike the store rule it
    /// keys no placement check — **only the hub ever writes the journal** (PRD
    /// resolved q42), so there is no second process to fork a file between — and
    /// what it decides instead is whether the block needs a `url:` at all
    /// (PRD resolved q45, q62, Decision D131).
    #[must_use]
    pub const fn opens_in_process(self) -> bool {
        match self {
            Self::Sqlite => true,
            Self::Postgres | Self::Mysql => false,
        }
    }

    /// The providers that dial out, in declaration order — the ones a `url:`
    /// belongs to.
    pub fn networked() -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|provider| !provider.opens_in_process())
    }
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

    /// Whether the process reaching a store on this backend **opens the store
    /// itself**, rather than dialling a service that holds it.
    ///
    /// The criterion is where the bytes live, not the keyword: a heap map, a
    /// SQLite file — with or without the vector extension — and a directory of
    /// blobs are all opened by whichever process reaches them, so two processes
    /// reaching one such store hold two stores. Everything else here is a
    /// connection to something outside the process, which is what makes two
    /// processes reaching it two readers of one store.
    ///
    /// It is what grammar 14.1 rule 5 refuses a placement over (PRD resolved
    /// q45, Decision D131), and it is a property of the *provider* rather than
    /// of a store or a target, so it is stated on the vocabulary that names
    /// them.
    #[must_use]
    pub const fn opens_in_process(self) -> bool {
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

    /// The networked providers, in declaration order — what an author binds
    /// instead of a process-local one (grammar 14.1 rule 5).
    pub fn networked() -> impl Iterator<Item = Self> {
        Self::ALL
            .iter()
            .copied()
            .filter(|provider| !provider.opens_in_process())
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
