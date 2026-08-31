//! The `module:` binding's two rules that one definition cannot decide
//! (grammar 6.1, PRD resolved q48 and q49).
//!
//! Everything else about the binding is settled where it is written: the path's
//! form, the fence around the project root, a name `build` emits, a version that
//! is not exact, a package the generated project already pins — all of them are
//! one file against a constant, so `parse/binding.rs` owns them (grammar
//! Appendix B). What is left needs more:
//!
//! * **one authored file implements one tool.** A stub is scaffolded from a
//!   tool's schemas and typed against them, so two bindings naming one file are
//!   two contracts over one module.
//! * **the dependency set agrees with itself.** Two tools pinning one package at
//!   two versions is one `package.json` with two answers, and the artifact
//!   carries no lockfile to reconcile them. That needs every tool of the
//!   composition, so it runs here, in the validator.
//! * **the file is there.** That needs the *filesystem*, which no other check in
//!   this crate touches — [`check`](super::check()) reads an [`Ir`] and nothing
//!   else, and it is called from `run`, `serve` and `build` as well as from
//!   `validate`. So it is [`missing`], a pass of its own, run by the two verbs
//!   whose answer is a verdict: `validate`, and `build --check`.
//!
//! A plain `build` deliberately does **not** run [`missing`]: an absent module
//! is what it scaffolds (PRD resolved q48), and a build that refused first could
//! never write the stub whose absence it was refusing over. That asymmetry is
//! the whole reason this is not a [`check`](super::check()) rule.

use std::collections::BTreeMap;
use std::path::Path;

use crate::diag::{Diagnostic, DiagnosticCode, Diagnostics, Spanned};
use crate::ir::Ir;
use crate::ir::binding::Module;
use crate::ir::definition::DefinitionBody;
use crate::ir::flow::ToolImplementation;

use super::Ctx;

/// Every `module:` binding the composition declares, by the tool's address, in
/// address order.
pub(crate) fn bindings(ir: &Ir) -> Vec<(&str, &Module)> {
    ir.definitions
        .iter()
        .filter_map(|(address, definition)| match &definition.body {
            DefinitionBody::Tool(tool) => match &tool.implementation {
                ToolImplementation::Module { module } => Some((address.as_str(), module)),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The two rules the whole composition decides: one file per tool, and one
/// version per package.
pub(crate) fn check(ctx: &mut Ctx) {
    one_file_one_tool(ctx);
    dependencies_agree(ctx);
}

/// One authored file implements one tool (grammar 6.1).
///
/// A file is scaffolded from a tool's schemas, typed against them, and imported
/// by the registry seam under that tool's contract, so two bindings naming one
/// file are two contracts over one default export — and only one of them could
/// ever be the file `build` wrote. The refusal names both, because either is the
/// one to change.
///
/// **One file** is decided case-insensitively, for the reason
/// `parse::binding`'s emitted-name rule is: these paths become file names, and
/// macOS and Windows hold `Sign.ts` and `sign.ts` in one place — so on such a
/// host the second tool's contract would be bound to the first tool's
/// implementation, the scaffold for it never written because the first one is
/// already there. A composition that means two files on one machine and one on
/// another is refused rather than resolved differently per host.
fn one_file_one_tool(ctx: &mut Ctx) {
    let mut first: BTreeMap<String, (&str, &Spanned<String>)> = BTreeMap::new();
    let mut collisions: Vec<Diagnostic> = Vec::new();
    for (address, module) in bindings(ctx.ir) {
        let path = module.path.value.as_str();
        match first.get(&path.to_ascii_lowercase()) {
            Some((declared_by, first_path)) => {
                let message = if first_path.value == module.path.value {
                    format!("`{address}` and `{declared_by}` are both implemented by `{path}`")
                } else {
                    format!(
                        "`{address}` is implemented by `{path}`, which differs only in case from `{}` — `{declared_by}`'s",
                        first_path.value
                    )
                };
                collisions.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidModulePath,
                        module.path.span.clone(),
                        message,
                    )
                    .with_label(first_path.span.clone(), "first bound here")
                    .with_help(
                        "an authored module is typed against the tool it implements and scaffolded from that tool's schemas, so one file answers to one contract — and on macOS and Windows two spellings that differ only in case are one file: give each tool its own `.ts`, and share what they have in common through a module both import (grammar 6.1)",
                    ),
                );
            }
            None => {
                first.insert(path.to_ascii_lowercase(), (address, &module.path));
            }
        }
    }
    for collision in collisions {
        ctx.push(collision);
    }
}

/// Two tools may not pin one package at two versions (PRD resolved q49).
///
/// The generated `package.json` is one document with one entry per package, and
/// no lockfile ships beside it to reconcile a disagreement, so a composition
/// that declares two versions of a package has no build at all. The refusal
/// names **both** tools, because either one of them could be the one to change.
///
/// Identical pins are not a conflict — two tools importing the same library at
/// the same version is the ordinary case, and they fold into one entry.
fn dependencies_agree(ctx: &mut Ctx) {
    // The first declaration of each package, in address order, so the diagnostic
    // lands on the second one and points back at the first — the same shape a
    // duplicate import and a repeated `expect_exit` entry take.
    let mut first: BTreeMap<&str, (&str, &Spanned<String>)> = BTreeMap::new();
    let mut conflicts: Vec<Diagnostic> = Vec::new();
    for (address, module) in bindings(ctx.ir) {
        for dependency in &module.dependencies {
            let package = dependency.package.value.as_str();
            match first.get(package) {
                Some((declared_by, version)) if version.value != dependency.version.value => {
                    conflicts.push(
                        Diagnostic::error(
                            DiagnosticCode::InvalidDependency,
                            dependency.version.span.clone(),
                            format!(
                                "`{address}` pins `{package}` to `{}` and `{declared_by}` pins it to `{}`",
                                dependency.version.value, version.value
                            ),
                        )
                        .with_label(version.span.clone(), "first pinned here")
                        .with_help(format!(
                            "the generated `package.json` holds one version of `{package}` and the artifact ships no lockfile to reconcile two, so both tools have to name one release (PRD resolved q49)"
                        )),
                    );
                }
                Some(_) => {}
                None => {
                    first.insert(package, (address, &dependency.version));
                }
            }
        }
    }
    for conflict in conflicts {
        ctx.push(conflict);
    }
}

/// Every `module:` binding whose file is not on disk, as diagnostics naming the
/// repair (grammar 6.1, PRD resolved q48).
///
/// `root` is the project root — the entrypoint's own directory, which every
/// path in the [`Ir`] is relative to.
///
/// The repair is a **command**, not an edit: `agent-compose build` writes a
/// typed stub for a referenced module it cannot find, once, and never rewrites
/// it. So the diagnostic sends the reader there rather than describing a file
/// for them to type out, and `build --check` reports the same sentence, because
/// a committed project whose implementation is missing is a project that does
/// not run.
#[must_use]
pub fn missing(ir: &Ir, root: &Path) -> Vec<Diagnostic> {
    let mut found = Diagnostics::new();
    for (address, module) in bindings(ir) {
        if present(root, &module.path.value) {
            continue;
        }
        found.push(
            Diagnostic::error(
                DiagnosticCode::IoError,
                module.path.span.clone(),
                format!(
                    "`{address}` is implemented by `{}`, which does not exist",
                    module.path.value
                ),
            )
            .with_help(
                "run `agent-compose build`: it scaffolds a referenced module it cannot find — the typed signature, the contract as a doc comment, and a body that throws — once, and never writes that file again (grammar 6.1, PRD resolved q48)",
            ),
        );
    }
    found.sort();
    found.into_vec()
}

/// Whether one `module:` binding's implementation is **there**, at
/// `root`-relative `path`.
///
/// One predicate for the two verbs that ask, which is the point of it being
/// public: [`missing`] refuses a binding whose file is not there and names
/// `agent-compose build` as the repair, and `build` writes the stub for exactly
/// the files this answers `false` for. Two spellings of "there" would let
/// `validate` name a repair the build then declines to make — a directory at the
/// path is the case that separates `Path::exists` from this.
#[must_use]
pub fn present(root: &Path, path: &str) -> bool {
    at(root, path).is_file()
}

/// Where a `/`-separated project-relative path lands on this host.
fn at(root: &Path, path: &str) -> std::path::PathBuf {
    let mut full = root.to_path_buf();
    for segment in path.split('/') {
        full.push(segment);
    }
    full
}
