use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "agent-compose", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a spec: parse, resolve imports, and run all static checks
    Validate {
        /// Path to the spec entrypoint (main.yml)
        path: PathBuf,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.command {
        Command::Validate { path } => {
            eprintln!(
                "error: `validate` is not implemented yet (would validate {})",
                path.display()
            );
            ExitCode::from(2)
        }
    }
}
