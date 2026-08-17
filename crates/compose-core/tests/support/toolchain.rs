//! The JavaScript toolchain the suites run emitted projects with, and where
//! they find it.
//!
//! Three integration binaries execute generated code — `generated_code_gates`,
//! `property_conformance`, and the acceptance suite's `harness` — and each one
//! used to carry its own copy of "is the runtime here, and install the pins".
//! Three copies of that is three chances for one suite to be checking a
//! different toolchain than the others, which is exactly the failure this file
//! exists to make impossible. Files under `tests/` subdirectories are not test
//! binaries, so this one is included with `#[path]` — by the two binaries beside
//! it, and by `crates/agent-compose/tests/compiled_graph_acceptance/harness.rs`,
//! which reaches across for it the same way it already reaches across for the
//! fixture directory itself.
//!
//! # Bun is the default runtime
//!
//! PRD §9.18: Bun is the default runtime and package manager everywhere an
//! emitted project is installed, launched, or gated, so [`installed`] installs
//! with `bun install --frozen-lockfile` and every runner is spawned with
//! [`bun`]. That is the same posture the emitted `README.md` documents, which is
//! what makes these gates evidence for it: the command a reader is given is the
//! command CI runs.
//!
//! Node ≥ 22.18 stays a **supported fallback**, and it is machine-checked rather
//! than asserted — see gate 13 of `tests/generated_code_gates.rs`, which installs
//! a golden with npm and type-checks, constructs and runs it under Node. That is
//! the one place `node` and `npm` are still required, and the reason
//! [`runs`] and [`required`] live here beside the Bun half.
//!
//! # Where `bun` is looked for
//!
//! In order, taking the first that answers `--version`:
//!
//! 1. `bun` on `PATH` — what `oven-sh/setup-bun` leaves in CI, and what a
//!    package manager leaves on a developer machine;
//! 2. `$BUN_INSTALL/bin/bun` — the variable Bun's own installer sets and exports;
//! 3. `$HOME/.bun/bin/bun` — where that installer puts it when `BUN_INSTALL` is
//!    unset, which is the common case in a shell that has not re-read its
//!    profile since the install.
//!
//! 2 and 3 are the difference between "Bun is installed" and "this shell knows
//! about it", and a suite that skipped over that difference would report a
//! missing toolchain to someone who has one.
//!
//! # When a runtime is missing
//!
//! **In CI it is a failure; on a developer machine it is a skip.** CI is where
//! "the suite is green" has to mean "the generated code was installed, checked
//! and run" (CLAUDE.md), so a missing toolchain there is a broken job rather
//! than an absent one — the workflow installs both Bun and Node for exactly
//! this. That applies to **both** toolchains: a CI run without Bun would check
//! nothing, and a CI run without Node would leave the fallback promise of PRD
//! §9.18 unchecked while the README kept making it. Locally, a contributor
//! working on the parser should not be blocked by either install, and the skip
//! says so on stderr rather than passing silently. `CI` (any non-empty value,
//! which every CI provider sets) is the switch.

#![allow(
    dead_code,
    reason = "each binary uses the part of the toolchain surface it needs"
)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

/// The committed toolchain fixture: the pinned manifest, its lockfiles, and the
/// runners.
///
/// It lives in `compose-core` and is reached from the crate that includes this
/// file, which is always `crates/<name>` — so the repository root is the
/// manifest directory's grandparent either way.
pub fn root() -> PathBuf {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("a crate under `crates/` has a grandparent")
        .join("crates/compose-core/tests/toolchain");
    assert!(
        root.is_dir(),
        "the committed toolchain fixture is missing: {}",
        root.display()
    );
    root
}

/// Whether a missing toolchain is a failure rather than a skip.
pub fn required() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}

/// Whether a program on `PATH` answers `--version`.
pub fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Where `bun` is, by the search order in the module docs, or `None`.
pub fn bun_executable() -> Option<&'static Path> {
    static FOUND: OnceLock<Option<PathBuf>> = OnceLock::new();
    FOUND
        .get_or_init(|| {
            if runs("bun") {
                return Some(PathBuf::from("bun"));
            }
            let installed = std::env::var_os("BUN_INSTALL")
                .map(PathBuf::from)
                .into_iter()
                .chain(std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".bun")));
            for prefix in installed {
                let candidate = prefix.join("bin/bun");
                if Command::new(&candidate)
                    .arg("--version")
                    .output()
                    .is_ok_and(|output| output.status.success())
                {
                    return Some(candidate);
                }
            }
            None
        })
        .as_deref()
}

/// `bun`, as a command, or `None` when it is absent and this is not CI.
///
/// # Panics
///
/// Panics in CI when Bun is absent, which is the rule the module docs state.
pub fn bun_command() -> Option<Command> {
    let Some(bun) = bun_executable() else {
        assert!(
            !required(),
            "`bun` is required: it is the default runtime a generated project is installed, \
             launched and gated with (PRD §9.18), and these suites are what make `cargo test` \
             mean the emitted TypeScript installs, compiles and runs (CLAUDE.md). \
             CI installs it; see .github/workflows/ci.yml."
        );
        eprintln!(
            "warning: skipping the generated-code runs — `bun` is not on PATH, in `$BUN_INSTALL` \
             or in `~/.bun`. It is required in CI (`CI` is set there) and this run is not CI."
        );
        return None;
    };
    Some(Command::new(bun))
}

/// The installed toolchain, or `None` when Bun is absent and this is not CI.
///
/// The install runs once per test binary. `--frozen-lockfile` rather than a bare
/// `bun install`: it installs exactly the committed `bun.lock` and fails instead
/// of rewriting it, so a run of the suite cannot quietly change what the next one
/// checks against — the same contract `npm ci` gave the fixture before Bun became
/// the default.
pub fn installed() -> Option<&'static Path> {
    static TOOLCHAIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    TOOLCHAIN
        .get_or_init(|| {
            let mut install = bun_command()?;
            let root = root();
            let output = install
                .args(["install", "--frozen-lockfile"])
                .current_dir(&root)
                .output()
                .expect("bun runs");
            assert!(
                output.status.success(),
                "the pinned toolchain did not install with `bun install --frozen-lockfile`; \
                 when a pin moved, regenerate the lockfile with `bun install` in \
                 `crates/compose-core/tests/toolchain` and commit it:\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
            );
            Some(root)
        })
        .as_deref()
}

/// `bun`, for a caller that has already confirmed the toolchain is there.
///
/// # Panics
///
/// Panics when Bun is absent, so call it only after [`installed`] has answered
/// `Some` — every caller already has to, since running an emitted project needs
/// the install.
pub fn bun() -> Command {
    bun_command().expect("the toolchain installed, so bun was found")
}

/// A `bun` command that runs one of the fixture's runners.
pub fn runner(script: &str) -> Command {
    let mut command = bun();
    command.arg(root().join(script));
    command
}
