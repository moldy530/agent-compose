//! The flat IR: one JSON document per composition per target.
//!
//! Multi-file composition is authoring UX. The resolver flattens it into a
//! single self-contained artifact, and that artifact — not the `.yml` files — is
//! what the validator checks, what codegen lowers, and what a deployment ships
//! and diffs (PRD 5.1). Everything below follows from that one job.
//!
//! # What the artifact is
//!
//! ```text
//! {
//!   "ir_version":   "1",              // this document's shape
//!   "spec_version": "0.1",            // the DSL version the composition declares
//!   "entrypoint":   "main.yml",
//!   "target":       "local",
//!   "sources":      [ … ],            // every file that was read
//!   "defaults":     { … },            // the composition's policy defaults
//!   "state":        { … },            // channels, by name
//!   "triggers":     { … },            // declared triggers, by name
//!   "definitions":  { … },            // every definition, by typed address
//!   "deploy":       { … }             // the active target's layer
//! }
//! ```
//!
//! Each of those sections is written with the region it was declared in, so
//! `state` is `{"entries": {…}, "span": …}` and `defaults` is `{"value": {…},
//! "span": …}` (see [Spans](#spans)). `definitions` is the one map that is not
//! a section and is written as the bare map it is.
//!
//! **Self-contained.** Nothing in it refers to a file that has to be read to
//! understand it. Every definition is inlined under the address it is reached
//! by, every reference has been resolved against those addresses, and every
//! flow-local node id an edge names exists in the flow that declares it.
//!
//! **Faithful, not convenient.** The IR records what the composition *says*,
//! not what a consumer would find easiest. Defaults are not materialized: an
//! agent that declares no `max_tool_iterations:` has none here, because
//! grammar 9.3's four-level resolution chain and every other default belong to
//! the pass that applies them, and an artifact that had already applied them
//! could no longer tell an author's `timeout: 60s` from the one it invented.
//! Env refs are never substituted (PRD 5.8, 5.9). Nothing is lowered to JSON
//! Schema, Zod, or any codegen shape yet — grammar 3.8's mapping is M1's, and
//! designing the artifact around it now would bake one backend's convenience
//! into the format every backend reads.
//!
//! # Determinism
//!
//! PRD 5.12 requires byte-identical output for the same input, and grammar 1.4
//! makes import order non-semantic — so the artifact may not depend on it. The
//! rule is one line:
//!
//! > Collections whose order could otherwise come from **file layout** are
//! > sorted maps; everything below that level keeps the order it was written
//! > in.
//!
//! Canonicalized, therefore: [`Ir::definitions`] (sorted by address —
//! Decision D55), every [`Section::entries`] map — [`Ir::state`] and
//! [`Ir::triggers`], the deploy layer's placements and event sources — sorted
//! by name, a backend section's aliases and per-kind defaults, a model's
//! `settings:`, a plugin config's keys, and [`Ir::sources`] — which is in
//! resolution order, the entrypoint, then its imports **sorted by path**, then
//! the deploy file, and so does not move when an import list is reordered.
//! Kept as written, therefore:
//! a flow's `edges:` (grammar 7.3 evaluates them in declaration order), a
//! model's `route:` (failover order), an `exec:` block's `args:` (argv order),
//! and every other list and mapping inside a single definition — one file's
//! order is already deterministic, and preserving it keeps the artifact
//! readable against its source. Struct-shaped objects are written in the order
//! this module declares their fields.
//!
//! # Spans
//!
//! Every construct a later pass can report against carries the source region it
//! was written in, so the validator reports on the IR without re-parsing
//! anything. What does *not* carry one is the numeric and keyword constraint
//! keys the parser has already decided in full (`max_items`, `multiple_of`,
//! `max_concurrency`, `runtime`, …): the checks that read those report against
//! the construct carrying them, which is spanned. The one boolean that keeps
//! its span is `detach:`, because whether it is legal depends on the active
//! target and the diagnostic is about that key (grammar 8.6 rule 7).
//!
//! **A section is a construct too**, and every one of them is written the same
//! way: `defaults:`, `state:`, `triggers:`, and the deploy layer's
//! `placements:`, `storage_backends:`, and `event_sources:` each carry the
//! region the section itself was written in alongside what it declares
//! ([`Section`]). A rule whose subject is a whole section — a channel set that
//! declares nothing a flow writes, a target that binds no backend for a kind a
//! store needs — then has a place to point that is neither one arbitrary entry
//! nor the whole file. It also makes a declared-but-empty section distinct from
//! an absent one, which is the distinction grammar 14 already draws when it
//! refuses an empty `storage_backends:` under `local`.
//!
//! `definitions:` is deliberately not one of them: definitions are not written
//! under a section key at all, but at the top level of every file that declares
//! any (grammar 1.5), so there is no single region to record. Each definition
//! carries its own.
//!
//! [`leaf`] fixes how a span reaches JSON: one string, `file:line:col..line:col`.
//!
//! # `ir_version`
//!
//! [`IR_VERSION`] identifies the *shape* of this document, and is independent of
//! the `spec_version` a composition declares: the DSL and the artifact change
//! for different reasons and on different schedules. It is the first key, so a
//! consumer can dispatch on it before reading anything else, and a consumer that
//! does not know a version must refuse the document rather than guess. It is
//! bumped whenever an existing key changes meaning, moves, or disappears —
//! adding a key that was previously absent is not a bump, because a reader of
//! the older shape is unaffected by a key it does not look for.

pub mod binding;
pub mod definition;
pub mod deploy;
pub mod flow;
pub mod leaf;
pub mod policy;
pub mod schema;
pub mod trigger;

use std::collections::BTreeMap;

use serde::Serialize;

use crate::ast::common::Ident;
use crate::ast::document::Reduce;
use crate::diag::{Span, Spanned};

pub use definition::Definition;
pub use deploy::Deploy;
pub use policy::Policy;
pub use schema::TypeNode;
pub use trigger::Trigger;

/// The shape of the document this module emits. See the module docs.
pub const IR_VERSION: &str = "1";

/// One resolved composition: the deploy artifact of PRD 5.1.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Ir {
    /// The shape of this document — always [`IR_VERSION`] when the resolver
    /// wrote it.
    pub ir_version: &'static str,
    /// The DSL version the composition declares, from its entrypoint
    /// (grammar 1.3).
    pub spec_version: String,
    /// The entrypoint's path, relative to the project root — which is the
    /// entrypoint's own directory, and the root every other path here is
    /// relative to.
    pub entrypoint: String,
    /// The target this artifact was resolved for; `local` when none was named
    /// (grammar 14, Decision D59).
    pub target: String,
    /// Every file the composition was read from, in resolution order: the
    /// entrypoint, then its imports sorted by path, then the deploy file.
    pub sources: Vec<Source>,
    /// `defaults:` — level 3 of the policy resolution chain (grammar 9.3),
    /// with the span of the section that declares it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Spanned<Policy>>,
    /// `state:` — the channel set, which is composition-global in shape while
    /// each flow instance holds its own values (grammar 10.1).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state: Option<Section<Channel>>,
    /// `triggers:` — the **declared** triggers, which are the deployed surface
    /// (grammar 13, Decision D64).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub triggers: Option<Section<Trigger>>,
    /// Every definition, inlined under its typed address (grammar 2.2).
    pub definitions: BTreeMap<String, Definition>,
    /// The active target's deploy layer.
    pub deploy: Deploy,
}

impl Ir {
    /// The artifact as pretty-printed JSON, with a trailing newline.
    ///
    /// Pretty rather than compact because the document is meant to be read and
    /// diffed (PRD 5.1); the ordering rules in the module docs are what make the
    /// bytes reproducible either way.
    ///
    /// # Errors
    ///
    /// Returns the serializer's error. Every value the IR can hold is
    /// representable in JSON: the four surfaces that carry a float — a schema
    /// constraint, a retry `multiplier`, a `default:`/`settings:` literal, and
    /// a plugin-config value — are each checked for infinity and NaN where they
    /// are written, so no float here can be one, and `serde_json` would write
    /// `null` for one that was. The error is surfaced rather than swallowed all
    /// the same.
    pub fn to_json(&self) -> serde_json::Result<String> {
        let mut json = serde_json::to_string_pretty(self)?;
        json.push('\n');
        Ok(json)
    }

    /// The definition at this address, if the composition declares one.
    #[must_use]
    pub fn definition(&self, address: &str) -> Option<&Definition> {
        self.definitions.get(address)
    }
}

/// One section of the composition or of the deploy layer: what it declares,
/// keyed by name, and the region the section itself was written in.
///
/// Sections that are a map of named entries — `state:`, `triggers:`,
/// `placements:`, `event_sources:` — are all this one shape. The two that are
/// not still carry their span the same way: `defaults:` is a single policy
/// block, so it is a [`Spanned<Policy>`](Spanned), and `storage_backends:` has
/// two maps rather than one, so it is [its own
/// struct](deploy::StorageBackends). See the module docs on spans.
///
/// An `Option<Section<_>>` that is `None` means the section was **not
/// declared**, which is not the same as one that declares nothing.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Section<T> {
    /// The entries, sorted by name: a section's declaration order comes from
    /// file layout, which the artifact may not depend on (see the module docs).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub entries: BTreeMap<String, T>,
    /// The section's own span.
    pub span: Span,
}

impl<T> Section<T> {
    /// The entry declared under this name, if the section declares one.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<&T> {
        self.entries.get(name)
    }

    /// How many entries the section declares.
    #[must_use]
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the section declares nothing. A declared section may be empty;
    /// an absent one is `None` rather than an empty `Section`.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// One file the composition was read from.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Source {
    /// The path, relative to the project root, `/`-separated.
    pub path: String,
    /// What the file is to the composition.
    pub role: SourceRole,
}

/// How a file entered the composition (grammar 1.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceRole {
    /// The spec file named on the command line: the only one that may declare
    /// `imports:` (Decision D1).
    Entrypoint,
    /// A spec file reached from the entrypoint's `imports:`.
    Import,
    /// The active target's `deploy/<target>.yml`, which is selected rather than
    /// imported (grammar 14).
    Deploy,
}

/// One state channel: a type node plus the channel-only `reduce` key
/// (grammar 10.1). The channel's `default:` lives on its type node.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Channel {
    /// The channel name, repeated from the key it sits under, with its span.
    pub name: Spanned<Ident>,
    /// The channel's type.
    #[serde(rename = "type")]
    pub ty: TypeNode,
    /// `reduce:` — absent means unreduced: single-writer, sequential, and a
    /// compile error to write from a concurrent context (grammar 10.2).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reduce: Option<Reduce>,
    /// The whole entry's span.
    pub span: Span,
}
