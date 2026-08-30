//! The resolver: an entrypoint in, one flat IR artifact out.
//!
//! Multi-file composition is authoring UX (PRD 5.1). This pass is where it stops
//! being one: it follows the entrypoint's `imports:`, parses every file, binds
//! every name to what it names, and flattens the result into a single
//! self-contained [`Ir`] document keyed by typed address.
//!
//! # What it decides
//!
//! Everything the parser reports is decidable from one file. Everything here is
//! decidable from **names, files, and addresses** across the composition:
//!
//! * **Files** (grammar 1.2, 1.4, 14) — the entrypoint is a spec file; imports
//!   are relative paths resolving inside the project root, unique after
//!   normalization, naming neither the entrypoint nor a deploy file; an imported
//!   file declares no `imports:` of its own (Decision D1); the entrypoint
//!   declares `version:` and every other file's, when present, matches it
//!   (Decision D4); the active target's `deploy/<name>.yml` exists when the
//!   target is named, and carries no `storage_backends:` when it is `local`
//!   (Decision D87).
//! * **Sections** (grammar 1.5) — `defaults:`, `state:`, and `triggers:` are
//!   declared in at most one file each, and every typed address is defined
//!   exactly once, both reported at the second site with the first labelled
//!   (Decision D2).
//! * **References** (grammar 2.3, 2.4) — every typed address resolves to a
//!   definition in a namespace its position accepts, a model route's members
//!   are direct models rather than routes of their own (grammar 12.2), every
//!   flow-local node id an edge or a control transfer names is a node of that
//!   flow, and the two target-dependent bindings — a store's `backend:` alias
//!   and an `event` trigger's `source:` — resolve in the active target
//!   (grammar 11.3, 13.5). An alias that resolves is checked one step further:
//!   the provider behind it must serve the kind of the store that named it,
//!   which is decidable here and nowhere earlier because an alias declares no
//!   kind of its own (grammar 14.3).
//!
//! # What it leaves alone
//!
//! Everything that needs a *graph* or a *type*: SCC termination, routing
//! exhaustiveness, balanced convergence, `map.over` dominance, schema
//! compatibility, reducer-write rules, trigger input-binding compatibility, CEL
//! parsing and typing, and undefined state channels. Those read the IR this pass
//! produces, which is why the IR carries a span on everything they can report
//! against — the validator never re-parses a file.
//!
//! # The artifact and the diagnostics
//!
//! [`Resolution::ir`] is `Some` exactly when nothing was **rejected**: a
//! composition with an error has no artifact, because half of one would send
//! the next pass chasing failures that are really this one's. The distinction
//! costs a word and stays true the day this pass emits its first
//! [warning](Diagnostic::warning) — a composition accepted with a caveat has
//! both an artifact and a diagnostic. Diagnostics come back in source order —
//! by file, then by position — so a report reads top to bottom whatever order
//! the passes visited constructs in.

pub(crate) mod files;
pub(crate) mod index;
pub(crate) mod lower;
pub(crate) mod references;
pub(crate) mod target;

use std::path::Path;

use crate::diag::{Diagnostic, Diagnostics};
use crate::ir::Ir;

/// The target resolved when the command names none: `local`, the built-in
/// zero-infra target (grammar 14, Decision D87).
pub const DEFAULT_TARGET: &str = "local";

/// The result of resolving one composition.
#[derive(Clone, Debug)]
pub struct Resolution {
    /// The artifact, when the composition resolved cleanly.
    pub ir: Option<Ir>,
    /// Everything the parser and the resolver found, in source order.
    pub diagnostics: Vec<Diagnostic>,
}

impl Resolution {
    /// Whether any diagnostic rejects the composition.
    #[must_use]
    pub fn has_errors(&self) -> bool {
        self.diagnostics.iter().any(Diagnostic::is_error)
    }

    /// Split into the artifact and its diagnostics.
    #[must_use]
    pub fn into_parts(self) -> (Option<Ir>, Vec<Diagnostic>) {
        (self.ir, self.diagnostics)
    }
}

/// Resolve the composition rooted at `entrypoint` for the built-in `local`
/// target.
///
/// The entrypoint's directory is the **project root**: every import resolves
/// against it, and every file is read — and reported, and recorded in the IR —
/// under its path relative to it (grammar 1.4).
pub fn resolve(entrypoint: impl AsRef<Path>) -> Resolution {
    resolve_with_target(entrypoint, DEFAULT_TARGET)
}

/// Resolve the composition rooted at `entrypoint` for a named target, loading
/// `deploy/<target>.yml` (grammar 14).
pub fn resolve_with_target(entrypoint: impl AsRef<Path>, target: &str) -> Resolution {
    let mut diagnostics = Diagnostics::new();
    let ir = run(entrypoint.as_ref(), target, &mut diagnostics);

    // Lowering is total on a composition nothing was rejected in (see `lower`),
    // so an artifact may only be missing because something *was* rejected. A
    // silent `None` would leave a caller with no artifact and no reason for it.
    debug_assert!(
        ir.is_some() || diagnostics.has_errors(),
        "resolution produced neither an artifact nor an error"
    );

    diagnostics.sort();
    Resolution {
        ir,
        diagnostics: diagnostics.into_vec(),
    }
}

fn run(entrypoint: &Path, target: &str, diagnostics: &mut Diagnostics) -> Option<Ir> {
    let composition = files::load(entrypoint, target, diagnostics)?;
    let index = index::build(&composition, diagnostics);
    // Names are resolved only against a composition that is all there. An
    // import that did not load takes its definitions with it, and every
    // reference into that file would then be reported as undefined — a page of
    // consequences for the one cause already named (see `Composition::complete`).
    if composition.complete {
        references::check(&index, diagnostics);
        target::check(&composition, &index, diagnostics);
    }
    if diagnostics.has_errors() {
        return None;
    }
    lower::composition(&composition, &index)
}
