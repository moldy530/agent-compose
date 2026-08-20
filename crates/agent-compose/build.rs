//! What `agent-compose --version` prints, decided at compile time.
//!
//! A released binary is downloaded rather than built (PRD §7 M2), so the
//! question "which build am I holding?" has to be answerable from the binary
//! alone — a diagnostic's `unsupported-version` explanation already sends a
//! reader to `--version` for exactly that. Three facts answer it: the crate
//! version, the commit it was built from, and the target it was built for.
//! Cargo knows the first and the third; only the second needs asking.
//!
//! **`GIT_SHA` wins over the repository.** A release job hands the sha it
//! checked out, which is the sha a reader can go back to, and a build that
//! reads the environment instead of shelling out works the same in a container
//! with no `git` on it. When the variable says nothing usable the script asks
//! the repository, and when there is no repository — a source tarball, a vendored
//! build — the sha is `unknown` and the build still succeeds. That last case is
//! the one this script must never break: a crate that cannot build without a
//! `.git` beside it is not publishable.
//!
//! Nothing here reads a spec, so nothing here can change what the compiler
//! *does*. The one output is a string.

use std::path::{Path, PathBuf};
use std::process::Command;

/// What stands in for a commit nobody could name.
const UNKNOWN: &str = "unknown";

/// How many characters of a sha the version line carries — `git`'s own short
/// form, which is what a reader pastes back into `git show`.
const SHORT: usize = 7;

fn main() {
    // Emitting any `rerun-if` directive replaces cargo's default rule ("rerun
    // when a file in the package changed"), which is the point: this script's
    // output depends on the environment and on `HEAD`, not on `src/`. Editing a
    // module must not re-run it, and committing must.
    println!("cargo::rerun-if-changed=build.rs");
    println!("cargo::rerun-if-env-changed=GIT_SHA");

    let manifest_dir = PathBuf::from(
        std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets `CARGO_MANIFEST_DIR`"),
    );
    track_head(&manifest_dir);

    let sha = sha_from_environment()
        .or_else(|| sha_from_repository(&manifest_dir))
        .unwrap_or_else(|| UNKNOWN.to_string());
    // Cargo sets `TARGET` for every build script; the fallback is for the
    // shape of the line rather than for a case that happens.
    let target = std::env::var("TARGET").unwrap_or_else(|_| UNKNOWN.to_string());
    let version = std::env::var("CARGO_PKG_VERSION").expect("cargo sets `CARGO_PKG_VERSION`");

    println!("cargo::rustc-env=COMPOSE_VERSION_LINE={version} ({sha}, {target})");
}

/// The sha a build environment named, when it named a usable one.
fn sha_from_environment() -> Option<String> {
    shorten(&std::env::var("GIT_SHA").ok()?)
}

/// The sha the repository this package sits in is checked out at.
fn sha_from_repository(manifest_dir: &Path) -> Option<String> {
    shorten(&git(manifest_dir, &["rev-parse", "HEAD"])?)
}

/// A commit name, normalized to the short form the version line carries.
///
/// Held to being **hex**, which is what rejects the things that are not a
/// commit at all: an empty variable, a branch name, a `-dirty` suffix, a
/// multi-line accident. A rejected value falls through to the next source
/// rather than landing in the version line, so `--version` either names a
/// commit or says `unknown` — never something in between that a reader would
/// try to look up.
fn shorten(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() || !trimmed.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    Some(trimmed.to_ascii_lowercase().chars().take(SHORT).collect())
}

/// Re-run this script when `HEAD` moves.
///
/// Both files matter and for different reasons: `HEAD` itself changes on a
/// checkout or when a detached build moves to another commit, and the ref it
/// names changes on a commit. A packed ref has no file of its own, so
/// `packed-refs` stands in for it. Every one of them is tracked only if it
/// exists — a path cargo cannot stat is a path cargo treats as changed, which
/// would re-run this script on every build.
fn track_head(manifest_dir: &Path) {
    let Some(git_dir) = git(manifest_dir, &["rev-parse", "--absolute-git-dir"]) else {
        return;
    };
    let git_dir = PathBuf::from(git_dir);
    track(&git_dir.join("HEAD"));

    let Some(reference) = git(manifest_dir, &["symbolic-ref", "--quiet", "HEAD"]) else {
        // A detached `HEAD` names a commit rather than a ref: the file above is
        // the whole of it.
        return;
    };
    let loose = git_dir.join(&reference);
    if loose.is_file() {
        track(&loose);
    } else {
        track(&git_dir.join("packed-refs"));
    }
}

/// Track one path, if it is there to track.
fn track(path: &Path) {
    if path.is_file() {
        println!("cargo::rerun-if-changed={}", path.display());
    }
}

/// One line of `git` output, or nothing at all.
///
/// "Nothing at all" covers every way this can fail — no `git` on `PATH`, no
/// repository, a repository with no commits — because the caller does the same
/// thing with all of them.
fn git(directory: &Path, arguments: &[&str]) -> Option<String> {
    let output = Command::new("git")
        .args(arguments)
        .current_dir(directory)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let line = String::from_utf8(output.stdout).ok()?.trim().to_string();
    (!line.is_empty()).then_some(line)
}
