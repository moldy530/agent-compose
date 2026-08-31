//! `agent-compose` — the compiler's command line.
//!
//! The verbs fall into two groups. Seven act on a composition; the rest teach
//! the reader about compositions in general and are described at the bottom of
//! this header. The first is the product's core loop (PRD §7 M0):
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
//! The third reads **two** specs rather than one (PRD §7 M2):
//!
//! ```text
//! agent-compose plan <before> <after> [--format human|json]
//! ```
//!
//! It resolves both and reports what moved between them — components, topology,
//! interfaces, and what the validator now says that it did not. PRD §2's problem
//! is that "reviewing what changed in the topology requires reading router
//! functions"; this is the answer, and `docs/plan.md` is the normative account
//! of the document it writes. The work is `compose-core`'s
//! ([`compose_core::plan`]); what lives here is the two entrypoints, the choice
//! of format, and the exit code.
//!
//! The last three are invocation (PRD 5.11):
//!
//! ```text
//! agent-compose run <path> <flow> [--input k=v]... [--session <key>]
//!                                 [--target <name>] [--out <dir>]
//!                                 [--format human|json]
//! agent-compose resume <path> <execution> [--target <name>] [--out <dir>]
//!                                         [--format human|json]
//! agent-compose serve <path> [--host <host>] [--port <port>]
//!                            [--target <name>] [--out <dir>]
//!                            [--format human|json]
//! ```
//!
//! All three **build first**, exactly as `build` would and into the same
//! directory, and then launch the emitted project's own command line
//! (`bun src/index.ts …`, or `node` under the fallback). Nothing about invoking a
//! compiled graph lives in this binary — see [`launch`], which is also where the
//! two preconditions of *starting* something are checked: every `${ENV}`
//! reference has a value (PRD §9.15) and the pinned dependency set is installed
//! where the project can resolve it.
//!
//! The seventh takes no spec at all, and is the other half of a distributed
//! deployment (`docs/distributed.md`, PRD 5.10):
//!
//! ```text
//! agent-compose worker --hub <url> --claim <name>... --token-env <VAR>
//!                      [--data-dir <dir>]
//! ```
//!
//! A worker holds no checkout and no YAML: it joins the hub at `--hub`, is
//! served the compiled project, and executes the nodes placed on the names it
//! claims. It is grouped with the verbs that act on a composition because it
//! runs one — the composition simply arrives over the wire rather than off
//! disk — and everything about the protocol lives in [`worker`].
//!
//! `resume` is the durable half (PRD resolved q26–q29, `docs/durability.md`).
//! Every invocation journals its effects as it makes them, and a `run` that
//! crashed leaves an execution the journal holds open; `resume` re-runs that
//! execution's graph consuming the record read-only up to the frontier — the
//! first effect the journal does not hold — so nothing already done is issued a
//! second time. It takes no `--input` and no `--session`: the invocation it
//! replays is the one the journal recorded. `serve` needs no verb of its own,
//! because it recovers every open execution on start.
//!
//! `run`'s **stdout is the run's answer** — the flow's outputs as one JSON
//! object, or the whole record under `--format json` — so this side writes
//! nothing there; the build report goes to stderr, and to stdout only when the
//! composition was refused and there is no run to answer for. `serve`'s stdout
//! is the app's readiness line for the same reason.
//!
//! # Progressive discovery
//!
//! The remaining verbs take no spec at all (PRD §7 M2, resolved q23):
//!
//! ```text
//! agent-compose docs [<topic>]
//! agent-compose explain <code>
//! agent-compose schema
//! agent-compose init [<dir>]
//! agent-compose skill [--agent <name>] [--global]
//! ```
//!
//! They exist because a coding agent is this product's second audience (PRD
//! §1), and the one that arrives with nothing: a released binary, a directory,
//! and no checkout of this repository to read `docs/grammar.md` out of. So the
//! binary carries what it would have read. Every document is embedded
//! ([`compose_core::docs`]) and printed straight through — no file is looked
//! for at run time, and nothing is parsed on a path `validate` walks, which is
//! what keeps a surface measured in hundreds of kilobytes off the millisecond
//! budget.
//!
//! `docs` prints the topic index or one topic — the curriculum, sized for a
//! reader with a context window rather than for completeness, which is the
//! grammar's job. `explain` prints the expanded account of one diagnostic code;
//! every human report that carried a code — `validate`'s, the ones `build`,
//! `run` and `serve` print before refusing, and a `plan`'s, whose validation
//! section is codes and whose refusal is a diagnostic block — ends with a line
//! pointing at it (see [`report::explain_hint`]). `schema` writes the
//! published JSON Schema — the document an editor's `$schema` points at
//! (grammar Appendix B) — byte for byte. `init` writes a project that
//! validates, which is the loop's first step. `skill` prints or installs the
//! document a user hands their agent.
//!
//! Three of the five take a **name** — a topic, a code, an agent — and answer
//! one nobody defines the way a diagnostic would, with a suggestion where the
//! spelling is close. Two of them also print the vocabulary (see [`unknown`]);
//! `explain` does not, because there are dozens of codes and a reader typing
//! one has a report in front of them carrying the right spelling. All three
//! exit `2`: a name outside a closed set is the command failing to be runnable,
//! not a composition being refused.
//!
//! # Exit codes
//!
//! | code | meaning |
//! |---|---|
//! | `0` | clean: nothing was reported, or a `plan` was produced |
//! | `1` | diagnostics were reported, `build --check` found drift, a `run` produced no answer, a `plan`'s spec did not resolve, or a discovery verb found something already there and would not replace it |
//! | `2` | the command could not run: bad usage, an unreadable entrypoint, an output directory that could not be written, a missing environment variable or one carrying a value the command does not take (`AGENT_COMPOSE_INTERACTIVE`, grammar 8.7), an uninstalled dependency set, or no JavaScript runtime to launch |
//! | `3` | a `run` with nobody to ask stopped at a `human` pause (grammar 8.7, PRD 5.11) |
//!
//! `plan` reads the first two differently from every other verb, and the
//! difference is the point: a composition the validator rejects still has an
//! artifact to diff, so "the after spec introduces three errors" is a plan that
//! was produced — exit `0` — while a spec that does not **resolve** has no
//! artifact at all and there is nothing to compare, which is exit `1`. A CI step
//! branching on `plan` is asking "could this be planned", not "is it valid";
//! `validate` is the verb that asks the second question.
//!
//! `3` is the emitted project's own — [`launch`] passes a child's exit code
//! through — and it is a code rather than a shade of `1` because it asks for
//! something neither of the others does. `2` is an invocation to fix and `1` is a
//! run to look into; a run holding a pause did everything it was asked to and is
//! waiting on a person. A supervisor that read it as `1` would re-run a graph
//! whose effects have already happened.
//!
//! It is the **non-interactive** path and only that. A pause has two delivery
//! surfaces (grammar 8.7, PRD §9.21): the app's `POST /executions/:id/resume`,
//! and the terminal of a `run` whose standard input is one — where the emitted
//! CLI asks the question, reads the answer and carries on, so the run ends `0`
//! like any other. `3` is what is left when neither is available: standard input
//! is not a terminal, `AGENT_COMPOSE_INTERACTIVE=0` said not to ask, or the
//! terminal went away mid-run. Nothing here decides which of those happened; the
//! child does, and this passes its code through.
//!
//! The split between `1` and `2` is the difference between *the answer is no*
//! and *the command could not be run at all*. A missing `imports:`
//! entry is the composition's problem and exits `1` with a diagnostic naming the
//! file; an entrypoint that is not a readable file, or an `--out` holding files
//! this compiler did not write and would have replaced (see [`build::write`]),
//! is the command's own precondition and exits `2` with a plain message, because
//! there is no span to point at. Argument errors are clap's, which exits `2` for
//! them already.
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
mod discover;
mod launch;
mod plan;
mod report;
mod worker;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand, ValueEnum};
use compose_core::docs;
use compose_core::plan::SpecSide;
use compose_core::{Composition, DEFAULT_TARGET, Diagnostics, Ir, resolve_with_target};

/// Nothing was reported.
const CLEAN: u8 = 0;
/// Diagnostics were reported.
const REPORTED: u8 = 1;
/// The command could not run.
const UNUSABLE: u8 = 2;

/// What `--version` prints after the program name.
///
/// The crate version, the commit, and the target triple — `build.rs` composes
/// it and says why all three are there. A binary that was downloaded rather
/// than built is the case it exists for: the version alone does not say which
/// build of it a reader is holding, and a bug report that cannot name the
/// commit is a bug report about a moving target.
const VERSION: &str = env!("COMPOSE_VERSION_LINE");

#[derive(Parser)]
#[command(name = "agent-compose", version = VERSION, about)]
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
    /// Diff two specs: what changed in the components, the topology, the interfaces, and the
    /// validation
    Plan {
        /// The spec compared from: an entrypoint, or a project directory holding `main.yml`
        before: PathBuf,
        /// The spec compared to
        after: PathBuf,
        /// How to report what changed
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
    /// Run one flow, declared manual trigger or not (validates, builds, then launches)
    Run {
        /// Path to the spec entrypoint (conventionally `main.yml`)
        path: PathBuf,
        /// The flow to run, as a typed address (`flow.<name>`)
        flow: String,
        /// One input binding, repeatable: `--input goal=ship it`
        #[arg(long = "input", value_name = "FIELD=VALUE")]
        inputs: Vec<String>,
        /// The session identity session-scoped stores key off (grammar 11.3)
        #[arg(long, value_name = "KEY")]
        session: Option<String>,
        /// Deploy target to resolve and emit for
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TARGET)]
        target: String,
        /// Where to write the generated project [default: <project>/build/<target>]
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// How to report what was found, and how the run answers
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Replay a journaled execution to completion, re-issuing nothing it already did
    Resume {
        /// Path to the spec entrypoint (conventionally `main.yml`)
        path: PathBuf,
        /// The execution to resume, as `run` printed it (`execution: exec_…`)
        #[arg(value_name = "EXECUTION")]
        execution: String,
        /// Deploy target to resolve and emit for
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TARGET)]
        target: String,
        /// Where to write the generated project [default: <project>/build/<target>]
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// How to report what was found, and how the run answers
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Serve the project's `http` triggers (validates, builds, then launches the app)
    Serve {
        /// Path to the spec entrypoint (conventionally `main.yml`)
        path: PathBuf,
        /// The address to listen on
        #[arg(long, value_name = "HOST", default_value = "127.0.0.1")]
        host: String,
        /// The port to listen on; `0` takes one the operating system picks
        #[arg(long, value_name = "PORT", default_value_t = 0)]
        port: u16,
        /// Deploy target to resolve and emit for
        #[arg(long, value_name = "NAME", default_value = DEFAULT_TARGET)]
        target: String,
        /// Where to write the generated project [default: <project>/build/<target>]
        #[arg(long, value_name = "DIR")]
        out: Option<PathBuf>,
        /// How to report what was found
        #[arg(long, value_enum, default_value_t = Format::Human)]
        format: Format,
    },
    /// Execute the nodes a hub has placed on the names this process claims
    Worker {
        /// The hub's base URL, as the deployment publishes it
        #[arg(long, value_name = "URL")]
        hub: String,
        /// One placement name this worker claims, repeatable
        #[arg(long = "claim", value_name = "NAME")]
        claims: Vec<String>,
        /// The variable the join token is read from — the one `hub.join_token:` names
        #[arg(long, value_name = "VAR")]
        token_env: String,
        /// Where this worker keeps the artifacts it materialises [default: ~/.agent-compose/worker]
        #[arg(long, value_name = "DIR")]
        data_dir: Option<PathBuf>,
    },
    // The five that teach, after the five that act on a composition. Clap lists
    // subcommands in declaration order, and `--help` is the first thing a
    // coding agent reads: the order it prints is the grouping this header, the
    // `cli` topic and the skill's verb table all describe, or it contradicts
    // all three (`tests/discovery_surface_inventory.rs`).
    /// Print the topic index, or one topic: the grammar, sized to be read
    Docs {
        /// The topic to print; omit for the index
        #[arg(value_name = "TOPIC")]
        topic: Option<String>,
    },
    /// Explain one diagnostic code: what it protects, what triggers it, the fix
    Explain {
        /// The code, exactly as a diagnostic reports it
        #[arg(value_name = "CODE")]
        code: String,
    },
    /// Print the published JSON Schema, for an editor's `$schema` or a linter
    Schema,
    /// Write a minimal project that validates: one agent, one flow, one trigger
    Init {
        /// Where to write it; must be empty or absent [default: the current directory]
        #[arg(value_name = "DIR", default_value = ".")]
        directory: PathBuf,
    },
    /// Print the agent skill, or install it for a named coding agent
    Skill {
        /// Install for this agent instead of printing the bare document
        #[arg(long, value_name = "NAME")]
        agent: Option<String>,
        /// Install under `$HOME` rather than under the current directory
        #[arg(long)]
        global: bool,
    },
}

/// How a report is written.
///
/// One vocabulary for every verb, so `--format` reads the same wherever it is
/// typed — which is why the two lines below say *where* a report goes rather
/// than what it looks like. What it looks like is the verb's own: `validate` and
/// `build` draw a snippet under the source each diagnostic points at, while
/// `plan` writes a flat list of changes, which is a report rather than a
/// diagnostic and goes through no snippet renderer at all (`docs/plan.md` §13).
#[derive(Clone, Copy, ValueEnum)]
enum Format {
    /// Written for a person, on stderr.
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
        Command::Plan {
            before,
            after,
            format,
        } => plan_specs(&before, &after, format),
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
        Command::Run {
            path,
            flow,
            inputs,
            session,
            target,
            out,
            format,
        } => {
            let out = out.unwrap_or_else(|| build::default_out(&path, &target));
            let mut arguments = vec!["run".to_string(), flow];
            for binding in inputs {
                arguments.push("--input".to_string());
                arguments.push(binding);
            }
            if let Some(session) = session {
                arguments.push("--session".to_string());
                arguments.push(session);
            }
            arguments.push("--format".to_string());
            arguments.push(
                match format {
                    Format::Human => "human",
                    Format::Json => "json",
                }
                .to_string(),
            );
            launch(&path, "run", &target, &out, format, &arguments)
        }
        Command::Resume {
            path,
            execution,
            target,
            out,
            format,
        } => {
            let out = out.unwrap_or_else(|| build::default_out(&path, &target));
            // No `--input` and no `--session`: the invocation a resume replays is
            // the one the journal recorded, and a second set of inputs would be
            // one execution's record replayed into another execution's run
            // (PRD resolved q29). The emitted CLI reads both off the lifecycle
            // row.
            let arguments = vec![
                "resume".to_string(),
                execution,
                "--format".to_string(),
                match format {
                    Format::Human => "human",
                    Format::Json => "json",
                }
                .to_string(),
            ];
            launch(&path, "resume", &target, &out, format, &arguments)
        }
        Command::Worker {
            hub,
            claims,
            token_env,
            data_dir,
        } => worker::run(&worker::Options {
            hub,
            claims,
            token_env,
            data_dir,
        }),
        Command::Serve {
            path,
            host,
            port,
            target,
            out,
            format,
        } => {
            let out = out.unwrap_or_else(|| build::default_out(&path, &target));
            let arguments = vec![
                "serve".to_string(),
                "--host".to_string(),
                host,
                "--port".to_string(),
                port.to_string(),
            ];
            launch(&path, "serve", &target, &out, format, &arguments)
        }
        Command::Docs { topic } => docs(topic.as_deref()),
        Command::Explain { code } => explain(&code),
        Command::Schema => emit_text(compose_core::docs::SCHEMA),
        Command::Init { directory } => init(&directory),
        Command::Skill { agent, global } => skill(agent.as_deref(), global),
    }
}

/// `agent-compose run`, `agent-compose resume` and `agent-compose serve`: build,
/// then start.
///
/// The build half is [`build_project`]'s, run in the same order and refusing on
/// the same report — a composition the compiler has something to say about is
/// not one to launch. What it adds is the two preconditions of *starting*
/// something, both of them PRD §9.15's ("`run`/`serve` fail fast before invoking
/// the graph"): every `${ENV}` reference the composition makes has a value, and
/// the pinned dependency set is installed where the project can resolve it.
///
/// Nothing is written to stdout on this side. The child's stdout **is** the
/// command's answer — the flow's outputs for `run`, the readiness line for
/// `serve` — so a report that shared the stream would corrupt it; the build
/// report goes to stderr, or to stdout as JSON only when it is what the command
/// answers, which is when the composition was refused and nothing was launched.
fn launch(
    entrypoint: &Path,
    command: &str,
    target: &str,
    out: &Path,
    format: Format,
    arguments: &[String],
) -> ExitCode {
    // The verb is the caller's, not this function's: all three commands come
    // through here, and a `serve` told that "`run` takes a spec entrypoint" names
    // a command the caller did not type.
    if let Err(reason) = usable(entrypoint, command) {
        return fail(&reason);
    }

    let (validation, ir) = analyse(entrypoint, target);
    let mut report = Diagnostics::new();
    report.extend(validation);
    let validated = !report.has_errors();
    if let (Some(ir), true) = (&ir, validated) {
        report.extend(compose_core::target_diagnostics(ir));
    }
    report.sort();
    let diagnostics = report.into_vec();
    let Some(ir) = ir.filter(|_| !report::refuses(&diagnostics)) else {
        // The composition is the answer, so the report is what this command
        // writes — on the stream the format names, exactly as `validate` does.
        let written = match format {
            Format::Json => match report::json(&diagnostics) {
                Ok(text) => write(&mut io::stdout().lock(), &text),
                Err(error) => return fail(&format!("cannot write the report as JSON: {error}")),
            },
            Format::Human => {
                let color = report::color_enabled();
                let root = entrypoint.parent().unwrap_or_else(|| Path::new(""));
                let mut stream = io::stderr().lock();
                write(&mut stream, &report::human(root, &diagnostics, color))
                    .and_then(|()| {
                        write(
                            &mut stream,
                            &report::verdict(entrypoint, target, &diagnostics, color),
                        )
                    })
                    .and_then(|()| {
                        write(&mut stream, &report::explain_hint(!diagnostics.is_empty()))
                    })
            }
        };
        let _ = written;
        return ExitCode::from(REPORTED);
    };

    // The composition was accepted **with** something to say. The warnings are
    // printed and the launch goes ahead — that is what a warning is (see
    // `compose_core::diag::Severity`) — and they go to **stderr in both
    // formats**, because on this side the answer is the child's stdout: a JSON
    // report written there would interleave with the flow's outputs, which is
    // the one thing `--format json` promises never to do.
    warned(entrypoint, target, &diagnostics);

    let project = compose_core::emit(&ir);
    let scaffolds = compose_core::codegen::authored::scaffolds(&ir);
    match build::write(&project, &scaffolds, out, root(entrypoint)) {
        Ok(_) => {}
        Err(build::Refusal::Io(error)) => {
            return fail(&format!("cannot write `{}`: {error}", out.display()));
        }
        Err(build::Refusal::NotOurs(paths)) => {
            return fail(&occupied(out, &paths));
        }
    }

    let runtime = match launch::runtime()
        .and_then(|runtime| launch::environment(&ir).map(|()| runtime))
        .and_then(|runtime| launch::dependencies(out).map(|()| runtime))
    {
        Ok(runtime) => runtime,
        Err(launch::Refusal::Unusable(reason)) => return fail(&reason),
    };
    match launch::spawn(&runtime, out, arguments) {
        Ok(code) => ExitCode::from(code),
        Err(launch::Refusal::Unusable(reason)) => fail(&reason),
    }
}

/// Print the warnings of a composition that was accepted, on the one stream a
/// launched child never writes to.
///
/// A no-op for a clean report, which is nearly every run: nothing is printed,
/// not even a verdict, because `run` and `serve` answer with what they produced
/// and a line saying the spec was fine is noise in front of it. What is printed
/// when there *is* something is the same block `validate` prints, through the
/// same renderer, ending in the same pointer at `explain` — a reader who meets a
/// code here is further from `validate` than one who typed it.
fn warned(entrypoint: &Path, target: &str, diagnostics: &[compose_core::Diagnostic]) {
    if diagnostics.is_empty() {
        return;
    }
    let color = report::color_enabled();
    let root = entrypoint.parent().unwrap_or_else(|| Path::new(""));
    let mut stream = io::stderr().lock();
    let _ = write(&mut stream, &report::human(root, diagnostics, color))
        .and_then(|()| {
            write(
                &mut stream,
                &report::verdict(entrypoint, target, diagnostics, color),
            )
        })
        .and_then(|()| write(&mut stream, &report::explain_hint(true)));
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

/// The one question that needs the filesystem: is every `module:` binding's
/// authored file there (grammar 6.1, PRD resolved q48)?
///
/// Deliberately **not** part of [`analyse`], because the verbs differ on it. A
/// plain `build`, and the `run`/`serve` that build before they launch, *scaffold*
/// an absent module — so a refusal here would stop the very command that repairs
/// it. `validate` and `build --check` answer with a verdict instead, and this is
/// the sentence they answer with: the same diagnostic, naming `build` as the
/// repair.
fn authored(
    entrypoint: &Path,
    ir: Option<&Ir>,
    diagnostics: Vec<compose_core::Diagnostic>,
) -> Vec<compose_core::Diagnostic> {
    let Some(ir) = ir else {
        return diagnostics;
    };
    let mut report = Diagnostics::new();
    report.extend(diagnostics);
    report.extend(compose_core::check_modules(ir, root(entrypoint)));
    report.sort();
    report.into_vec()
}

fn validate(entrypoint: &Path, target: &str, format: Format) -> ExitCode {
    if let Err(reason) = usable(entrypoint, "validate") {
        return fail(&reason);
    }

    let (diagnostics, ir) = analyse(entrypoint, target);
    let diagnostics = authored(entrypoint, ir.as_ref(), diagnostics);
    // The exit code is the **verdict**, and a warning is not one: a composition
    // reported with nothing but warnings is one this compiler accepts, builds
    // and runs, so `validate` exits `0` and the report says what it noticed.
    // A CI step that wants warnings to fail reads the `warnings` array of
    // `--format json`, which is why that key is its own (see `report::json`).
    let verdict = if report::refuses(&diagnostics) {
        REPORTED
    } else {
        CLEAN
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
            write(&mut stream, &report::human(root, &diagnostics, color))
                .and_then(|()| {
                    write(
                        &mut stream,
                        &report::verdict(entrypoint, target, &diagnostics, color),
                    )
                })
                .and_then(|()| write(&mut stream, &report::explain_hint(!diagnostics.is_empty())))
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

/// `agent-compose plan`: two specs in, one report on what moved.
///
/// Both sides are resolved, and a side that does not resolve is the one thing
/// that stops the command: a plan is a diff of two **artifacts**, so a spec with
/// no artifact leaves nothing to compare, and what there is to report is that
/// spec's own diagnostics — rendered exactly as `validate` renders them,
/// because they are the same diagnostics about the same file.
///
/// Everything the **validator** says is content rather than a refusal. "The
/// after spec introduces two errors" is the sentence a plan exists to say
/// (`compose_core::plan`), so a plan that says it still exits `0`.
///
/// The target is the built-in `local` on both sides. `plan` takes no
/// `--target`: the question it answers is what changed in the composition, and
/// comparing one target's artifact against another's would answer a different
/// one — see `docs/plan.md`.
fn plan_specs(before: &Path, after: &Path, format: Format) -> ExitCode {
    let before_entrypoint = match entrypoint(before, SpecSide::Before) {
        Ok(path) => path,
        Err(reason) => return fail(&reason),
    };
    let after_entrypoint = match entrypoint(after, SpecSide::After) {
        Ok(path) => path,
        Err(reason) => return fail(&reason),
    };

    let before_resolution = resolve_with_target(&before_entrypoint, DEFAULT_TARGET);
    let after_resolution = resolve_with_target(&after_entrypoint, DEFAULT_TARGET);
    let before_root = root(&before_entrypoint);
    let after_root = root(&after_entrypoint);
    let before_name = before_entrypoint.display().to_string();
    let after_name = after_entrypoint.display().to_string();

    let held = match (&before_resolution.ir, &after_resolution.ir) {
        (Some(before_ir), Some(after_ir)) => compose_core::plan(
            Composition {
                entrypoint: &before_name,
                ir: before_ir,
                resolution: &before_resolution.diagnostics,
            },
            Composition {
                entrypoint: &after_name,
                ir: after_ir,
                resolution: &after_resolution.diagnostics,
            },
        ),
        _ => {
            let mut failed = Vec::new();
            for (side, resolution, name, root) in [
                (
                    SpecSide::Before,
                    &before_resolution,
                    &before_name,
                    before_root,
                ),
                (SpecSide::After, &after_resolution, &after_name, after_root),
            ] {
                if resolution.ir.is_none() {
                    failed.push(plan::Failed {
                        side,
                        entrypoint: name.clone(),
                        root,
                        diagnostics: &resolution.diagnostics,
                    });
                }
            }
            let written = match format {
                Format::Json => match plan::refusal(&failed).to_json() {
                    Ok(text) => write(&mut io::stdout().lock(), &text),
                    Err(error) => {
                        return fail(&format!("cannot write the report as JSON: {error}"));
                    }
                },
                Format::Human => write(
                    &mut io::stderr().lock(),
                    &plan::refused(&failed, DEFAULT_TARGET, report::color_enabled()),
                ),
            };
            return match written {
                Ok(()) => ExitCode::from(REPORTED),
                Err(error) if departed(&error) => ExitCode::from(REPORTED),
                Err(error) => fail(&format!("cannot write the report: {error}")),
            };
        }
    };

    let written = match format {
        Format::Json => match held.to_json() {
            Ok(text) => write(&mut io::stdout().lock(), &text),
            Err(error) => return fail(&format!("cannot write the report as JSON: {error}")),
        },
        Format::Human => write(
            &mut io::stderr().lock(),
            &plan::human(before_root, after_root, &held),
        ),
    };

    match written {
        Ok(()) => ExitCode::from(CLEAN),
        // The reader closed the pipe: it has as much of the plan as it asked
        // for, and the plan was still produced (see the module header).
        Err(error) if departed(&error) => ExitCode::from(CLEAN),
        Err(error) => fail(&format!("cannot write the report: {error}")),
    }
}

/// The project root a composition's spans are relative to: its entrypoint's own
/// directory (grammar 1.4).
fn root(entrypoint: &Path) -> &Path {
    entrypoint.parent().unwrap_or_else(|| Path::new(""))
}

/// The spec file one side of a `plan` names: the file itself, or the `main.yml`
/// inside a directory.
///
/// `validate` takes a file and only a file, because there is one spec and the
/// path a person types is the one they mean. What a `plan` is given is usually
/// two *trees* — a checkout and a worktree, a branch exported beside the working
/// copy — so a directory carrying the conventional entrypoint (PRD 5.1) is taken
/// as naming it. Nothing else is guessed: a directory with no `main.yml` is the
/// command's own precondition failing, and says so.
fn entrypoint(path: &Path, side: SpecSide) -> Result<PathBuf, String> {
    let side = side.as_str();
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(path.to_path_buf()),
        Ok(metadata) if metadata.is_dir() => {
            let held = path.join("main.yml");
            match std::fs::metadata(&held) {
                Ok(metadata) if metadata.is_file() => Ok(held),
                _ => Err(format!(
                    "`{}` holds no `main.yml`: the {side} spec is a spec entrypoint, or a \
                     project directory that holds one",
                    path.display()
                )),
            }
        }
        Ok(_) => Err(format!(
            "`{}` is neither a file nor a directory: the {side} spec is a spec entrypoint, \
             conventionally `main.yml`",
            path.display()
        )),
        Err(error) => Err(format!(
            "cannot read the {side} spec `{}`: {error}",
            path.display()
        )),
    }
}

/// `agent-compose build`, and `build --check`.
///
/// Validation comes first and an **error** refuses the emission. Generated code
/// is a build artifact of a valid composition (PRD 5.12), and a project emitted
/// from one the compiler refused would report that refusal again, later, as a
/// `tsc` error with no span.
///
/// A **warning** does not refuse it, which is the whole of what the severity
/// means: the composition is one this compiler accepts, so the project is
/// written, the exit code is `0`, and the warnings are printed above the verdict
/// that says how many there were. `unknown-server-tool` is the case that makes
/// this load-bearing — a server tool the vendor shipped after this release has
/// to build (grammar 12.1, Decision D122).
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
    if let Err(reason) = usable(entrypoint, "build") {
        return fail(&reason);
    }

    let (validation, ir) = analyse(entrypoint, target);
    // `--check` is a verdict about a committed project, so it asks the one
    // question a plain `build` answers by writing instead: is every authored
    // implementation there? A `build` that refused over an absent module could
    // never scaffold the one it was refusing over (PRD resolved q48).
    let validation = if checking {
        authored(entrypoint, ir.as_ref(), validation)
    } else {
        validation
    };
    // What the *target* cannot express, over a composition the validator
    // accepted: `pattern:` is RE2 and RE2 is not a subset of ECMAScript, so a
    // legal pattern can be one no JavaScript regular expression holds
    // (`compose_core::codegen::diagnostics`). It is asked only once validation
    // is clean, because a composition that does not resolve has no artifact to
    // ask about, and a second report about the same broken file would bury the
    // first.
    let mut report = Diagnostics::new();
    report.extend(validation);
    let validated = !report.has_errors();
    if let (Some(ir), true) = (&ir, validated) {
        report.extend(compose_core::target_diagnostics(ir));
    }
    report.sort();
    let diagnostics = report.into_vec();
    let refused = report::refuses(&diagnostics);
    // A composition that validated and is still **refused** is one this target
    // cannot express, which is a different sentence from "not valid" — see
    // `report::build_verdict`. A warning does not put a build in that state, so
    // this is asked of the errors rather than of the report.
    let target_only = validated && refused;

    let project = match (&ir, refused) {
        (Some(ir), false) => Some(compose_core::emit(ir)),
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
    // Whether the `build` the drift report is about to recommend would run at
    // all. Asked only when there is drift to report a remedy for: a directory
    // that matches gets no help line, and a clean `--check` is the common CI
    // path, which is not the place for a second walk of the output directory.
    let not_ours = match (&project, drift.is_empty()) {
        (Some(project), false) => match build::not_ours(project, out) {
            Ok(paths) => paths,
            Err(error) => {
                return fail(&format!("cannot read `{}`: {error}", out.display()));
            }
        },
        _ => Vec::new(),
    };
    let wrote = match (&project, checking) {
        (Some(project), false) => {
            let scaffolds = ir
                .as_ref()
                .map(compose_core::codegen::authored::scaffolds)
                .unwrap_or_default();
            match build::write(project, &scaffolds, out, root(entrypoint)) {
                Ok(written) => Some(written),
                Err(build::Refusal::Io(error)) => {
                    return fail(&format!("cannot write `{}`: {error}", out.display()));
                }
                Err(build::Refusal::NotOurs(paths)) => return fail(&occupied(out, &paths)),
            }
        }
        _ => None,
    };

    let verdict = if refused || !drift.is_empty() {
        REPORTED
    } else {
        CLEAN
    };
    let built = report::Built {
        diagnostics: &diagnostics,
        drift: &drift,
        not_ours: &not_ours,
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
            write(&mut stream, &report::human(root, &diagnostics, color))
                .and_then(|()| {
                    write(
                        &mut stream,
                        &report::build_verdict(entrypoint, target, out, &built, color),
                    )
                })
                .and_then(|()| write(&mut stream, &report::explain_hint(!diagnostics.is_empty())))
        }
    };

    match written {
        Ok(()) => ExitCode::from(verdict),
        Err(error) if departed(&error) => ExitCode::from(verdict),
        Err(error) => fail(&format!("cannot write the report: {error}")),
    }
}

/// The refusal a build writes when an authored file already holds a name the
/// compiler emits (PRD resolved q47).
///
/// **Both sides, named.** The compiler's claim on an output directory is the
/// file list it emits and nothing else, so the only way a build can collide with
/// authored code is a name it has to write that somebody else's file is at —
/// and a message saying only "refused" leaves the reader to guess which of the
/// twenty-one it was. So it names the directory, the paths, and what the
/// compiler was going to do to them.
fn occupied(out: &Path, paths: &[String]) -> String {
    let (noun, pronoun) = if paths.len() == 1 {
        ("a file", "it")
    } else {
        ("files", "them")
    };
    format!(
        "`{}` holds {noun} this compiler did not write at {noun} it emits ({}), and this build \
         would have replaced {pronoun}: what a build owns is the file list it emits, and in a \
         directory holding none of its own files it claims none of those names — the \
         generated-file header is how it tells its work from yours. Point `--out` at a directory \
         of its own, or move {pronoun} aside",
        out.display(),
        paths
            .iter()
            .map(|path| format!("`{path}`"))
            .collect::<Vec<_>>()
            .join(", "),
    )
}

/// `agent-compose docs [<topic>]`: the index, or one topic.
///
/// A name nobody defines is the command's own precondition failing, so it exits
/// `2` — there is no composition here to be right or wrong about. It still
/// answers the way a diagnostic would (PRD G3): the valid names, and a
/// suggestion where one is close.
fn docs(topic: Option<&str>) -> ExitCode {
    let Some(name) = topic else {
        return emit_text(&docs::index());
    };
    match docs::topic(name) {
        Some(topic) => emit_text(topic.body),
        None => fail(&unknown(
            "a topic",
            "topics",
            name,
            docs::topics::nearest(name),
            &docs::topics::names(),
        )),
    }
}

/// `agent-compose explain <code>`: the expanded account of one diagnostic.
///
/// The argument is the code exactly as a report spells it, because that is what
/// a reader copies — and what `validate`'s own trailing line tells them to
/// paste (see [`report::explain_hint`]).
fn explain(code: &str) -> ExitCode {
    match docs::codes::named(code) {
        Some(code) => emit_text(docs::explanation(code)),
        // The one name that is *not* answered with its vocabulary. There are
        // dozens of codes, and a reader typing one already has a report in
        // front of them with the right spelling in its header — a wall of every
        // failure class this compiler has would bury the suggestion that is
        // actually useful.
        None => fail(&format!(
            "`{code}` is not a diagnostic code{}. Every code this compiler reports has an \
             explanation: copy the one from a report's `error[<code>]` header",
            docs::codes::nearest(code)
                .map_or_else(String::new, |near| format!(" (did you mean `{near}`?)")),
        )),
    }
}

/// `agent-compose skill [--agent <name>] [--global]`: print the skill, or
/// install it.
///
/// Bare, it prints the agent-agnostic document. `--agent` selects an install,
/// and every install is the same write under a different root, because the
/// agents read the same portable `SKILL.md` — see [`compose_core::docs::skill`].
/// `--global` names where an install writes, so it is refused beside the bare
/// posture, which writes nowhere: a flag that was silently ignored would leave a
/// user believing they had installed something under `$HOME`.
fn skill(agent: Option<&str>, global: bool) -> ExitCode {
    let Some(agent) = agent else {
        if global {
            return fail(
                "`--global` needs an `--agent`: it names where an install writes, and printing \
                 the skill writes nowhere",
            );
        }
        return emit_text(docs::skill::SKILL);
    };
    let Some(relative) = docs::skill::path(agent) else {
        return fail(&unknown(
            "an agent this command has an install posture for",
            "agents",
            agent,
            docs::skill::nearest(agent),
            &docs::skill::agents(),
        ));
    };
    // Relative under the current directory, rather than joined onto a `.`: the
    // path is the one line this verb prints, and `install` reports back what it
    // was given.
    let path = if global {
        match std::env::var_os("HOME") {
            Some(home) => PathBuf::from(home).join(&relative),
            None => {
                return fail(
                    "`--global` needs `HOME`, which is not set: run without it to install under \
                     the current directory",
                );
            }
        }
    } else {
        PathBuf::from(&relative)
    };
    match discover::install(&path, &docs::skill::installed()) {
        Ok(discover::Wrote::Created(path)) => {
            let _ = write(
                &mut io::stderr().lock(),
                &format!("wrote `{}`\n", path.display()),
            );
            ExitCode::from(CLEAN)
        }
        Ok(discover::Wrote::Unchanged(path)) => {
            let _ = write(
                &mut io::stderr().lock(),
                &format!("`{}` is already this skill\n", path.display()),
            );
            ExitCode::from(CLEAN)
        }
        Err(discover::Refusal::Occupied(reason)) => {
            let _ = write(&mut io::stderr().lock(), &format!("error: {reason}\n"));
            ExitCode::from(REPORTED)
        }
        Err(discover::Refusal::Unusable(reason)) => fail(&reason),
    }
}

/// The sentence a discovery verb answers a name nobody defines with.
///
/// The vocabulary, and a suggestion where the spelling is close. It is written
/// like a diagnostic's `help:` for the reason PRD G3 gives — error UX is a
/// product feature, and a coding agent reads this as its next instruction.
///
/// `noun`/`plural` are both taken rather than one pluralized, because English
/// does not pluralize by suffix reliably and a message reading "is not a agent"
/// is a message somebody stopped proofreading.
fn unknown(
    noun: &str,
    plural: &str,
    written: &str,
    nearest: Option<&str>,
    known: &[&str],
) -> String {
    let suggestion = nearest.map_or_else(String::new, |near| format!(" (did you mean `{near}`?)"));
    format!(
        "`{written}` is not {noun}{suggestion}. The {plural} are: {}",
        known
            .iter()
            .map(|name| format!("`{name}`"))
            .collect::<Vec<_>>()
            .join(", ")
    )
}

/// `agent-compose init [<dir>]`: write the scaffold, then say what to run.
///
/// The two lines after the write are the verb's actual product. A scaffold on
/// disk with nothing said about it leaves a reader where they started — the
/// point is the *loop*, and naming the command that starts it is what turns a
/// file into a first step. They go to **stderr**: `init`'s answer is the file,
/// and stdout stays free for the one thing a caller might redirect.
///
/// The `.` the argument defaults to is stripped back off before the path is
/// printed, for the reason [`skill`] never joins one on: both lines are meant to
/// be read and retyped, and `./main.yml` is not how a reader would type it.
fn init(directory: &Path) -> ExitCode {
    match discover::init(directory) {
        Ok(path) => {
            let path = path.strip_prefix(".").unwrap_or(&path);
            let _ = write(
                &mut io::stderr().lock(),
                &format!(
                    "wrote `{}`\n\nnext: `agent-compose validate {}`, then `agent-compose docs` \
                     for the topics\n",
                    path.display(),
                    path.display()
                ),
            );
            ExitCode::from(CLEAN)
        }
        Err(discover::Refusal::Occupied(reason)) => {
            let _ = write(&mut io::stderr().lock(), &format!("error: {reason}\n"));
            ExitCode::from(REPORTED)
        }
        Err(discover::Refusal::Unusable(reason)) => fail(&reason),
    }
}

/// Print one embedded document to stdout, and exit `0`.
///
/// The discovery verbs (`schema`, and the ones that follow it) answer with a
/// document rather than with a verdict: there is no composition to be right or
/// wrong about, so the only outcomes are "here it is" and a usage error clap
/// already reports. A reader that stops reading — `| head`, `| less` then `q` —
/// is not a failure for the same reason it is not one under `validate`: it took
/// as much of the answer as it wanted (see the module header).
fn emit_text(text: &str) -> ExitCode {
    match write(&mut io::stdout().lock(), text) {
        Ok(()) => ExitCode::from(CLEAN),
        Err(error) if departed(&error) => ExitCode::from(CLEAN),
        Err(error) => fail(&format!("cannot write to standard output: {error}")),
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
///
/// `command` is the subcommand asking, because the sentence that follows names
/// it — every command here takes an entrypoint, and one that told a `build` user
/// what `validate` takes would be answering a question nobody asked.
fn usable(entrypoint: &Path, command: &str) -> Result<(), String> {
    match std::fs::metadata(entrypoint) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(format!(
            "`{}` is not a file: `{command}` takes a spec entrypoint, conventionally `main.yml`",
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
