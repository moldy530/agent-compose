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
//! `<out>/src/` is the compiler's directory and the rest of `<out>` is the
//! user's. So:
//!
//! * **writing** replaces every emitted file and **removes** any other file under
//!   `src/`, then prunes the directories left empty. A module that was renamed
//!   between two compiler releases would otherwise linger, still importable, and
//!   `--check` would report it forever on a project that had just been rebuilt.
//! * **checking** reports a file that is missing, one whose bytes differ, and one
//!   under `src/` that the emitter did not produce — the three ways a committed
//!   project can stop matching its spec (PRD §8: "hand-edited generated code
//!   forks the source of truth", mitigated by "`build --check` in CI").
//! * neither touches anything outside `src/`. `node_modules/`, a lockfile, and a
//!   `.env` live in `<out>` beside the generated files, and a build that deleted
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

/// Write the whole emission set, and remove whatever else is under `src/`.
///
/// Answers how many files were written.
pub(crate) fn write(project: &GeneratedProject, out: &Path) -> io::Result<usize> {
    for file in project.files() {
        let path = at(out, &file.path);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(&path, &file.contents)?;
    }

    let emitted: BTreeSet<&str> = project.paths().collect();
    for stale in owned_files(out)?
        .into_iter()
        .filter(|path| !emitted.contains(path.as_str()))
    {
        std::fs::remove_file(at(out, &stale))?;
    }
    prune_empty_directories(&out.join(compose_core::codegen::OWNED_DIRECTORY))?;

    Ok(project.files().len())
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

/// Remove the directories under `src/` that hold nothing, deepest first, and
/// `src/` itself if it is empty.
fn prune_empty_directories(root: &Path) -> io::Result<()> {
    let mut directories = Vec::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(directory) = queue.pop() {
        let entries = match std::fs::read_dir(&directory) {
            Ok(entries) => entries,
            Err(error) if error.kind() == io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error),
        };
        for entry in entries {
            let entry = entry?;
            if entry.file_type()?.is_dir() {
                queue.push(entry.path());
            }
        }
        directories.push(directory);
    }
    // Deepest first, so a directory holding only empty directories is empty by
    // the time it is reached.
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
            project.files().len()
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

    /// `src/` is the compiler's directory: a file in it that the emitter did not
    /// write is drift, and a rebuild removes it. Everything outside is the
    /// user's and is left alone.
    #[test]
    fn a_stray_file_under_src_is_drift_and_a_rebuild_removes_it() {
        let project = project();
        let out = scratch("stray");
        write(&project, &out).expect("writable");
        std::fs::create_dir_all(out.join("src/nodes")).expect("writable");
        std::fs::write(out.join("src/nodes/old.ts"), "// stale\n").expect("writable");
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

        write(&project, &out).expect("writable");
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
