//! `agent-compose build`: the emitted tree on disk, the check that it still
//! matches, and the two writes a build makes that are not its own code.
//!
//! `compose_core::emit` answers a set of `(path, contents)` pairs and touches
//! nothing (see `compose_core::codegen`). This module is the half that does:
//! writing that set into a directory, and — under `--check` — comparing it
//! against what is already there without writing anything.
//!
//! # The manifest is the boundary
//!
//! PRD resolved q47: **`build` overwrites and `--check`s exactly its own emitted
//! file list, and never writes, deletes, or reports anything else under the
//! output directory.** `compose_core::codegen::EMITTED_PATHS` is that list, and
//! it is the whole of the compiler's claim on a directory. So:
//!
//! * **writing** replaces every emitted file and removes nothing at all. A file
//!   the compiler does not emit is not the compiler's, wherever it sits — a
//!   `node_modules/`, a `.env`, and the hand-authored TypeScript a `module:`
//!   binding names are all the same case, and the last of them is why the rule
//!   has to be stated this way rather than "everything under `src/` is ours"
//!   (grammar 6.1).
//! * **checking** reports a file that is missing, one whose bytes differ, and
//!   one of this compiler's own files sitting at a claimed path that *this*
//!   build does not emit — the three ways a committed project can stop matching
//!   its spec (PRD §8: "hand-edited generated code forks the source of truth",
//!   mitigated by "`build --check` in CI"). The third exists because the
//!   emission set is target-dependent while `EMITTED_PATHS` is not: a target
//!   that stops declaring `package_registry:` stops emitting `.npmrc` and
//!   `bunfig.toml` (grammar 14.6, PRD resolved q59), and the previous build's
//!   copies stay where they are — still carrying the generated-file header, so
//!   still reading as this compiler's, and still telling every `bun install` in
//!   that directory to go through a mirror the spec no longer names. That is a
//!   disagreement about a file the compiler claims, which is what drift is.
//!   A file **outside** [`EMITTED_PATHS`], or one at a claimed path that this
//!   compiler did not write, is still nobody's business but its author's.
//! * neither **removes** anything, anywhere — so a stale claimed path is
//!   reported and never deleted, and the help under it says so rather than
//!   promising a rebuild that would not touch it.
//!
//! # The first write that is not the compiler's code: a carried implementation
//!
//! A `module:` binding's implementation, copied in at the same project-relative
//! path (`GeneratedProject::carried`, PRD resolved q49). The *artifact* is "what
//! `build` wrote plus the authored files the spec references", and a worker
//! holds nothing but the artifact, so the tree the hub tars has to contain the
//! file — and `src/modules.ts`, which imports it, has to resolve inside that
//! tree rather than back out into somebody's checkout.
//!
//! It is a copy rather than a claim on the name. What the author edits is the
//! project-side original beside `main.yml`, which is where `validate` looks for
//! it and where a scaffold is written; the copy under `--out` is a build
//! artifact like every generated file beside it, is compared by [`check`] for
//! exactly that reason, and carries no generated-file header because the bytes
//! are the author's. The boundary is unmoved: the composition decides which
//! authored files travel, and the ones it does not name are still nobody's
//! business but the author's.
//!
//! What that costs, said plainly: a module an older compiler release wrote and
//! this one no longer emits stays where it is, importable, until somebody
//! deletes it. The alternative was a walk of `src/` that removed whatever
//! carried the generated-file header — which is what made a build a read of its
//! own previous output, and is the machinery q47 removes so that authored code
//! can live in the same tree with no marker protocol guarding it.
//!
//! # The two things a build refuses over
//!
//! **An emitted name an authored file already holds.** `--out` can name any
//! path — an existing Node project, an ejected copy, `.` — and a first build
//! into one that replaced `package.json`, `README.md` or `src/graph.ts` because
//! they happened to share a name would destroy somebody's project and report
//! nothing. [`claimed`] asks the question once, for the whole directory: a
//! directory holding even one file this compiler wrote is one it has built into,
//! and every emitted path in it is a build artifact. A directory holding none of
//! them may be anything, and an emitted path already occupied there stops the
//! write, naming both sides.
//!
//! The question is about the *directory* rather than about each file on purpose.
//! Inside a directory this compiler built, a generated file somebody edited —
//! header and all — is exactly what PRD §8 says to regenerate and what the drift
//! report sends the reader to `build` for; a per-file rule would answer that
//! instruction with a refusal.
//!
//! **A carried path is under the same rule, and that is deliberate.** [`write`]
//! puts an authored file down at its project-relative name too, so a directory
//! this compiler has never built into is scanned for those names as well
//! ([`not_ours`]) — a first build into somebody's tree does not put a
//! composition's `src/tools/sign.ts` over a `src/tools/sign.ts` of theirs. Inside
//! a directory it *has* built into, the scan is skipped for a carried path
//! exactly as it is for an emitted one: the copy there is a build artifact, the
//! drift report compares it, and "run `agent-compose build`" is the remedy it
//! prints — which a refusal over the copy would make a lie. The author's file is
//! the project-side original, and nothing here writes to that tree but
//! [`scaffold`].
//!
//! # The second: a scaffold, in the one tree that is not the output directory
//!
//! A `module:` binding names authored TypeScript, and `build` **scaffolds** it
//! when it is absent: once, with the typed signature and a body that throws
//! ([`scaffold`], `compose_core::codegen::authored`). A scaffold is not an
//! emitted file — it is not in the manifest, nothing compares it against a
//! template, and a rebuild that finds it present writes nothing. `build` never
//! reads what is in it to decide what to write.
//!
//! It goes in the **project**, not in `--out`, and that is not an arbitrary
//! choice: a `module:` path is project-relative, like an `imports:` entry
//! (grammar 6.1, 1.4), and `agent-compose validate` — which takes no `--out` at
//! all — is required to refuse a binding whose file is missing. One place the
//! path can mean, and it is the entrypoint's own directory. The path is the same
//! *inside the artifact*, which is exactly why a binding may not name a file the
//! emitter writes — under that rule the authored file's project-relative name
//! and its name in the shipped tree are one string with no collision possible.
//!
//! Two trees and two writes, in this order: [`scaffold`] into the project, then
//! [`write`] into `--out`. The order is forced — the emitter hashes the authored
//! bytes into the artifact, so they have to be on disk before there is a project
//! to write — and it means a build refused over an occupied `--out` may have
//! left a stub behind. That is not a half-done write to undo: a stub is the
//! author's file, in the author's tree, written once, and the build that
//! follows a fixed `--out` writes it no second time.
//!
//! `--check` does not scaffold, and does not stay silent either: a missing
//! implementation is refused before this module is reached, by
//! `compose_core::check_modules`, with the diagnostic naming `build` as the
//! repair — the same sentence `validate` prints.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use compose_core::GeneratedProject;
use compose_core::codegen::authored::Scaffold;

/// One way the directory disagrees with what the compiler would emit.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Drift {
    /// The path, relative to the output directory, `/`-separated.
    pub(crate) path: String,
    /// How it disagrees.
    pub(crate) state: State,
}

/// The three ways a generated project can stop matching its spec.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum State {
    /// The compiler emits it and the directory does not have it.
    Missing,
    /// Both have it and the bytes differ.
    Differs,
    /// The directory holds one of this compiler's files at a path
    /// [`compose_core::codegen::EMITTED_PATHS`] claims, and **this** build does
    /// not emit it.
    ///
    /// An earlier build of this target did, before the deploy file stopped
    /// asking for it (grammar 14.6's `package_registry:` is the one key whose
    /// presence moves a path in and out of the emission set today). Nothing
    /// deletes it — PRD resolved q47 — so `--check` is where it is said out
    /// loud, and [`Self::Stale`] is a different sentence from
    /// [`Self::Differs`] because the remedy is a removal rather than a rebuild.
    Stale,
}

impl State {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Differs => "differs",
            Self::Stale => "stale",
        }
    }

    /// The sentence a human report writes for it.
    const fn explanation(self) -> &'static str {
        match self {
            Self::Missing => "is not in the output directory",
            Self::Differs => "differs from what the spec produces",
            Self::Stale => "is an earlier build's, and this target does not emit it",
        }
    }
}

impl fmt::Display for Drift {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "`{}` {}", self.path, self.state.explanation())
    }
}

/// What one successful write did.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Written {
    /// How many files the emitter produced.
    pub(crate) files: usize,
    /// How many authored files the tree carries beside them (PRD resolved q49).
    pub(crate) carried: usize,
}

/// Why a write did not happen.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The filesystem said no.
    Io(io::Error),
    /// The write would have replaced files this compiler did not write. The
    /// paths, sorted.
    NotOurs(Vec<String>),
}

impl From<io::Error> for Refusal {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The marker every generated file opens with (`compose_core::codegen::header`).
///
/// Ownership of an emitted **name** is claimed by *this* string rather than by
/// the path existing: it is the only evidence on disk that a file came from a
/// spec. A scaffolded module deliberately does not carry it — it is the
/// author's, and the compiler's only interaction with it is asking whether it is
/// there.
const GENERATED_MARKER: &str = "generated by agent-compose";

/// Write the whole tree into `out`: the emission set, and the authored files
/// the composition references beside it.
///
/// Both, because both are what the artifact is (PRD resolved q49, and see the
/// module header). The authored bytes came from the project root, and
/// [`scaffold`] is what put a missing one there before the emitter was asked for
/// a tree at all.
///
/// A refusal is [`Refusal::NotOurs`] naming the paths, and nothing has been
/// written when it happens, because the scan runs first.
pub(crate) fn write(project: &GeneratedProject, out: &Path) -> Result<Written, Refusal> {
    let foreign = not_ours(project, out)?;
    if !foreign.is_empty() {
        return Err(Refusal::NotOurs(foreign));
    }

    for file in project.artifact() {
        let path = at(out, &file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.contents)?;
    }

    Ok(Written {
        files: project.files().len(),
        carried: project.carried().len(),
    })
}

/// Write into `root` the authored implementations that are not there yet.
///
/// **Before the emission set, and in a different tree.** `root` is the project
/// the composition was read from, which is where a `module:` path is resolved
/// and therefore where an absent implementation is written (see the module
/// header). It runs first because [`compose_core::emit`] hashes those files into
/// the artifact, so they have to exist before there is a project to write — a
/// build that scaffolded afterwards would emit an artifact list naming a file
/// that was not there when the list was made.
///
/// A scaffold is a write `build` makes **once**: a rebuild finds the file there
/// and leaves the author's bytes alone, without reading them (PRD resolved q48).
/// The answer is what was written, as `(path, tool)`, sorted by path — reported,
/// so "the build also wrote you a stub" is something the run says rather than
/// something a later `git status` discovers.
pub(crate) fn scaffold(scaffolds: &[Scaffold], root: &Path) -> io::Result<Vec<(String, String)>> {
    let mut scaffolded: Vec<(String, String)> = Vec::new();
    for scaffold in scaffolds {
        let path = at(root, &scaffold.path);
        // The **same** predicate `validate` and `build --check` refuse over
        // (`compose_core::module_is_present`), rather than a second reading of
        // "there". A directory at the path is the case that separates them: with
        // `Path::exists` here, `validate` would name this command as the repair
        // and this command would write nothing and then fail on the read.
        if compose_core::module_is_present(root, &scaffold.path) {
            continue;
        }
        if path.exists() {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!(
                    "cannot scaffold `{}` for `{}`: something that is not a file is already there",
                    scaffold.path, scaffold.tool
                ),
            ));
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &scaffold.contents)?;
        scaffolded.push((scaffold.path.clone(), scaffold.tool.clone()));
    }
    scaffolded.sort();
    Ok(scaffolded)
}

/// What a [`write`] would refuse over, answered without writing: the payload
/// [`Refusal::NotOurs`] would carry, sorted, and empty when the write would go
/// through.
///
/// `--check` needs this for a reason `write` does not have: its report ends in a
/// remedy, and "run `agent-compose build`" is the wrong one for a directory
/// `build` declines to touch. Reading the answer off the same scan the write
/// makes is what keeps the help from promising something the next command
/// refuses.
pub(crate) fn not_ours(project: &GeneratedProject, out: &Path) -> io::Result<Vec<String>> {
    // In a directory this compiler has built into, every emitted path is its
    // own to replace — including one whose header an edit removed, which is what
    // PRD §8 says to regenerate.
    if claimed(project, out)? {
        return Ok(Vec::new());
    }
    let mut foreign: BTreeSet<String> = BTreeSet::new();
    for file in project.files() {
        if occupied(&at(out, &file.path))? {
            foreign.insert(file.path.clone());
        }
    }
    // A **carried** file is the author's own bytes, so the question is a
    // different one: the refusal exists to stop this compiler putting its output
    // over somebody's file, and writing a file the bytes it already holds
    // destroys nothing. That is not a nicety — `--out` may name the project
    // itself, where the copy's destination *is* its source, and a rule asking
    // only "does something exist here" would refuse every such build.
    for file in project.carried() {
        if replaced(&at(out, &file.path), &file.contents)? {
            foreign.insert(file.path.clone());
        }
    }
    Ok(foreign.into_iter().collect())
}

/// Whether writing `contents` at this path would replace something else.
///
/// Nothing there, or exactly these bytes already, is not a replacement.
fn replaced(path: &Path, contents: &str) -> io::Result<bool> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(bytes != contents.as_bytes()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Whether the output directory already holds something this compiler wrote.
///
/// The evidence is [`GENERATED_MARKER`] at a path the emitter produces: a
/// directory holding even one of this compiler's files is one it has built into,
/// and every emitted path in it is a build artifact rather than somebody's file
/// that happens to share a name. A directory holding none of them may be
/// anything at all, and is treated as if it were the user's.
fn claimed(project: &GeneratedProject, out: &Path) -> io::Result<bool> {
    for file in project.files() {
        match generated(&at(out, &file.path)) {
            Ok(true) => return Ok(true),
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// Whether an emitted path is already held by a file this compiler did not
/// write.
///
/// Nothing there is the ordinary case — a first build into an empty directory —
/// and is not an obstacle.
fn occupied(path: &Path) -> io::Result<bool> {
    match generated(path) {
        Ok(ours) => Ok(!ours),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

/// Whether a file on disk carries the generated-file header.
///
/// Only the first bytes are read: the marker is on the first line of every
/// emitted file, and something large that is not ours should not be loaded into
/// memory to find that out. A file that is not UTF-8 is not one this compiler
/// wrote.
fn generated(path: &Path) -> io::Result<bool> {
    use std::io::Read;

    let mut head = [0u8; 512];
    let mut file = std::fs::File::open(path)?;
    let mut filled = 0;
    while filled < head.len() {
        match file.read(&mut head[filled..])? {
            0 => break,
            read => filled += read,
        }
    }
    Ok(String::from_utf8_lossy(&head[..filled]).contains(GENERATED_MARKER))
}

/// Compare the tree a build would produce against the directory, writing
/// nothing.
///
/// Answers the drift, sorted by path. Exactly that tree is compared — the
/// emission set and the authored files the composition references, which is what
/// a build writes and what the artifact hash covers; nothing else under the
/// output directory is the compiler's to have an opinion about (see the module
/// header).
///
/// Then one question the tree cannot answer, because it is about a file that is
/// *not* in it: [`stale`]. The emission set is target-dependent
/// (`compose_core::codegen::TARGET_PATHS`) and the compiler's claim on a
/// directory is not, so a path this build does not write can still be one it
/// owns — and this compiler's own file sitting there is drift like any other.
pub(crate) fn check(project: &GeneratedProject, out: &Path) -> io::Result<Vec<Drift>> {
    let mut drift = Vec::new();
    for file in project.artifact() {
        match std::fs::read(at(out, &file.path)) {
            Ok(bytes) if bytes == file.contents.as_bytes() => {}
            Ok(_) => drift.push(Drift {
                path: file.path.clone(),
                state: State::Differs,
            }),
            Err(error) if error.kind() == io::ErrorKind::NotFound => drift.push(Drift {
                path: file.path.clone(),
                state: State::Missing,
            }),
            Err(error) => return Err(error),
        }
    }
    for path in stale(project, out)? {
        drift.push(Drift {
            path,
            state: State::Stale,
        });
    }
    drift.sort();
    Ok(drift)
}

/// The claimed paths in `out` that hold one of this compiler's files and that
/// **this** build does not emit, sorted.
///
/// The claim is [`compose_core::codegen::EMITTED_PATHS`] — every path the
/// compiler owns across every target, which is the same whole claim
/// `parse::binding` refuses a `module:` binding against. What varies per target
/// is the *emission set*, and the difference is exactly where an artifact can
/// keep a file its spec no longer asks for: drop `package_registry:` from a
/// deploy file and the next build stops writing `.npmrc` and `bunfig.toml`
/// (grammar 14.6, PRD resolved q59) without removing the pair the last one
/// wrote.
///
/// The generated-file header is the test, for the reason it is everywhere else
/// in this module: it is the only evidence on disk that a file is this
/// compiler's. An `.npmrc` an author placed in the output directory of a target
/// that emits none is the author's file at a name nothing is writing, and
/// reporting it would be this compiler having an opinion about somebody else's
/// file.
fn stale(project: &GeneratedProject, out: &Path) -> io::Result<Vec<String>> {
    let mut left = Vec::new();
    for path in compose_core::codegen::EMITTED_PATHS {
        if project.file(path).is_some() {
            continue;
        }
        match generated(&at(out, path)) {
            Ok(true) => left.push((*path).to_string()),
            Ok(false) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
    }
    Ok(left)
}

/// Where a `/`-separated generated path lands on this host.
fn at(out: &Path, path: &str) -> PathBuf {
    let mut full = out.to_path_buf();
    full.extend(path.split('/'));
    full
}

/// Where `--out` points when it was not given: `<project>/build/<target>`.
///
/// Per target, because a composition resolves differently under each one
/// (grammar 14's per-target invariant) and two targets sharing a directory would
/// each report the other's output as drift. `<project>` is the entrypoint's own
/// directory, which is the project root every path in the IR is relative to.
pub(crate) fn default_out(entrypoint: &Path, target: &str) -> PathBuf {
    entrypoint
        .parent()
        .unwrap_or_else(|| Path::new(""))
        .join("build")
        .join(target)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project() -> GeneratedProject {
        let resolution = compose_core::resolve(scratch_spec().as_path());
        assert!(resolution.diagnostics.is_empty());
        compose_core::emit(
            &resolution.ir.expect("an artifact"),
            &compose_core::Authored::none(),
        )
    }

    /// A one-file composition on disk, for the emitter to run over.
    fn scratch_spec() -> PathBuf {
        let directory = scratch("spec");
        let entrypoint = directory.join("main.yml");
        std::fs::write(
            &entrypoint,
            "version: \"0.1\"\nstate:\n  draft: { type: string }\n",
        )
        .expect("the entrypoint is writable");
        entrypoint
    }

    /// A composition with one `module:` binding: its artifact, the scaffolds it
    /// asks for, and the **project root** they are written into, which is the
    /// entrypoint's own directory rather than `--out`.
    ///
    /// The project itself is not emitted here, because it cannot be: the
    /// artifact carries the authored file's bytes, and the whole point of the
    /// tests below is what happens before and after those bytes exist. Each one
    /// emits where it means to (see [`built`]).
    fn with_module() -> (compose_core::Ir, Vec<Scaffold>, PathBuf) {
        let directory = scratch("module-spec");
        let entrypoint = directory.join("main.yml");
        std::fs::write(
            &entrypoint,
            r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts
"#,
        )
        .expect("the entrypoint is writable");
        let resolution = compose_core::resolve(entrypoint.as_path());
        assert!(resolution.diagnostics.is_empty());
        let ir = resolution.ir.expect("an artifact");
        let scaffolds = compose_core::codegen::authored::scaffolds(&ir);
        (ir, scaffolds, directory)
    }

    /// The project one artifact emits, reading the authored files out of the
    /// project root — which is what the CLI does between scaffolding and
    /// writing (`main::emitted`).
    fn built(ir: &compose_core::Ir, root: &Path) -> GeneratedProject {
        let authored =
            compose_core::Authored::read(ir, root).expect("every referenced module is readable");
        compose_core::emit(ir, &authored)
    }

    fn scratch(purpose: &str) -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static NEXT: AtomicU32 = AtomicU32::new(0);
        let path = std::env::temp_dir().join(format!(
            "agent-compose-build-{purpose}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).expect("a scratch directory");
        path
    }

    #[test]
    fn a_written_project_checks_clean() {
        let project = project();
        let out = scratch("clean");
        assert_eq!(
            write(&project, &out).expect("the directory is writable"),
            Written {
                files: project.files().len(),
                carried: 0,
            }
        );
        assert_eq!(
            check(&project, &out).expect("the directory is readable"),
            []
        );
    }

    #[test]
    fn an_edited_file_and_a_deleted_one_are_both_drift() {
        let project = project();
        let out = scratch("edited");
        write(&project, &out).expect("writable");
        std::fs::write(out.join("src/state.ts"), "// mine now\n").expect("writable");
        std::fs::remove_file(out.join("src/env.ts")).expect("removable");
        assert_eq!(
            check(&project, &out).expect("readable"),
            [
                Drift {
                    path: "src/env.ts".to_string(),
                    state: State::Missing,
                },
                Drift {
                    path: "src/state.ts".to_string(),
                    state: State::Differs,
                },
            ]
        );
    }

    /// The boundary, in the direction that changed: a file the emitter does not
    /// produce is not the compiler's, wherever it sits. It is not drift, it is
    /// not removed, and it does not stop a rebuild — which is what makes an
    /// authored `src/tools/sign.ts` able to live beside generated modules at all
    /// (PRD resolved q47).
    #[test]
    fn a_file_the_emitter_does_not_produce_is_left_alone_everywhere() {
        let project = project();
        let out = scratch("not-emitted");
        write(&project, &out).expect("writable");

        std::fs::create_dir_all(out.join("src/tools")).expect("writable");
        std::fs::write(out.join("src/tools/sign.ts"), "export default 1;\n").expect("writable");
        // …including one an older compiler release wrote, which the build no
        // longer claims.
        std::fs::write(
            out.join("src/old.ts"),
            "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
        )
        .expect("writable");
        std::fs::create_dir_all(out.join("node_modules/left")).expect("writable");
        std::fs::write(out.join("node_modules/left/index.js"), "//\n").expect("writable");

        assert_eq!(
            check(&project, &out).expect("readable"),
            [],
            "nothing outside the emitted list is drift"
        );
        assert_eq!(
            write(&project, &out).expect("writable"),
            Written {
                files: project.files().len(),
                carried: 0,
            },
            "and nothing outside it is removed"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("src/tools/sign.ts")).expect("readable"),
            "export default 1;\n"
        );
        assert!(out.join("src/old.ts").is_file());
        assert!(out.join("node_modules/left/index.js").is_file());
    }

    /// The other side of that boundary: a **claimed** path is the compiler's
    /// whether or not this target emits it, so one of its own files left at a
    /// name this build does not write is drift (grammar 14.6, PRD resolved q59).
    ///
    /// `project()` declares no `package_registry:`, so `.npmrc` and
    /// `bunfig.toml` are exactly the pair an earlier build of a target that did
    /// would have written and that nothing removes (PRD resolved q47).
    #[test]
    fn a_claimed_file_this_build_does_not_emit_is_drift() {
        let project = project();
        let out = scratch("stale");
        write(&project, &out).expect("writable");
        assert!(
            project.file(".npmrc").is_none() && project.file("bunfig.toml").is_none(),
            "this composition's target declares no `package_registry:`"
        );

        for name in [".npmrc", "bunfig.toml"] {
            std::fs::write(
                out.join(name),
                format!("# {GENERATED_MARKER} 0.0.1 from `main.yml`.\n"),
            )
            .expect("writable");
        }
        assert_eq!(
            check(&project, &out).expect("readable"),
            [
                Drift {
                    path: ".npmrc".to_string(),
                    state: State::Stale,
                },
                Drift {
                    path: "bunfig.toml".to_string(),
                    state: State::Stale,
                },
            ]
        );
        // Reported, never removed, and no obstacle to the rebuild the report
        // does not recommend for it.
        assert_eq!(
            write(&project, &out).expect("writable"),
            Written {
                files: project.files().len(),
                carried: 0,
            },
        );
        assert!(out.join(".npmrc").is_file() && out.join("bunfig.toml").is_file());
    }

    /// …and only one of *its own* files. A claimed name is not a claim on
    /// whatever holds it: the generated-file header is the evidence everywhere
    /// else in this module, and an author's `.npmrc` in the output directory of
    /// a target that emits none is the author's.
    #[test]
    fn a_claimed_name_someone_else_holds_is_not_stale() {
        let project = project();
        let out = scratch("stale-not-ours");
        write(&project, &out).expect("writable");
        std::fs::write(out.join(".npmrc"), "registry=https://npm.example/\n").expect("writable");
        assert_eq!(check(&project, &out).expect("readable"), []);
    }

    /// A directory this compiler never built into is not one it may overwrite —
    /// under `src/` or at the root.
    #[test]
    fn emitted_names_someone_else_holds_stop_the_build() {
        let project = project();
        let out = scratch("not-ours");
        std::fs::create_dir_all(out.join("src")).expect("writable");
        std::fs::write(out.join("src/graph.ts"), "export const mine = 1;\n").expect("writable");
        std::fs::write(
            out.join("package.json"),
            "{\"name\":\"my-real-app\",\"version\":\"3.2.1\"}\n",
        )
        .expect("writable");
        std::fs::write(out.join("README.md"), "# My real project\n").expect("writable");
        // Not an emitted name, so not part of the refusal.
        std::fs::write(out.join("src/mine.ts"), "export const mine = 2;\n").expect("writable");

        let refusal = write(&project, &out).expect_err("the write is refused");
        let Refusal::NotOurs(paths) = refusal else {
            panic!("the refusal names the files, not an io error: {refusal:?}");
        };
        assert_eq!(paths, ["README.md", "package.json", "src/graph.ts"]);
        assert_eq!(
            std::fs::read_to_string(out.join("package.json")).expect("readable"),
            "{\"name\":\"my-real-app\",\"version\":\"3.2.1\"}\n",
            "nothing is overwritten before the scan decides"
        );
        assert!(
            !out.join("src/schemas.ts").exists(),
            "and nothing is written either: the refusal is total"
        );

        // `--check` was always right about these: it reports them as drift
        // rather than replacing them, and `not_ours` is what tells the report
        // that `build` would refuse rather than settle it.
        let drift = check(&project, &out).expect("readable");
        assert!(
            drift.contains(&Drift {
                path: "package.json".to_string(),
                state: State::Differs,
            }),
            "{drift:?}"
        );
        assert_eq!(
            not_ours(&project, &out).expect("readable"),
            ["README.md", "package.json", "src/graph.ts"]
        );
    }

    /// The compiler's own output is its to replace, header or no header.
    ///
    /// The refusal above is about the *directory*, and this is the other half of
    /// that: inside one this compiler has built into, a generated file someone
    /// edited — taking the header with it — is what PRD §8 says to regenerate and
    /// what the drift report sends the reader to `build` for. A per-file rule
    /// would answer that instruction with a refusal.
    #[test]
    fn a_hand_edited_generated_file_is_regenerated_rather_than_refused() {
        let project = project();
        let out = scratch("rebuild");
        write(&project, &out).expect("writable");
        std::fs::write(out.join("package-lock.json"), "{}\n").expect("writable");
        std::fs::write(out.join("src/state.ts"), "// mine now\n").expect("writable");
        std::fs::write(out.join("README.md"), "# mine now\n").expect("writable");

        assert_eq!(
            write(&project, &out).expect("the second write is not refused"),
            Written {
                files: project.files().len(),
                carried: 0,
            }
        );
        assert_eq!(check(&project, &out).expect("readable"), []);
        assert!(
            out.join("package-lock.json").is_file(),
            "a file the emitter does not produce is untouched"
        );
        assert_eq!(
            not_ours(&project, &out).expect("readable"),
            Vec::<String>::new()
        );
    }

    /// Scaffold-once, in all three of its states: absent, written, and then
    /// left alone whatever the author put in it (PRD resolved q48).
    ///
    /// It also pins **where**: the stub lands in the project, beside `main.yml`,
    /// and not in `--out`. That is the only reading under which `validate` —
    /// which has no `--out` — can be required to refuse a binding whose file is
    /// missing, and it is what makes the path one string in the spec, on disk,
    /// and inside the artifact.
    #[test]
    fn an_absent_module_is_scaffolded_once_and_never_rewritten() {
        let (ir, scaffolds, root) = with_module();
        let out = scratch("scaffold");
        assert_eq!(scaffolds.len(), 1);

        let scaffolded = scaffold(&scaffolds, &root).expect("writable");
        assert_eq!(
            scaffolded,
            [("src/tools/sign.ts".to_string(), "tool.sign".to_string())],
            "the build reports what it wrote for the author"
        );
        let stub = std::fs::read_to_string(root.join("src/tools/sign.ts")).expect("readable");
        assert!(
            stub.contains("const toolSign: ToolSignModule = async (input) => {"),
            "{stub}"
        );
        assert!(
            !stub.contains(GENERATED_MARKER),
            "a scaffold is not generated: {stub}"
        );

        // The author fills it in. A rebuild neither rewrites it nor mentions it.
        std::fs::write(root.join("src/tools/sign.ts"), "export default 1;\n").expect("writable");
        assert_eq!(scaffold(&scaffolds, &root).expect("writable"), Vec::new());
        assert_eq!(
            std::fs::read_to_string(root.join("src/tools/sign.ts")).expect("readable"),
            "export default 1;\n"
        );

        // What the build then writes carries the author's bytes into the output
        // tree, at the path the composition names them by (PRD resolved q49) —
        // and `--check` compares that copy, because it is what the artifact
        // hash covers.
        let project = built(&ir, &root);
        let written = write(&project, &out).expect("writable");
        assert_eq!(
            written,
            Written {
                files: project.files().len(),
                carried: 1,
            }
        );
        assert_eq!(
            std::fs::read_to_string(out.join("src/tools/sign.ts")).expect("readable"),
            "export default 1;\n",
            "the artifact carries what the spec references"
        );
        assert_eq!(check(&project, &out).expect("readable"), []);

        // …and a copy that stopped matching what the author wrote is drift, the
        // same as a generated file somebody edited.
        std::fs::write(out.join("src/tools/sign.ts"), "export default 2;\n").expect("writable");
        assert_eq!(
            check(&project, &out).expect("readable"),
            [Drift {
                path: "src/tools/sign.ts".to_string(),
                state: State::Differs,
            }]
        );
    }

    /// An empty file counts as present: a scaffold is a write `build` makes
    /// where there is nothing at all, and "nothing at all" is a path holding no
    /// file. Anything else would mean reading what the author wrote to decide
    /// whether it was worth keeping.
    #[test]
    fn a_module_that_exists_is_never_scaffolded_over_however_empty() {
        let (_, scaffolds, root) = with_module();
        std::fs::create_dir_all(root.join("src/tools")).expect("writable");
        std::fs::write(root.join("src/tools/sign.ts"), "").expect("writable");

        assert_eq!(scaffold(&scaffolds, &root).expect("writable"), Vec::new());
        assert_eq!(
            std::fs::read_to_string(root.join("src/tools/sign.ts")).expect("readable"),
            ""
        );
    }

    /// "There" means the same thing to this command and to the verbs that refuse
    /// over it.
    ///
    /// A **directory** at a module path is the case that separates the two
    /// readings: `validate` and `build --check` ask whether a *file* is there
    /// and name `agent-compose build` as the repair, so a `build` that asked
    /// merely whether something exists would write nothing, say nothing, and
    /// then fail further down on a read — the named repair not repairing. It
    /// refuses instead, naming the path and the tool.
    #[test]
    fn a_directory_at_a_module_path_is_refused_rather_than_scaffolded_around() {
        let (ir, scaffolds, root) = with_module();
        std::fs::create_dir_all(root.join("src/tools/sign.ts")).expect("writable");

        // The two verbs agree that nothing is there…
        let refusals = compose_core::check_modules(&ir, &root);
        assert_eq!(refusals.len(), 1, "{refusals:?}");
        assert!(
            refusals[0]
                .message
                .contains("`src/tools/sign.ts`, which does not exist"),
            "{refusals:?}"
        );
        // …and the repair they name says so too, rather than writing nothing.
        let refused =
            scaffold(&scaffolds, &root).expect_err("a directory is not an implementation");
        assert_eq!(refused.kind(), io::ErrorKind::AlreadyExists);
        assert!(
            refused
                .to_string()
                .contains("cannot scaffold `src/tools/sign.ts` for `tool.sign`"),
            "{refused}"
        );
    }

    /// `--out` may name the project itself, and then a carried file's
    /// destination *is* its source. Writing a file the bytes it already holds
    /// replaces nothing, so the "somebody else's file at a name I emit" refusal
    /// has nothing to say about it.
    #[test]
    fn a_carried_file_written_over_itself_is_not_somebody_elses() {
        let (ir, scaffolds, root) = with_module();
        scaffold(&scaffolds, &root).expect("writable");
        let project = built(&ir, &root);

        assert_eq!(
            not_ours(&project, &root).expect("readable"),
            Vec::<String>::new(),
            "the project root holds the author's own bytes at that path"
        );
        write(&project, &root).expect("the write is not refused");
        assert_eq!(check(&project, &root).expect("readable"), []);

        // The other direction: a directory this compiler never built into, that
        // holds a *different* file at a carried name, is one it refuses.
        let out = scratch("carried-collision");
        std::fs::create_dir_all(out.join("src/tools")).expect("writable");
        std::fs::write(out.join("src/tools/sign.ts"), "// somebody else's\n").expect("writable");
        assert_eq!(
            not_ours(&project, &out).expect("readable"),
            ["src/tools/sign.ts"]
        );
    }

    #[test]
    fn an_empty_directory_is_all_drift() {
        let project = project();
        let out = scratch("empty");
        let drift = check(&project, &out).expect("readable");
        assert_eq!(drift.len(), project.files().len());
        assert!(drift.iter().all(|entry| entry.state == State::Missing));
    }

    #[test]
    fn the_default_output_directory_is_per_target() {
        assert_eq!(
            default_out(Path::new("services/api/main.yml"), "staging"),
            Path::new("services/api/build/staging")
        );
        assert_eq!(
            default_out(Path::new("main.yml"), "local"),
            Path::new("build/local")
        );
    }
}
