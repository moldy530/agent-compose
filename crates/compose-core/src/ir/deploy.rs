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

use crate::ast::common::{Address, Ident};
use crate::ast::definition::StoreKind;
use crate::ast::deploy::{BackendProvider, EventSourceKind, Network, PluginValue, Runtime};
use crate::diag::{Span, Spanned};

/// The active target's deploy layer.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Deploy {
    /// The target this artifact was resolved for.
    pub target: String,
    /// The deploy file it was read from, when there is one. `local` needs
    /// none, and a target that has one is required to have it (Decision D87).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// `placements:` — reserved grammar, keyed by the component address, which
    /// resolves in the composition (grammar 14.1).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub placements: BTreeMap<String, Placement>,
    /// `storage_backends:` — absent means the section was not declared, which
    /// is not the same as an empty one: under a named target a store's alias
    /// still has to resolve somewhere (grammar 11.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub storage_backends: Option<StorageBackends>,
    /// `event_sources:` — reserved grammar, keyed by the logical name an
    /// `event` trigger's `source:` names (grammar 14.3).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub event_sources: BTreeMap<String, EventSource>,
}

/// Where a component runs (grammar 14.1, Decision D47).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Placement {
    /// The component address, repeated from the key with its own span.
    pub address: Spanned<Address>,
    /// `runtime:` — own instance/container, or in-process.
    pub runtime: Runtime,
    /// `network:` — reserved; absent means the default, `all`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub network: Option<Network>,
    /// `description:` — documentation only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<Spanned<String>>,
    /// The whole entry's span.
    pub span: Span,
}

/// The `storage_backends:` section (grammar 14.2).
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

/// A backend configuration (grammar 14.2).
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

/// One event source: a logical name bound to infrastructure (grammar 14.3).
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
