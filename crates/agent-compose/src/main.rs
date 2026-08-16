//! `agent-compose` — the compiler's command line.
//!
//! One command exists in M0, and it is the product's core loop (PRD §7 M0):
//!
//! ```text
//! agent-compose validate <path> [--target <name>] [--format human|json]
//! ```
//!
//! It parses the entrypoint, follows its `imports:`, resolves every name, and
//! runs every static check the grammar defines, then reports. The work is all
//! `compose-core`'s; what lives here is the surface — argument parsing, the
//! choice of report format, and the exit code.
//!
//! # Exit codes
//!
//! | code | meaning |
//! |---|---|
//! | `0` | clean: nothing was reported |
//! | `1` | diagnostics were reported |
//! | `2` | the command could not run: bad usage, or an unreadable entrypoint |
//!
//! The split between `1` and `2` is the difference between *the composition is
//! wrong* and *there was no composition to look at*. A missing `imports:` entry
//! is the composition's problem and exits `1` with a diagnostic naming the file;
//! an entrypoint that is not a readable file is the command's own precondition
//! and exits `2` with a plain message, because there is no span to point at.
//! Argument errors are clap's, which exits `2` for them already.
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

mod report;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use compose_core::{DEFAULT_TARGET, Diagnostics, resolve_with_target};

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
    }
}

fn validate(entrypoint: &Path, target: &str, format: Format) -> ExitCode {
    if let Err(reason) = usable(entrypoint) {
        return fail(&reason);
    }

    let resolution = resolve_with_target(entrypoint, target);
    // The checks read the artifact, and a composition that was rejected has
    // none: half an artifact would send them chasing failures resolution has
    // already named (see `compose_core::resolve`).
    let mut report = Diagnostics::new();
    report.extend(resolution.diagnostics);
    if let Some(ir) = &resolution.ir {
        report.extend(compose_core::check(ir));
    }
    // Both passes sort their own output; the concatenation needs sorting once
    // more so the whole report reads in one source order.
    report.sort();
    let diagnostics = report.into_vec();

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
