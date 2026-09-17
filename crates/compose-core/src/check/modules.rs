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
    harness_pins_agree(ctx);
}

/// A `module:` dependency may not contradict a harness SDK this composition
/// pins (grammar 6.1, 8.9, PRD resolved q49, q57).
///
/// `parse::binding` already refuses a pin that contradicts the generated
/// project's own set, and that check reads a **constant**: the runtime's pins
/// are the same in every project. A harness SDK's are not — they are pinned only
/// where some `coder:` node binds that harness — so whether a given package is
/// already held is a question about the whole composition, which is what puts
/// this rule here beside the one about two tools disagreeing.
///
/// The repair is the same one that rule gives: name the version the project
/// already holds, or drop the pin. The manifest carries one version of a package
/// and the artifact ships no lockfile to reconcile two.
///
/// **One pin is one finding, however many harnesses hold the package.**
/// `@modelcontextprotocol/sdk` is pinned by `cc` *and* by `codex` — the two SDKs
/// share a peer — so a composition binding both would otherwise report one
/// mistake twice, with one span, one message and two harness names in the help.
/// A form the target cannot take is one mistake (`check/bindings.rs`), so the
/// harnesses that hold the package are gathered into the help of a single
/// refusal instead: which of them brought it is context for the repair, not a
/// second complaint about it.
fn harness_pins_agree(ctx: &mut Ctx) {
    let bound = crate::codegen::harness::bound(ctx.ir);
    if bound.is_empty() {
        return;
    }
    let mut refusals: Vec<Diagnostic> = Vec::new();
    for (address, module) in bindings(ctx.ir) {
        for dependency in &module.dependencies {
            let package = dependency.package.value.as_str();
            // Every bound harness that brings this package, and the version they
            // bring it at — one version, because `codegen::project` refuses to
            // emit a manifest for two and a gate holds the table to it
            // (`runtime_and_harness_pins`).
            let mut holders: Vec<&'static str> = Vec::new();
            let mut pinned: Option<&'static str> = None;
            for harness in &bound {
                let Some((_, held)) = crate::codegen::harness::pins_of(*harness)
                    .iter()
                    .find(|(held, _)| *held == package)
                else {
                    continue;
                };
                holders.push(harness.as_str());
                pinned = Some(held);
            }
            let Some(pinned) = pinned else {
                continue;
            };
            if pinned == dependency.version.value {
                continue;
            }
            let binding = holders
                .iter()
                .map(|name| format!("`harness: {name}`"))
                .collect::<Vec<String>>()
                .join(" or ");
            refusals.push(
                Diagnostic::error(
                    DiagnosticCode::InvalidDependency,
                    dependency.version.span.clone(),
                    format!(
                        "`{address}` pins `{package}` to `{}`, which this project already holds at `{pinned}`",
                        dependency.version.value
                    ),
                )
                .with_help(format!(
                    "a `coder:` node binding {binding} pins `{package}` at `{pinned}`, and the generated `package.json` holds one version of it (grammar 8.9, PRD resolved q57)"
                )),
            );
        }
    }
    for refusal in refusals {
        ctx.push(refusal);
    }
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
///
/// **And one tree.** The same rule covers a path *under* another — `sign.ts`
/// and `sign.ts/helper.ts` — for the reason the emitted-name rule does
/// (`parse::lexical::path_conflict`): one of them would have to be a file and a
/// directory at once, so the scaffold writes one and then fails to make a
/// directory for the other, and no tar could carry the pair either. Every
/// spelling of "these two cannot share a checkout" is one predicate and one
/// refusal.
fn one_file_one_tool(ctx: &mut Ctx) {
    let mut seen: Vec<(&str, &Spanned<String>)> = Vec::new();
    let mut collisions: Vec<Diagnostic> = Vec::new();
    for (address, module) in bindings(ctx.ir) {
        let path = module.path.value.as_str();
        let conflict = seen.iter().find_map(|(declared_by, first_path)| {
            crate::parse::lexical::path_conflict(&first_path.value, path)
                .map(|kind| (*declared_by, *first_path, kind))
        });
        match conflict {
            Some((declared_by, first_path, kind)) => {
                use crate::parse::lexical::PathConflict;
                let message = match kind {
                    PathConflict::Same => {
                        format!("`{address}` and `{declared_by}` are both implemented by `{path}`")
                    }
                    PathConflict::Cased => format!(
                        "`{address}` is implemented by `{path}`, which differs only in case from `{}` — `{declared_by}`'s",
                        first_path.value
                    ),
                    PathConflict::Nested => {
                        let relation = if path.len() > first_path.value.len() {
                            "is inside"
                        } else {
                            "contains"
                        };
                        format!(
                            "`{address}` is implemented by `{path}`, which {relation} `{}` — `{declared_by}`'s",
                            first_path.value
                        )
                    }
                };
                collisions.push(
                    Diagnostic::error(
                        DiagnosticCode::InvalidModulePath,
                        module.path.span.clone(),
                        message,
                    )
                    .with_label(first_path.span.clone(), "first bound here")
                    .with_help(
                        "an authored module is typed against the tool it implements and scaffolded from that tool's schemas, so one file answers to one contract — and no checkout holds two paths that are one file on macOS, or a name that is a file for one tool and a directory for another: give each tool its own `.ts`, and share what they have in common through a module both import (grammar 6.1)",
                    ),
                );
            }
            None => seen.push((address, &module.path)),
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
