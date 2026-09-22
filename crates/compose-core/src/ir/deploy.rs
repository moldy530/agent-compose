//! The deploy layer in the IR (grammar 14).
//!
//! The artifact is **per target**: `deploy/<name>.yml` is selected by
//! `--target <name>`, never imported, and only this layer forks per environment
//! (PRD 5.8's per-target invariant). [`Deploy::target`] records which one was
//! resolved, so two artifacts built from one composition are told apart by the
//! document rather than by the file they were written to.
//!
//! `local` is the built-in target: its deploy file is optional, it may not
//! declare `storage_backends:` — the section it overrides unconditionally — and
//! the two target-dependent binding checks are satisfied vacuously under it
//! (Decision D87).

use std::collections::BTreeMap;

use serde::Serialize;

use crate::ast::common::{Address, EnvRef, Ident};
use crate::ast::definition::StoreKind;
use crate::ast::deploy::{
    BackendProvider, EventSourceKind, JournalProvider, PluginValue, TraceSinkFormat,
};
use crate::diag::{Span, Spanned};

use super::Section;

/// The active target's deploy layer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Deploy {
    /// The target this artifact was resolved for.
    pub target: String,
    /// The deploy file it was read from, when there is one. `local` needs
    /// none, and a target that has one is required to have it (Decision D87).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `hub:` — the process that owns the graph: its join credential and its
    /// ingress base (grammar 14.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hub: Option<Hub>,
    /// `placements:` — keyed by the name a worker claims at join, each naming
    /// the components that claim runs (grammar 14.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub placements: Option<Section<Placement>>,
    /// `storage_backends:` — absent means the section was not declared, which
    /// is not the same as an empty one: under a named target a store's alias
    /// still has to resolve somewhere (grammar 11.3). It carries its span the
    /// way every section does; what keeps it from being a [`Section`] is that
    /// it holds two maps rather than one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_backends: Option<StorageBackends>,
    /// `journal:` — which backend this target's execution journal binds
    /// (grammar 14.7, PRD resolved q62). Absent means
    /// [`JournalProvider::DEFAULT`], which [`journal_of`] is the one reading of.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub journal: Option<Journal>,
    /// `package_registry:` — where this target's installer resolves packages
    /// from (grammar 14.6, PRD resolved q59).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub package_registry: Option<PackageRegistry>,
    /// `trace_sink:` — where every settled execution's trace ships
    /// (grammar 14.5, PRD resolved q50).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trace_sink: Option<TraceSink>,
    /// `event_sources:` — reserved grammar, keyed by the logical name an
    /// `event` trigger's `source:` names (grammar 14.4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub event_sources: Option<Section<EventSource>>,
}

/// The `trace_sink:` block, resolved (grammar 14.5, PRD resolved q50, q51).
///
/// A struct beside [`Section`] rather than one of them, for the reason [`Hub`]
/// is: it is the deployment's own singleton rather than a map of named entries.
///
/// The grammar's defaults land **here** rather than being left to whoever reads
/// the artifact: [`Self::format`] decides what a delivery's body is, and a
/// reader that re-derived it differently would ship a collector something it
/// cannot parse (the reading `inbound_auth`'s defaults take, one layer up).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct TraceSink {
    /// `url:` — the absolute address every settled execution's trace is POSTed
    /// to.
    pub url: Spanned<String>,
    /// `format:` — what the body is, with grammar 14.5's default applied.
    pub format: TraceSinkFormat,
    /// `auth:` — how a delivery identifies itself to the sink, absent where the
    /// collector wants no credential. The **outbound** shape of grammar 13.3,
    /// resolved, so a sink delivery is signed by the headers a callback is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auth: Option<crate::ir::trigger::CallbackAuth>,
    /// The section's own span.
    pub span: Span,
}

/// The `journal:` block, resolved (grammar 14.7, PRD resolved q62).
///
/// A struct beside [`Section`] rather than one of them, for the reason [`Hub`],
/// [`TraceSink`] and [`PackageRegistry`] are: it is the deployment's own
/// singleton. **One journal per target** — nothing in the composition names one,
/// so there is nothing for a map of named entries to be keyed by (PRD resolved
/// q27).
///
/// The **absence** of the block is not modelled here and deliberately: `None` on
/// [`Deploy::journal`] is a target that declared nothing, which is
/// [`JournalProvider::DEFAULT`], and [`journal_of`] is the single place that
/// reading is made. A default applied here instead would make a target that
/// wrote nothing indistinguishable in the artifact from one that wrote
/// `provider: sqlite`, which is a difference a plan document reports.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Journal {
    /// `provider:` — which of the three backends this target binds.
    pub provider: JournalProvider,
    /// `url:` — the connection a provider that dials out is reached at,
    /// unresolved (grammar 4.3): a name the deployment supplies, never a value
    /// the artifact carries. Absent on `sqlite`, which opens a file.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<Spanned<EnvRef>>,
    /// The section's own span.
    pub span: Span,
}

/// The `package_registry:` block, resolved (grammar 14.6, PRD resolved q59).
///
/// A struct beside [`Section`] rather than one of them, for the reason [`Hub`]
/// and [`TraceSink`] are: it is the deployment's own singleton rather than a map
/// of named entries — with one map hanging off it, which is the per-scope half.
///
/// It is the one part of the deploy layer no *running* process reads. `build`
/// lowers it to `bunfig.toml` and `.npmrc` (`codegen::registry`), and the two
/// installers read those before this project's own code exists, which is why the
/// token here is an unresolved reference like every other credential: what ships
/// in the artifact is the variable's **name**, and the installer expands it
/// (PRD resolved q32, q40).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PackageRegistry {
    /// `url:` — the registry every package resolves from unless a scope names
    /// another.
    pub url: Spanned<String>,
    /// `token:` — the credential the installer presents, unresolved
    /// (grammar 4.3), absent where the mirror reads through without one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<Spanned<EnvRef>>,
    /// `scopes:` — the per-scope overrides, keyed by the scope with its `@`.
    /// Sorted: a config object's key order carries nothing, and the emitted
    /// files are a function of what was declared rather than of the order it was
    /// written in.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub scopes: BTreeMap<String, PackageRegistryScope>,
    /// The section's own span.
    pub span: Span,
}

/// One entry of `package_registry.scopes` (grammar 14.6).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PackageRegistryScope {
    /// The scope, repeated from the key with its own span — which is what
    /// carries the span a diagnostic about the entry points at.
    pub name: Spanned<String>,
    /// `url:` — the registry packages in this scope resolve from.
    pub url: Spanned<String>,
    /// `token:` — this scope's own credential, unresolved.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub token: Option<Spanned<EnvRef>>,
    /// The whole entry's span.
    pub span: Span,
}

/// The `hub:` block (grammar 14.2, Decision D130).
///
/// Both keys are the deployment's own rather than a component's, so this is a
/// struct beside [`Section`] rather than one of them — the shape
/// [`StorageBackends`] takes for the same reason.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Hub {
    /// `join_token:` — the bearer credential a worker joins with, unresolved
    /// (grammar 4.3): a name the deployment supplies, never a value the
    /// artifact carries.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub join_token: Option<Spanned<EnvRef>>,
    /// `public_url:` — the absolute base every ingress URL derives from (PRD
    /// resolved q44 invariant 4).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub public_url: Option<Spanned<String>>,
    /// The section's own span.
    pub span: Span,
}

/// One named placement: the claim, and the components it runs (grammar 14.1,
/// Decision D128).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Placement {
    /// The claim's name, repeated from the key with its own span.
    pub name: Spanned<Ident>,
    /// `members:` — the components this claim runs, in declaration order. One
    /// placement's members are the author's list rather than a set the artifact
    /// re-derives, so the order is the file's (see the module docs on order).
    pub members: Vec<Spanned<Address>>,
    /// `description:` — documentation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// The whole entry's span.
    pub span: Span,
}

/// The `storage_backends:` section (grammar 14.3).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct StorageBackends {
    /// `defaults:` — the per-kind fallback, consulted when a store names no
    /// alias.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub defaults: BTreeMap<String, BackendConfig>,
    /// `aliases:` — the named slots a store's `backend:` refers to.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub aliases: BTreeMap<String, BackendConfig>,
    /// The section's own span.
    pub span: Span,
}

/// A backend configuration (grammar 14.3).
///
/// An open plugin-config object (Decision D50): `provider` is a closed
/// vocabulary and the connection fields of grammar 4.3 are `${ENV}` value-form
/// references, while every other key is passed through for the storage plugin's
/// own schema to check.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BackendConfig {
    /// The alias name, or the store kind for a per-kind default, repeated from
    /// the key it sits under — which is what carries the span a diagnostic
    /// about the entry points at.
    pub name: Spanned<String>,
    /// The store kind this backend serves, present on a per-kind `defaults:`
    /// entry and absent on an alias. It is what tells the two apart in a value
    /// read on its own, which is why it stays beside the key that repeats it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kind: Option<StoreKind>,
    /// `provider:` — the storage plugin.
    pub provider: BackendProvider,
    /// The connection fields of grammar 4.3, each an unresolved environment
    /// reference. Sorted by field name: a config object's key order carries
    /// nothing.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub connection: BTreeMap<String, Spanned<crate::ast::common::EnvRef>>,
    /// Everything else, for the storage plugin's schema. Class-2 strings, so a
    /// `${NAME}` token in one is an unresolved reference rather than text.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Spanned<PluginValue>>,
    /// The whole entry's span.
    pub span: Span,
}

/// One event source: a logical name bound to infrastructure (grammar 14.4).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EventSource {
    /// The logical name, repeated from the key with its own span.
    pub name: Spanned<Ident>,
    /// `kind:` — the consumer plugin.
    pub kind: EventSourceKind,
    /// The connection fields of grammar 4.3, each an unresolved environment
    /// reference.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub connection: BTreeMap<String, Spanned<crate::ast::common::EnvRef>>,
    /// Everything else, for the consumer plugin's schema.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub extra: BTreeMap<String, Spanned<PluginValue>>,
    /// The whole entry's span.
    pub span: Span,
}

// ---------------------------------------------------------------------------
// Which journal a target binds (grammar 14.7, PRD resolved q62)
// ---------------------------------------------------------------------------

/// Which journal backend the active target binds.
///
/// One reading of the absent block, stated once, for [`backend_of`]'s reason:
/// codegen asks in order to emit the arm and pin its driver,
/// [`crate::codegen::env`] asks whether a connection variable joins the hub's
/// manifest, and the emitted `README.md` asks in order to tell a reader where
/// this project's journal lives. Three answers to one question would be three
/// ways for a deployment to be told something its journal does not do.
///
/// A target that declares no `journal:` binds [`JournalProvider::DEFAULT`] —
/// SQLite beside the project — under **every** target and not only `local`,
/// which is PRD resolved q62's "zero-config parity is the default everywhere,
/// not a local-only concession".
#[must_use]
pub fn journal_of(ir: &crate::ir::Ir) -> JournalProvider {
    ir.deploy
        .journal
        .as_ref()
        .map_or(JournalProvider::DEFAULT, |journal| journal.provider)
}

// ---------------------------------------------------------------------------
// Which backend a store binds (grammar 11.3, 14.3, Decision D87)
// ---------------------------------------------------------------------------

/// Which backend a store resolved to under the active target, and why.
///
/// The `from` clause is written for a person: a diagnostic and the runtime's own
/// refusal both quote it, so an author reading either is told which line of
/// which file decided the binding.
#[derive(Clone, Debug, PartialEq)]
pub struct ResolvedBackend {
    /// The storage plugin (grammar 14.3).
    pub provider: BackendProvider,
    /// Where the binding came from, as a clause a sentence can end with.
    pub from: String,
    /// The `storage_backends:` entry that decided it, as a spec address —
    /// `deploy.storage_backends.aliases.<alias>` or
    /// `deploy.storage_backends.defaults.<kind>`. Absent where the target
    /// built-in decided it, because no entry wrote that one.
    pub site: Option<String>,
}

/// Grammar 11.3's resolution order, run at compile time.
///
/// `--target local` substitutes local storage for **every** store
/// unconditionally, so under it no alias and no per-kind default is consulted at
/// all (PRD 5.8, Decision D87) — which is what makes a project with production
/// infrastructure in `deploy/staging.yml` still buildable and runnable with none.
/// Under any other target the order is the grammar's: explicit alias, then the
/// per-kind `defaults:`, then the target built-in, which is the same local
/// storage because it is the only backend this compiler release implements.
///
/// Stated once, here, rather than at each of the three passes that ask. Codegen
/// asks in order to emit the binding and the refusal that goes with it,
/// `check::placements` asks whether a placed component reaches a store only its
/// own process can see (grammar 14.1), and `codegen::env` asks which processes a
/// backend's credentials belong to (`docs/distributed.md` §9.1). Three answers
/// to one question would be three ways for a deployment to be told something the
/// store does not do.
#[must_use]
pub fn backend_of(ir: &crate::ir::Ir, store: &crate::ir::definition::Store) -> ResolvedBackend {
    let built_in = match store.kind {
        StoreKind::Kv => BackendProvider::Sqlite,
        StoreKind::Vector => BackendProvider::SqliteVec,
        StoreKind::Blob => BackendProvider::LocalFs,
    };
    if ir.target == crate::DEFAULT_TARGET {
        return ResolvedBackend {
            provider: built_in,
            from: "the `local` target substitutes local storage for every store unconditionally"
                .to_string(),
            site: None,
        };
    }
    let backends = ir.deploy.storage_backends.as_ref();
    if let Some(alias) = &store.backend
        && let Some(config) =
            backends.and_then(|backends| backends.aliases.get(alias.value.as_str()))
    {
        return ResolvedBackend {
            provider: config.provider,
            from: format!(
                "the alias `{}`, defined by the `{}` target",
                alias.value, ir.target
            ),
            site: Some(format!("deploy.storage_backends.aliases.{}", alias.value)),
        };
    }
    if let Some(config) = backends.and_then(|backends| backends.defaults.get(store.kind.as_str())) {
        return ResolvedBackend {
            provider: config.provider,
            from: format!(
                "the `{}` default of the `{}` target",
                store.kind.as_str(),
                ir.target
            ),
            site: Some(format!(
                "deploy.storage_backends.defaults.{}",
                store.kind.as_str()
            )),
        };
    }
    ResolvedBackend {
        provider: built_in,
        from: format!(
            "the built-in for `kind: {}`, which the `{}` target does not override",
            store.kind.as_str(),
            ir.target
        ),
        site: None,
    }
}
