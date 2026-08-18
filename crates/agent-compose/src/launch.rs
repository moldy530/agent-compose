//! `agent-compose run` and `agent-compose serve`: building a project and then
//! **starting** it (PRD 5.11, §7 M1).
//!
//! Both verbs are the same four steps, and only the last differs:
//!
//! 1. **validate**, and refuse on anything reported — a run of a composition the
//!    compiler had something to say about would report that thing again, later,
//!    with no span to point at (the same rule `build` follows);
//! 2. **build**, through `build::write` and `build::default_out`, so a run leaves
//!    behind exactly the directory `agent-compose build` would have written and a
//!    reader can inspect, re-run or eject it;
//! 3. **check the environment**, which is PRD §9.15's second half — "`run`/`serve`
//!    fail fast before invoking the graph, naming the missing variable". The
//!    emitted project checks itself at process start too (`src/index.ts` calls
//!    `readEnvironment()`), and both are wanted: the compiler knows every
//!    reference statically, so it can name all of them and the site each is
//!    written at *before* spawning anything;
//! 4. **launch** the emitted project's own command line — `bun src/index.ts run …`
//!    or `… serve …`, falling back to `node` where Bun is absent (PRD §9.18).
//!
//! # Why the verbs launch rather than link
//!
//! Nothing about invoking a compiled graph lives in this binary. The emitted
//! project is a plain TypeScript project with its own entry point, which is what
//! makes PRD 5.12's eject path real: `agent-compose run` runs the command a
//! reader could have typed, and the emitted `README.md` documents that command
//! first. A compiler that had reimplemented invocation would be a second
//! implementation of the runtime, drifting from the one an ejected project keeps.
//!
//! # Exit codes
//!
//! The table `main` documents, unchanged: `0` clean, `1` "the answer is no" — a
//! composition that reported diagnostics, or a run that produced no answer — and
//! `2` for a command that could not run at all, which is what a missing
//! entrypoint, an unresolvable JavaScript runtime, an unresolved dependency set
//! and a missing environment variable all are. The child's own code is passed
//! through unchanged, and `src/cli.ts` gives the three the same meanings.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use compose_core::Ir;

/// Where a launch stopped, when it stopped before the child ran.
pub(crate) enum Refusal {
    /// The command's own precondition failed: exit `2` with this message.
    Unusable(String),
}

/// The JavaScript runtime an emitted project is launched with (PRD §9.18).
///
/// Bun is the default — it is what the emitted `README.md` documents first and
/// what the generated-code gates run — and Node ≥ 22.18 is the supported
/// fallback. The search is the same one the test toolchain performs, and for the
/// same reason: `$BUN_INSTALL/bin/bun` and `~/.bun/bin/bun` are the difference
/// between "Bun is installed" and "this shell knows about it".
pub(crate) fn runtime() -> Result<PathBuf, Refusal> {
    if answers("bun") {
        return Ok(PathBuf::from("bun"));
    }
    let installed = std::env::var_os("BUN_INSTALL")
        .map(PathBuf::from)
        .into_iter()
        .chain(std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".bun")));
    for prefix in installed {
        let candidate = prefix.join("bin/bun");
        if answers(&candidate.to_string_lossy()) {
            return Ok(candidate);
        }
    }
    if answers("node") {
        return Ok(PathBuf::from("node"));
    }
    Err(Refusal::Unusable(
        "no JavaScript runtime was found: an emitted project runs on Bun by default and on \
         Node >= 22.18 as a fallback, and neither `bun` nor `node` answered `--version`. \
         Install one, or run the built project yourself — it is a plain TypeScript project \
         and its README says how"
            .to_string(),
    ))
}

/// Whether a program answers `--version`.
fn answers(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

/// Refuse before spawning when a `${ENV}` reference the composition makes is not
/// set (PRD 5.9, §9.15).
///
/// Every reference at once, each with the site it is written at: a run that
/// stopped at the first would take as many attempts to configure as the project
/// has secrets. The emitted project performs the same check at process start,
/// which is what covers a project run by hand; this one covers the case the
/// milestone names, and covers it before a graph is built.
pub(crate) fn environment(ir: &Ir) -> Result<(), Refusal> {
    let references = compose_core::codegen::env::References::of(ir);
    let mut missing: Vec<String> = Vec::new();
    for name in references.names() {
        if std::env::var_os(name).is_none() {
            missing.push(format!(
                "`{name}` (referenced by {})",
                references
                    .sites(name)
                    .iter()
                    .map(|site| format!("`{site}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
    }
    if missing.is_empty() {
        return Ok(());
    }
    Err(Refusal::Unusable(format!(
        "{} this composition references {} not set: {}. Environment references are resolved \
         when the graph runs, never at build (PRD 5.9), so this is the first point the values \
         are needed",
        if missing.len() == 1 {
            "an environment variable"
        } else {
            "environment variables"
        },
        if missing.len() == 1 { "is" } else { "are" },
        missing.join(", ")
    )))
}

/// Refuse before spawning when the pinned dependency set is not installed.
///
/// An emitted project is source, not a bundle: it imports LangGraph, Zod and the
/// rest of `codegen::project::PINS` by name, and a launch into a directory with
/// no `node_modules` above it fails inside the runtime with a resolution error
/// naming a package rather than a step. Node resolution walks up, so this walks
/// up too — a project built under a directory that already has the install is
/// exactly the arrangement the acceptance suite and a monorepo both use.
pub(crate) fn dependencies(out: &Path) -> Result<(), Refusal> {
    for directory in out.ancestors() {
        if directory.join("node_modules/@langchain/langgraph").is_dir() {
            return Ok(());
        }
    }
    Err(Refusal::Unusable(format!(
        "the pinned dependency set is not installed for `{}`: run `bun install` there (or \
         `npm install`) before running the graph. `agent-compose build` writes the project \
         and its `package.json`; installing is yours, because the lockfile is yours",
        out.display()
    )))
}

/// Spawn the emitted project's own command line and answer with its exit code.
///
/// stdin, stdout and stderr are **inherited**, which is what makes the child's
/// answer the command's answer: `run` prints the flow's outputs on stdout and
/// its report on stderr, and `serve` announces its address on stdout as one JSON
/// line — and a caller reading either is reading the child directly rather than
/// something this process copied.
pub(crate) fn spawn(runtime: &Path, out: &Path, arguments: &[String]) -> Result<u8, Refusal> {
    let mut command = Command::new(runtime);
    command
        .arg("src/index.ts")
        .args(arguments)
        .current_dir(out)
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    let status = command.status().map_err(|error| {
        Refusal::Unusable(format!(
            "cannot start `{} src/index.ts` in `{}`: {error}",
            runtime.display(),
            out.display()
        ))
    })?;
    // A child killed by a signal reports no code. It did not answer, which is
    // what `1` means here.
    Ok(u8::try_from(status.code().unwrap_or(1)).unwrap_or(1))
}
