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
//! # Targets
//!
//! `--target` selects `deploy/<name>.yml` and, with it, the answer to the
//! target-dependent rules: a store's `backend:` alias, an `event` trigger's
//! `source:`, and `detach: true` (grammar 14, Decisions D59, D87). Omitting it
//! resolves the built-in `local` target, which requires no deploy file at all.

mod report;

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
        eprintln!("error: {reason}");
        return ExitCode::from(UNUSABLE);
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

    match format {
        Format::Json => match report::json(&diagnostics) {
            Ok(text) => print!("{text}"),
            Err(error) => {
                eprintln!("error: cannot write the report as JSON: {error}");
                return ExitCode::from(UNUSABLE);
            }
        },
        Format::Human => {
            let color = report::color_enabled();
            let root = entrypoint.parent().unwrap_or_else(|| Path::new(""));
            eprint!("{}", report::human(root, &diagnostics, color));
            eprint!(
                "{}",
                report::verdict(entrypoint, target, &diagnostics, color)
            );
        }
    }

    if diagnostics.is_empty() {
        ExitCode::from(CLEAN)
    } else {
        ExitCode::from(REPORTED)
    }
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
