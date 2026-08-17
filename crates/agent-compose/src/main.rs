//! `agent-compose` — the compiler's command line.
//!
//! Two commands exist. The first is the product's core loop (PRD §7 M0):
//!
//! ```text
//! agent-compose validate <path> [--target <name>] [--format human|json]
//! ```
//!
//! It parses the entrypoint, follows its `imports:`, resolves every name, and
//! runs every static check the grammar defines, then reports. The second is
//! codegen (PRD §7 M1):
//!
//! ```text
//! agent-compose build <path> [--target <name>] [--out <dir>] [--check]
//!                            [--format human|json]
//! ```
//!
//! It runs `validate` first and emits **only** on a clean report — generated
//! code is a build artifact of a valid composition, and emitting from a broken
//! one would produce a project whose failures are the spec's, reported by `tsc`
//! instead of by the compiler that has spans to point at. A clean report is then
//! asked a second question, which `validate` never asks: whether *this target*
//! can express the composition (`compose_core::target_diagnostics`). `pattern:`
//! is RE2 and RE2 is not a subset of ECMAScript, so a composition can be valid
//! and have no TypeScript project. The work is all `compose-core`'s; what lives
//! here is the surface — argument parsing, the choice of report format, writing
//! the files, and the exit code.
//!
//! # Exit codes
//!
//! | code | meaning |
//! |---|---|
//! | `0` | clean: nothing was reported |
//! | `1` | diagnostics were reported, or `build --check` found drift |
//! | `2` | the command could not run: bad usage, an unreadable entrypoint, or an output directory that could not be written |
//!
//! The split between `1` and `2` is the difference between *the composition is
//! wrong* and *the command could not be run against it*. A missing `imports:`
//! entry is the composition's problem and exits `1` with a diagnostic naming the
//! file; an entrypoint that is not a readable file, or an `--out` whose `src/`
//! holds files this compiler did not write (see [`build::write`]), is the
//! command's own precondition and exits `2` with a plain message, because there
//! is no span to point at. Argument errors are clap's, which exits `2` for them
//! already.
//!
//! `build --check` uses the same three codes for the same three meanings: `1` is
//! "the answer is no" — the composition is invalid, or the directory no longer
//! matches it — which is what a CI step branches on (PRD §8).
//!
//! # A reader that stops reading
//!
//! `validate` is a command people pipe: `| head`, `| less` and then `q`, a CI
//! wrapper reading the first *n* bytes of `--format json`. Each of those closes
//! its end of the pipe while the report is still being written, and the write
//! that follows fails with `BrokenPipe` — which `print!` and `eprint!` answer by
//! panicking. That is exit `101`, a code the table above gives no meaning, plus
//! a Rust backtrace on the very stream `--format json` promises to leave empty
//! (see [`report`]), and neither says anything about the composition. It shows
//! up only once a report outgrows the pipe's buffer, so it arrives with a
//! project's growth rather than with a change.
//!
//! So every write of a report goes through [`write()`], and a reader's departure
//! is not this command's failure: what is left of the report is dropped and the
//! exit code is still the **verdict** — the run answered its question, and the
//! reader took as much of the answer as it wanted. A stream that fails for any
//! other reason (a full disk under `> report.json`) is the command's own
//! precondition failing, and exits `2` with a message like the rest of them.
//!
//! # Targets
//!
//! `--target` selects `deploy/<name>.yml` and, with it, the answer to the
//! target-dependent rules: a store's `backend:` alias, an `event` trigger's
//! `source:`, and `detach: true` (grammar 14, Decisions D59, D87). Omitting it
//! resolves the built-in `local` target, which requires no deploy file at all.

mod build;
mod report;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use compose_core::{DEFAULT_TARGET, Diagnostics, Ir, resolve_with_target};

/// Nothing was reported.
const CLEAN: u8 = 0;
/// Diagnostics were reported.
const REPORTED: u8 = 1;
/// The command could not run.
const UNUSABLE: u8 = 2;

#[derive(Parser)]
#[command(name = "agent-compose", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a spec: parse, resolve imports, and run every static check
    Validate {
        /// Path to the spec entrypoint (conventionally `main.yml`)
        path: PathBuf,
        /// Deploy target the target-dependent rules are checked against
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TARGET)]
        target: String,
        /// How to report what was found
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Compile a spec to a TypeScript project (validates first; emits only when clean)
    Build {
        /// Path to the spec entrypoint (conventionally `main.yml`)
        path: PathBuf,
        /// Deploy target to resolve and emit for
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TARGET)]
        target: String,
        /// Where to write the generated project [default: <project>/build/<target>]
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// Write nothing; report whether the directory already matches the spec
        #[arg(long)]
        check: bool,
        /// How to report what was found
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
}

/// How a report is written.
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Rendered snippets on stderr.
    Human,
    /// One JSON object on stdout.
    Json,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate {
            path,
            target,
            format,
        } => validate(&path, &target, format),
        Command::Build {
            path,
            target,
            out,
            check,
            format,
        } => {
            let out = out.unwrap_or_else(|| build::default_out(&path, &target));
            build_project(&path, &target, &out, check, format)
        }
    }
}

/// Everything both commands do before they differ: resolve, check, and hand back
/// the report alongside the artifact.
///
/// The checks read the artifact, and a composition that was rejected has none:
/// half an artifact would send them chasing failures resolution has already named
/// (see `compose_core::resolve`).
fn analyse(entrypoint: &Path, target: &str) -> (Vec<compose_core::Diagnostic>, Option<Ir>) {
    let resolution = resolve_with_target(entrypoint, target);
    let mut report = Diagnostics::new();
    report.extend(resolution.diagnostics);
    if let Some(ir) = &resolution.ir {
        report.extend(compose_core::check(ir));
    }
    // Both passes sort their own output; the concatenation needs sorting once
    // more so the whole report reads in one source order.
    report.sort();
    (report.into_vec(), resolution.ir)
}

fn validate(entrypoint: &Path, target: &str, format: Format) -> ExitCode {
    if let Err(reason) = usable(entrypoint) {
        return fail(&reason);
    }

    let (diagnostics, _) = analyse(entrypoint, target);
    let verdict = if diagnostics.is_empty() {
        CLEAN
    } else {
        REPORTED
    };
    let written = match format {
        Format::Json => match report::json(&diagnostics) {
            Ok(text) => write(&mut io::stdout().lock(), &text),
            Err(error) => return fail(&format!("cannot write the report as JSON: {error}")),
        },
        Format::Human => {
            let color = report::color_enabled();
            let root = entrypoint.parent().unwrap_or_else(|| Path::new(""));
            let mut stream = io::stderr().lock();
            write(&mut stream, &report::human(root, &diagnostics, color)).and_then(|()| {
                write(
                    &mut stream,
                    &report::verdict(entrypoint, target, &diagnostics, color),
                )
            })
        }
    };

    match written {
        Ok(()) => ExitCode::from(verdict),
        // The reader closed the pipe: it has as much of the report as it asked
        // for, and the verdict is still the answer to the question the exit code
        // asks (see the module header).
        Err(error) if departed(&error) => ExitCode::from(verdict),
        Err(error) => fail(&format!("cannot write the report: {error}")),
    }
}

/// `agent-compose build`, and `build --check`.
///
/// Validation comes first and **any** diagnostic refuses the emission — a
/// warning included. Generated code is a build artifact of a valid composition
/// (PRD 5.12), and a project emitted from one the compiler had something to say
/// about would report that thing again, later, as a `tsc` error with no span.
///
/// A clean validation is not on its own enough to emit: `validate` answers "is
/// this composition well formed", which is target-independent, and codegen has
/// its own preconditions about what *this* target can express. Those are
/// `compose_core::target_diagnostics`, and they join the same report, so the two
/// are one verdict and one exit code rather than two passes a caller has to
/// remember to run in order.
fn build_project(
    entrypoint: &Path,
    target: &str,
    out: &Path,
    checking: bool,
    format: Format,
) -> ExitCode {
    if let Err(reason) = usable(entrypoint) {
        return fail(&reason);
    }

    let (validation, ir) = analyse(entrypoint, target);
    // What the *target* cannot express, over a composition the validator
    // accepted: `pattern:` is RE2 and RE2 is not a subset of ECMAScript, so a
    // legal pattern can be one no JavaScript regular expression holds
    // (`compose_core::codegen::diagnostics`). It is asked only once validation
    // is clean, because a composition that does not resolve has no artifact to
    // ask about, and a second report about the same broken file would bury the
    // first.
    let mut report = Diagnostics::new();
    report.extend(validation);
    let validated = report.is_empty();
    if let (Some(ir), true) = (&ir, validated) {
        report.extend(compose_core::target_diagnostics(ir));
    }
    report.sort();
    let diagnostics = report.into_vec();
    // A composition that validated and still has something reported against it
    // is one this target cannot express, which is a different sentence from
    // "not valid" — see `report::build_verdict`.
    let target_only = validated && !diagnostics.is_empty();

    let project = match (&ir, diagnostics.is_empty()) {
        (Some(ir), true) => Some(compose_core::emit(ir)),
        _ => None,
    };

    let drift = match (&project, checking) {
        (Some(project), true) => match build::check(project, out) {
            Ok(drift) => drift,
            Err(error) => {
                return fail(&format!("cannot read `{}`: {error}", out.display()));
            }
        },
        _ => Vec::new(),
    };
    let wrote = match (&project, checking) {
        (Some(project), false) => match build::write(project, out) {
            Ok(written) => Some(written),
            Err(build::Refusal::Io(error)) => {
                return fail(&format!("cannot write `{}`: {error}", out.display()));
            }
            Err(build::Refusal::NotOurs(paths)) => {
                return fail(&format!(
                    "`{}` holds {} this compiler did not write ({}), and `src/` is a directory \
                     `build` owns outright: it replaces what it emits and removes the rest. Point \
                     `--out` at a directory of its own, or move those files out of `src/`",
                    out.display(),
                    if paths.len() == 1 { "a file" } else { "files" },
                    paths
                        .iter()
                        .map(|path| format!("`{path}`"))
                        .collect::<Vec<_>>()
                        .join(", "),
                ));
            }
        },
        _ => None,
    };

    let verdict = if diagnostics.is_empty() && drift.is_empty() {
        CLEAN
    } else {
        REPORTED
    };
    let built = report::Built {
        diagnostics: &diagnostics,
        drift: &drift,
        wrote: wrote.as_ref(),
        target_only,
    };
    let written = match format {
        Format::Json => match report::build_json(&diagnostics, &drift) {
            Ok(text) => write(&mut io::stdout().lock(), &text),
            Err(error) => return fail(&format!("cannot write the report as JSON: {error}")),
        },
        Format::Human => {
            let color = report::color_enabled();
            let root = entrypoint.parent().unwrap_or_else(|| Path::new(""));
            let mut stream = io::stderr().lock();
            write(&mut stream, &report::human(root, &diagnostics, color)).and_then(|()| {
                write(
                    &mut stream,
                    &report::build_verdict(entrypoint, target, out, &built, color),
                )
            })
        }
    };

    match written {
        Ok(()) => ExitCode::from(verdict),
        Err(error) if departed(&error) => ExitCode::from(verdict),
        Err(error) => fail(&format!("cannot write the report: {error}")),
    }
}

/// Write a whole report to a stream.
///
/// Every report goes through this rather than through `print!`, which answers a
/// failed write by panicking. The flush is half of the write: `stdout` is line
/// buffered, so a stream whose reader has gone often *takes* the bytes and
/// refuses only when they are pushed through — a delivery that stopped at
/// `write_all` would call that report delivered.
fn write(stream: &mut impl Write, text: &str) -> io::Result<()> {
    stream.write_all(text.as_bytes())?;
    stream.flush()
}

/// Whether a stream's error is the reader having left rather than the stream
/// having failed.
fn departed(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::BrokenPipe
}

/// Report that the command itself could not run, and exit `2`.
///
/// Best effort by construction: the one place to say so is stderr, which may be
/// the stream that just failed, so a message that cannot be delivered is dropped
/// rather than panicked over — the exit code carries it instead.
fn fail(reason: &str) -> ExitCode {
    let _ = write(&mut io::stderr().lock(), &format!("error: {reason}\n"));
    ExitCode::from(UNUSABLE)
}

/// Whether there is an entrypoint to read at all.
fn usable(entrypoint: &Path) -> Result<(), String> {
    match std::fs::metadata(entrypoint) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!(
            "`{}` is not a file: `validate` takes a spec entrypoint, conventionally `main.yml`",
            entrypoint.display()
        )),
        Err(error) => Err(format!("cannot read `{}`: {error}", entrypoint.display())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stream that takes every write and fails only when it is flushed —
    /// which is what a buffered stream whose reader has gone does.
    struct Buffered(io::ErrorKind);

    impl Write for Buffered {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Err(io::Error::from(self.0))
        }
    }

    /// `tests/cli.rs` pins the exit code a closed pipe produces through the real
    /// binary; what it cannot show is *where* the failure surfaces. A report is
    /// not delivered until it is flushed, so both halves of a write are the
    /// write — a helper that returned after `write_all` would report a delivery
    /// that never left the buffer, and the exit code would be the verdict of a
    /// report nobody received.
    #[test]
    fn a_report_is_not_delivered_until_it_is_flushed() {
        let error = write(&mut Buffered(io::ErrorKind::BrokenPipe), "a report\n")
            .expect_err("the stream refuses the flush");
        assert!(departed(&error), "a closed pipe is the reader leaving");

        let error = write(&mut Buffered(io::ErrorKind::PermissionDenied), "a report\n")
            .expect_err("the stream refuses the flush");
        assert!(!departed(&error), "anything else is the stream failing");

        let mut delivered = Vec::new();
        write(&mut delivered, "a report\n").expect("a stream that takes it");
        assert_eq!(delivered, b"a report\n");
    }
}
