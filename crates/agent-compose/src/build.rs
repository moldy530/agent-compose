//! `agent-compose build`: the emission set on disk, and the check that it still
//! matches.
//!
//! `compose_core::emit` answers a set of `(path, contents)` pairs and touches
//! nothing (see `compose_core::codegen`). This module is the half that does:
//! writing that set into a directory, and — under `--check` — comparing it
//! against what is already there without writing anything.
//!
//! # What the compiler owns
//!
//! Ownership is claimed by the **generated-file header**, not by a path.
//! `<out>/src/` is the compiler's *directory* — it is the one place a build
//! removes files from — but the four files it writes at the root
//! (`package.json`, `tsconfig.json`, `README.md`, `.gitignore`) sit in a
//! directory that may be someone's project, so being emitted is not on its own a
//! licence to replace what is already there. So:
//!
//! * **writing** replaces every emitted file and **removes** any other file under
//!   `src/` *that this compiler wrote*, then prunes the directories **that
//!   removal** left empty. A module that was renamed between two compiler
//!   releases would otherwise linger, still importable, and `--check` would
//!   report it forever on a project that had just been rebuilt. A file under
//!   `src/` with no generated-file header is not the compiler's to delete, and
//!   an emitted path already occupied in a directory the compiler has never
//!   built into is not its to replace; either one stops the write instead — see
//!   [`write`] — because `--out` can name a directory the compiler never made.
//!   An empty directory under `src/` is neither: it holds nothing to lose and
//!   nothing to refuse over, so it is simply left where it is
//!   ([`prune_emptied_directories`]).
//! * **checking** reports a file that is missing, one whose bytes differ, and one
//!   under `src/` that the emitter did not produce — the three ways a committed
//!   project can stop matching its spec (PRD §8: "hand-edited generated code
//!   forks the source of truth", mitigated by "`build --check` in CI"). It also
//!   answers, through [`not_ours`], whether a rebuild would *fix* what it found,
//!   because a drift report ends in a remedy and the two ways a directory can
//!   drift are not the two the write treats alike: a stale module the compiler
//!   wrote and somebody's own file are both "under `src/` and not emitted", and
//!   `build` removes the first and refuses over the second.
//! * neither **removes** anything outside `src/`. `node_modules/`, a lockfile, and
//!   a `.env` live in `<out>` beside the generated files, and a build that deleted
//!   them would make the output directory unusable as a project directory.

use std::collections::BTreeSet;
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

use compose_core::GeneratedProject;

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
    /// The directory has it under `src/` and the compiler does not emit it.
    Unexpected,
}

impl State {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Differs => "differs",
            Self::Unexpected => "unexpected",
        }
    }

    /// The sentence a human report writes for it.
    const fn explanation(self) -> &'static str {
        match self {
            Self::Missing => "is not in the output directory",
            Self::Differs => "differs from what the spec produces",
            Self::Unexpected => "is under `src/` and is not generated",
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
    /// Files removed from under `src/` because the emitter no longer produces
    /// them, sorted.
    pub(crate) removed: Vec<String>,
}

/// Why a write did not happen.
#[derive(Debug)]
pub(crate) enum Refusal {
    /// The filesystem said no.
    Io(io::Error),
    /// The write would have replaced or removed files this compiler did not
    /// write. The paths, sorted.
    NotOurs(Vec<String>),
}

impl From<io::Error> for Refusal {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

/// The marker every generated file opens with (`compose_core::codegen::header`).
///
/// Ownership of `src/` is claimed by *this* string rather than by the directory
/// existing: it is the only evidence on disk that a file came from a spec.
const GENERATED_MARKER: &str = "generated by agent-compose";

/// Write the whole emission set, and remove whatever else is under `src/`.
///
/// # The check before the write
///
/// `src/` is the compiler's directory (see the module header), and a build that
/// removes a stale module from it is doing its job. But "the compiler's
/// directory" is a claim about a directory the compiler *made*, and `--out` can
/// name any path — an existing Node project, a checkout of an ejected copy, `.`.
/// Deleting a `src/` full of someone's own TypeScript, silently, on the first
/// build into it, is not a rule about generated code; it is data loss.
///
/// The same sentence, word for word, covers the four files the emitter writes at
/// the root. A `package.json` naming somebody's application, a hand-written
/// `README.md`, a `.gitignore` with their own entries in it: all four are paths
/// this compiler emits, and a first build that replaced them because they
/// happened to share a name would destroy exactly as much as an emptied `src/`,
/// while reporting *less* — an overwrite leaves no [`Written::removed`] entry to
/// notice.
///
/// So there are two questions, and they are asked of different things:
///
/// * **Is this directory one this compiler has built into?** [`claimed`] answers
///   it once, for the whole write, by looking for [`GENERATED_MARKER`] at any
///   path the emitter produces. If nothing there is this compiler's, then an
///   emitted path that is already occupied belongs to somebody else and the
///   write stops.
/// * **Is this individual file one this compiler wrote?** Asked only of files
///   under `src/` that the emitter does *not* produce, because the answer decides
///   whether to **delete** them.
///
/// The first question is about the directory rather than about each file on
/// purpose. Inside a directory this compiler built, a generated file that has
/// been hand-edited — header and all — is exactly what PRD §8 says to
/// regenerate, and what the drift report tells the reader to run `build` for; a
/// rule that refused it would send them to a command that then refuses too.
/// Outside one, no file at an emitted path has ever been the compiler's.
///
/// Either refusal is [`Refusal::NotOurs`] naming the paths, and nothing has been
/// written or deleted when it happens, because both scans run first. The
/// removals that do happen come back in [`Written::removed`] and are reported, so
/// "the build also deleted three files" is something the run says rather than
/// something a later `git status` discovers.
pub(crate) fn write(project: &GeneratedProject, out: &Path) -> Result<Written, Refusal> {
    let Existing { stale, foreign } = existing(project, out)?;
    if !foreign.is_empty() {
        return Err(Refusal::NotOurs(foreign));
    }

    for file in project.files() {
        let path = at(out, &file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.contents)?;
    }
    for path in &stale {
        std::fs::remove_file(at(out, path))?;
    }
    prune_emptied_directories(out, &stale)?;

    Ok(Written {
        files: project.files().len(),
        removed: stale,
    })
}

/// What is already in the output directory, sorted into the two answers
/// [`write`]'s two questions reach.
struct Existing {
    /// Under `src/`, not emitted, and carrying the generated-file header: the
    /// write's to remove.
    stale: Vec<String>,
    /// Paths this compiler did not write that the write would have replaced or
    /// removed. Any one of them stops it ([`Refusal::NotOurs`]).
    foreign: Vec<String>,
}

/// Ask the directory both of [`write`]'s questions, touching nothing.
fn existing(project: &GeneratedProject, out: &Path) -> io::Result<Existing> {
    let emitted: BTreeSet<&str> = project.paths().collect();
    let mut stale = Vec::new();
    let mut foreign = Vec::new();
    // What is under `src/` and is not emitted: this compiler's to remove when it
    // wrote it, and not its to remove otherwise.
    for path in owned_files(out)? {
        if emitted.contains(path.as_str()) {
            continue;
        }
        if generated(&at(out, &path))? {
            stale.push(path);
        } else {
            foreign.push(path);
        }
    }
    // …and, in a directory this compiler has never built into, what *is* emitted
    // and already there — which the write would replace.
    if !claimed(project, out)? {
        for path in project.paths() {
            if occupied(&at(out, path))? {
                foreign.push(path.to_string());
            }
        }
    }
    foreign.sort();
    Ok(Existing { stale, foreign })
}

/// What a [`write`] would refuse over, answered without writing: the payload
/// [`Refusal::NotOurs`] would carry, sorted, and empty when the write would go
/// through.
///
/// `--check` needs this for a reason `write` does not have: its report ends in a
/// remedy, and "run `agent-compose build`" is the wrong one for a directory
/// `build` declines to touch. The two ways a file can be "under `src/` and not
/// emitted" are indistinguishable in a [`Drift`] — a module an older compiler
/// release wrote and somebody's own TypeScript are both [`State::Unexpected`] —
/// and they are the two the write treats *differently*, so the report cannot
/// tell them apart on its own. Reading the answer off the same scan the write
/// makes is what keeps the help from promising something the next command
/// refuses.
pub(crate) fn not_ours(project: &GeneratedProject, out: &Path) -> io::Result<Vec<String>> {
    Ok(existing(project, out)?.foreign)
}

/// Whether the output directory already holds something this compiler wrote.
///
/// The evidence is [`GENERATED_MARKER`] at any path the emitter produces: a
/// directory holding even one of this compiler's files is one it has built into,
/// and every emitted path in it is a build artifact rather than somebody's file
/// that happens to share a name. A directory holding none of them may be
/// anything at all, and is treated as if it were the user's.
fn claimed(project: &GeneratedProject, out: &Path) -> io::Result<bool> {
    for path in project.paths() {
        match generated(&at(out, path)) {
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
/// emitted file, and a `src/` holding something large that is not ours should not
/// be loaded into memory to find that out. A file that is not UTF-8 is not one
/// this compiler wrote.
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

/// Compare the emission set against the directory, writing nothing.
///
/// Answers the drift, sorted by path.
pub(crate) fn check(project: &GeneratedProject, out: &Path) -> io::Result<Vec<Drift>> {
    let mut drift = Vec::new();
    for file in project.files() {
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

    let emitted: BTreeSet<&str> = project.paths().collect();
    for unexpected in owned_files(out)?
        .into_iter()
        .filter(|path| !emitted.contains(path.as_str()))
    {
        drift.push(Drift {
            path: unexpected,
            state: State::Unexpected,
        });
    }

    drift.sort();
    Ok(drift)
}

/// Where a `/`-separated generated path lands on this host.
fn at(out: &Path, path: &str) -> PathBuf {
    let mut full = out.to_path_buf();
    full.extend(path.split('/'));
    full
}

/// Every file under `<out>/src`, as `/`-separated paths relative to `<out>`,
/// sorted.
///
/// A missing `src/` is not an error: it is what a first build finds.
fn owned_files(out: &Path) -> io::Result<Vec<String>> {
    let root = out.join(compose_core::codegen::OWNED_DIRECTORY);
    let mut found = Vec::new();
    let mut queue = vec![root.clone()];
    while let Some(directory) = queue.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            let path = entry.path();
            if entry.file_type()?.is_dir() {
                queue.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(out)
                .expect("the walk started inside the output directory");
            found.push(
                relative
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    found.sort();
    Ok(found)
}

/// Remove the directories **this write** emptied: the ancestors, under `src/`,
/// of the files it removed, deepest first.
///
/// A module that moved between two compiler releases leaves its directory
/// behind, and a rebuild that left it there would report drift forever on a
/// project it had just rebuilt — so the removal has to take the directory with
/// it. What it must not take is a directory the compiler never created. An empty
/// `src/vendor/` is not a file, so [`owned_files`] does not see it, [`claimed`]
/// has nothing to refuse over it, and a sweep of every empty directory under
/// `src/` deleted it on the *first* build into the directory — the one case the
/// module header says is not the compiler's to touch.
///
/// Scoping it to the ancestors of what was removed is the whole of the fix: a
/// directory this write did not empty is a directory it has no claim on,
/// whatever else is or is not in it.
fn prune_emptied_directories(out: &Path, removed: &[String]) -> io::Result<()> {
    let root = out.join(compose_core::codegen::OWNED_DIRECTORY);
    let mut directories: BTreeSet<PathBuf> = BTreeSet::new();
    for path in removed {
        let mut directory = at(out, path);
        while directory.pop() && directory.starts_with(&root) {
            directories.insert(directory.clone());
        }
    }
    // Deepest first, so a directory holding only directories this write emptied
    // is empty by the time it is reached.
    let mut directories: Vec<PathBuf> = directories.into_iter().collect();
    directories.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    for directory in directories {
        if std::fs::read_dir(&directory).is_ok_and(|mut entries| entries.next().is_none()) {
            std::fs::remove_dir(&directory)?;
        }
    }
    Ok(())
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
        compose_core::emit(&resolution.ir.expect("an artifact"))
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
                removed: Vec::new(),
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

    /// `src/` is the compiler's directory: a module *it* wrote that the emitter
    /// no longer produces is drift, and a rebuild removes it and says so.
    /// Everything outside is the user's and is left alone.
    #[test]
    fn a_stale_generated_module_is_drift_and_a_rebuild_removes_it() {
        let project = project();
        let out = scratch("stale");
        write(&project, &out).expect("writable");
        std::fs::create_dir_all(out.join("src/nodes")).expect("writable");
        std::fs::write(
            out.join("src/nodes/old.ts"),
            "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
        )
        .expect("writable");
        std::fs::create_dir_all(out.join("node_modules/left")).expect("writable");
        std::fs::write(out.join("node_modules/left/index.js"), "//\n").expect("writable");
        std::fs::write(out.join("package-lock.json"), "{}\n").expect("writable");

        assert_eq!(
            check(&project, &out).expect("readable"),
            [Drift {
                path: "src/nodes/old.ts".to_string(),
                state: State::Unexpected,
            }]
        );

        assert_eq!(
            write(&project, &out).expect("writable").removed,
            ["src/nodes/old.ts".to_string()],
            "a removal is part of what the write reports"
        );
        assert_eq!(check(&project, &out).expect("readable"), []);
        assert!(
            !out.join("src/nodes").exists(),
            "the directory it left behind is pruned too"
        );
        assert!(
            out.join("node_modules/left/index.js").is_file(),
            "nothing outside `src/` is removed"
        );
        assert!(out.join("package-lock.json").is_file());
    }

    /// The prune takes the directories *this write* emptied and no others.
    ///
    /// An empty directory under `src/` is invisible to every refusal the write
    /// makes: `owned_files` collects files, so there is nothing to call foreign,
    /// and `claimed` has no header to read. A sweep of every empty directory
    /// under `src/` therefore deleted one on the **first** build into a
    /// directory — which is exactly the case the module header says is not the
    /// compiler's to touch.
    #[test]
    fn a_directory_the_write_did_not_empty_is_left_where_it_is() {
        let project = project();
        let out = scratch("keep-directory");
        std::fs::create_dir_all(out.join("src/keepme")).expect("writable");
        std::fs::create_dir_all(out.join("src/nested/deeper")).expect("writable");

        assert_eq!(
            write(&project, &out).expect("writable"),
            Written {
                files: project.files().len(),
                removed: Vec::new(),
            },
            "an empty directory is not a file the write has anything to say about"
        );
        assert!(
            out.join("src/keepme").is_dir() && out.join("src/nested/deeper").is_dir(),
            "a first build emptied a directory it never created"
        );

        // …and the directory a removal *does* empty still goes, at any depth.
        std::fs::write(
            out.join("src/nested/deeper/old.ts"),
            "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
        )
        .expect("writable");
        assert_eq!(
            write(&project, &out).expect("writable").removed,
            ["src/nested/deeper/old.ts".to_string()]
        );
        assert!(
            !out.join("src/nested").exists(),
            "the empty chain the removal left behind is pruned to `src/`"
        );
        assert!(out.join("src/keepme").is_dir(), "and nothing beside it is");
    }

    /// A `src/` the compiler did not write is not a `src/` it may empty.
    ///
    /// The scenario is a plausible `--out`: an existing Node project, or an
    /// ejected copy being re-synced. The first build into one used to delete
    /// every file under `src/` it did not emit, report nothing, and exit `0`.
    #[test]
    fn a_src_directory_the_compiler_did_not_write_stops_the_build() {
        let project = project();
        let out = scratch("not-ours");
        std::fs::create_dir_all(out.join("src/deep")).expect("writable");
        std::fs::write(out.join("src/deep/mine.ts"), "export const mine = 1;\n").expect("writable");
        std::fs::write(out.join("src/also-mine.ts"), "export const x = 2;\n").expect("writable");

        let refusal = write(&project, &out).expect_err("the write is refused");
        let Refusal::NotOurs(paths) = refusal else {
            panic!("the refusal names the files, not an io error: {refusal:?}");
        };
        assert_eq!(paths, ["src/also-mine.ts", "src/deep/mine.ts"]);
        assert_eq!(
            std::fs::read_to_string(out.join("src/deep/mine.ts")).expect("readable"),
            "export const mine = 1;\n",
            "nothing is deleted before the scan decides"
        );
        assert!(
            !out.join("src/schemas.ts").exists(),
            "and nothing is written either: the refusal is total"
        );
    }

    /// The four files the emitter writes at the root are not its to overwrite
    /// either.
    ///
    /// The `src/` scan above cannot see this one: the scenario is a real Node
    /// project whose sources are not under `src/`, so there is nothing under
    /// `src/` to find and the write used to replace `package.json`, `README.md`,
    /// `tsconfig.json` and `.gitignore` in place — no refusal, no
    /// [`Written::removed`] entry, exit `0`, originals gone.
    #[test]
    fn root_files_the_compiler_did_not_write_stop_the_build() {
        let project = project();
        let out = scratch("root-not-ours");
        std::fs::write(
            out.join("package.json"),
            "{\"name\":\"my-real-app\",\"version\":\"3.2.1\"}\n",
        )
        .expect("writable");
        std::fs::write(out.join("README.md"), "# My real project\n").expect("writable");
        std::fs::write(out.join(".gitignore"), "dist/\n").expect("writable");
        std::fs::create_dir_all(out.join("lib")).expect("writable");
        std::fs::write(out.join("lib/index.ts"), "export const mine = 1;\n").expect("writable");

        let refusal = write(&project, &out).expect_err("the write is refused");
        let Refusal::NotOurs(paths) = refusal else {
            panic!("the refusal names the files, not an io error: {refusal:?}");
        };
        assert_eq!(
            paths,
            [".gitignore", "README.md", "package.json"],
            "every emitted path already held by someone else's file, and only those: \
             `tsconfig.json` is not there, so it is not in the refusal"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("package.json")).expect("readable"),
            "{\"name\":\"my-real-app\",\"version\":\"3.2.1\"}\n",
            "nothing is overwritten before the scan decides"
        );
        assert_eq!(
            std::fs::read_to_string(out.join("README.md")).expect("readable"),
            "# My real project\n"
        );
        assert!(
            !out.join("src").exists(),
            "and nothing is written either: the refusal is total"
        );

        // `--check` was always right about these: it reports them as drift
        // rather than replacing them.
        let drift = check(&project, &out).expect("readable");
        assert!(
            drift.contains(&Drift {
                path: "package.json".to_string(),
                state: State::Differs,
            }),
            "{drift:?}"
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
                removed: Vec::new(),
            }
        );
        assert_eq!(check(&project, &out).expect("readable"), []);
        assert!(
            out.join("package-lock.json").is_file(),
            "a file the emitter does not produce and does not own is untouched"
        );
    }

    /// `--check` can tell the two halves of [`State::Unexpected`] apart, which
    /// is what its remedy depends on.
    ///
    /// Both files below are "under `src/` and not emitted" and the drift report
    /// gives them the same state, but a rebuild **removes** the one carrying the
    /// header and **refuses** over the other. A report that could not tell them
    /// apart would send a reader from a drift it can act on to an exit `2`.
    #[test]
    fn a_check_learns_which_drift_a_rebuild_would_refuse_over() {
        let project = project();
        let out = scratch("what-a-rebuild-would-do");
        write(&project, &out).expect("writable");

        // A module an older release wrote: drift, and a rebuild's to remove.
        std::fs::write(
            out.join("src/old.ts"),
            "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
        )
        .expect("writable");
        assert_eq!(
            check(&project, &out).expect("readable"),
            [Drift {
                path: "src/old.ts".to_string(),
                state: State::Unexpected,
            }]
        );
        assert_eq!(
            not_ours(&project, &out).expect("readable"),
            Vec::<String>::new(),
            "a stale generated module is drift `build` settles"
        );

        // Somebody's own file: the same state, and a rebuild refuses over it.
        std::fs::write(out.join("src/mine.ts"), "export const mine = 1;\n").expect("writable");
        assert_eq!(
            check(&project, &out).expect("readable"),
            [
                Drift {
                    path: "src/mine.ts".to_string(),
                    state: State::Unexpected,
                },
                Drift {
                    path: "src/old.ts".to_string(),
                    state: State::Unexpected,
                },
            ],
            "the report cannot tell them apart"
        );
        assert_eq!(
            not_ours(&project, &out).expect("readable"),
            ["src/mine.ts".to_string()],
            "and this is the answer that can"
        );

        // …and it is the write's own answer: the same paths, from the same scan.
        let refusal = write(&project, &out).expect_err("the write is refused");
        let Refusal::NotOurs(paths) = refusal else {
            panic!("the refusal names the files, not an io error: {refusal:?}");
        };
        assert_eq!(paths, ["src/mine.ts".to_string()]);
        assert!(
            out.join("src/old.ts").is_file(),
            "the refusal is total, so the stale module it would have removed is still there"
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
