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
//! Decision D55), [`Ir::state`] and [`Ir::triggers`] (sorted by name), the
//! deploy layer's placements, aliases, and event sources, a model's
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
    /// `defaults:` — level 3 of the policy resolution chain (grammar 9.3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub defaults: Option<Policy>,
    /// `state:` — the channel set, which is composition-global in shape while
    /// each flow instance holds its own values (grammar 10.1).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub state: BTreeMap<String, Channel>,
    /// `triggers:` — the **declared** triggers, which are the deployed surface
    /// (grammar 13, Decision D64).
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub triggers: BTreeMap<String, Trigger>,
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
    /// representable in JSON — the parser refuses non-finite numbers where they
    /// are written, so no float here can be one — but the error is surfaced
    /// rather than swallowed.
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
