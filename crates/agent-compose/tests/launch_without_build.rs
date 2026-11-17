//! `run --no-build`: a tree a build already wrote, launched **as it stands**
//! (PRD resolved q87, Decision D170).
//!
//! Without the flag every launching verb validates and builds first, and
//! `tests/compiled_graph_acceptance.rs` is where that path is decided. This
//! file decides the other one, through the real binary over the real emitted
//! project:
//!
//! * the tree runs and the spec is never read — the entrypoint is replaced by
//!   text no parser accepts, then removed, and the run still answers;
//! * a directory no build wrote, and a tree another release wrote, are refused
//!   before anything is launched, exit `2`, in the sentences `docs/topics/cli.md`
//!   gives;
//! * the environment precondition still runs, read out of the tree's own
//!   `src/env.ts`, and names the variable and the site that writes it exactly as
//!   a build-first run does;
//! * a hand-edited tree restamped by `build --restamp` runs, edit and all.
//!
//! `serve --no-build` and `cancel --no-build` are driven in
//! `tests/cancellation_acceptance.rs`, and a hub served from a restamped tree
//! handing a worker its artifact in `tests/distributed_mesh_acceptance.rs`;
//! what `--restamp` writes is `tests/build_cli.rs`'s.

#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use mock_provider::{MockProvider, Outcome, Script};
use serde_json::json;

/// The model `agent.reviewer` routes to in the `agent-anthropic` fixture.
const SONNET: &str = "claude-sonnet-4-6";

/// The fixture every run here launches: `flow.review` is one model call.
const FIXTURE: &str = "agent-anthropic";

/// The compiler under test, with nothing from this process's environment but
/// what [`harness::seal`] passes on.
fn compiler(environment: &[(String, String)]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
    harness::seal(&mut command, environment);
    command
}

/// `agent-compose build <entrypoint> [--out <out>]`, which must succeed.
fn built(entrypoint: &Path, out: Option<&Path>) {
    let mut command = Command::new(env!("CARGO_BIN_EXE_agent-compose"));
    command.arg("build").arg(entrypoint).env("NO_COLOR", "1");
    if let Some(out) = out {
        command.arg("--out").arg(out);
    }
    let output = command.output().expect("the compiler runs");
    assert!(
        output.status.success(),
        "the build this test launches did not succeed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `agent-compose run <entrypoint> flow.review --no-build …`, with the review's
/// two inputs.
fn run_as_built(
    entrypoint: &Path,
    out: Option<&Path>,
    environment: &[(String, String)],
) -> harness::Run {
    let mut command = compiler(environment);
    command
        .arg("run")
        .arg(entrypoint)
        .arg("flow.review")
        .args(["--input", "goal=ship it", "--input", "draft=a draft"])
        .arg("--no-build");
    if let Some(out) = out {
        command.arg("--out").arg(out);
    }
    harness::Run {
        output: command.output().expect("the compiler runs"),
    }
}

/// One scripted approval for `agent.reviewer`.
fn approving(provider: &MockProvider) {
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));
}

/// What a refused command wrote on stderr, after asserting it was refused as
/// a command that could not run: exit `2`, nothing on stdout.
#[track_caller]
fn refused(output: &Output) -> String {
    assert_eq!(
        output.status.code(),
        Some(2),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "",
        "a refused launch answers nothing on stdout"
    );
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// A scratch directory outside the toolchain, for a refusal that is made
/// before any runtime would be reached.
fn elsewhere(purpose: &str) -> harness::Scratch {
    harness::Scratch::new(&format!("no-build-{purpose}"))
}

/// **`run --no-build` runs the tree in the output directory and never reads
/// the spec** (PRD resolved q87).
///
/// The fixture is copied beneath the toolchain and built to its **default**
/// output directory, `<project>/build/local`, so the run below names no
/// `--out` at all: the spec path is what locates the tree, which is the whole
/// of what it is for under the flag. The spec is then overwritten with text no
/// parser accepts — a run that read it would be refused over it — and the run
/// answers the flow's outputs; then it is removed outright, and the run answers
/// again. Nothing is written beside it either: the garbage is the bytes it was.
#[test]
fn run_without_a_build_runs_the_tree_and_never_reads_the_spec() {
    let Some(project) = harness::scratch_project("no-build-run") else {
        return;
    };
    let _cleanup = harness::Scratch::at(project.clone());
    harness::copy_tree(&harness::projects().join(FIXTURE), &project);
    let entrypoint = project.join("main.yml");
    built(&entrypoint, None);

    let garbage = "this: [is not, a composition\n";
    std::fs::write(&entrypoint, garbage).expect("the copy is writable");
    let provider = MockProvider::start().expect("a loopback port");
    approving(&provider);
    let run = run_as_built(&entrypoint, None, &harness::environment(&provider));
    run.succeeded();
    assert_eq!(run.outputs()["verdict"], "approve");
    assert_eq!(
        std::fs::read_to_string(&entrypoint).expect("the spec is still there"),
        garbage,
        "the spec was neither read nor written"
    );

    std::fs::remove_file(&entrypoint).expect("the copy is removable");
    approving(&provider);
    let again = run_as_built(&entrypoint, None, &harness::environment(&provider));
    again.succeeded();
    assert_eq!(again.outputs()["verdict"], "approve");
    assert!(
        provider.snapshot().is_drained(),
        "each run made its one model call"
    );
}

/// **A directory no build wrote is refused, naming the build to run** — exit
/// `2`, before anything is launched: one that does not exist, and one that
/// exists and holds no `src/artifact.ts`.
#[test]
fn run_without_a_build_refuses_a_directory_no_build_wrote() {
    let scratch = elsewhere("absent");
    for out in [
        scratch.path().join("never-built"),
        scratch.path().to_path_buf(),
    ] {
        let output = run_as_built(&scratch.path().join("main.yml"), Some(&out), &[]).output;
        assert_eq!(
            refused(&output),
            format!(
                "error: `{}` holds no built project; run `agent-compose build` first or drop \
                 `--no-build`\n",
                out.display()
            )
        );
    }
}

/// **A tree another release wrote is refused, naming both releases** — exit
/// `2`. The tree is this compiler's own build with its `COMPILER_VERSION` line
/// rewritten, which is exactly what a tree an older binary left looks like to
/// this one.
///
/// And a tree whose file at a name the compiler writes no longer carries the
/// generated-file header is refused too, naming the file: an edit that kept the
/// header is still the compiler's file (that is `--restamp`'s case), one that
/// took it off is not.
#[test]
fn run_without_a_build_refuses_a_tree_another_release_or_nobody_wrote() {
    let scratch = elsewhere("other-release");
    let out = scratch.path().join("out");
    built(&harness::fixture(FIXTURE), Some(&out));
    let module = out.join("src/artifact.ts");
    let written = std::fs::read_to_string(&module).expect("the build wrote its stamp");
    let ours = format!(
        "export const COMPILER_VERSION = \"{}\";",
        compose_core::codegen::COMPILER_VERSION
    );
    assert!(written.contains(&ours), "{written}");
    std::fs::write(
        &module,
        written.replace(
            &ours,
            "export const COMPILER_VERSION = \"0.1.0-elsewhere\";",
        ),
    )
    .expect("the tree is writable");

    let output = run_as_built(&scratch.path().join("main.yml"), Some(&out), &[]).output;
    assert_eq!(
        refused(&output),
        format!(
            "error: `{}` was built by agent-compose 0.1.0-elsewhere; this is {} — rebuild, or run \
             it with the compiler that built it\n",
            out.display(),
            compose_core::codegen::COMPILER_VERSION
        )
    );

    std::fs::write(&module, &written).expect("the tree is writable");
    let graph = out.join("src/graph.ts");
    let body: String = std::fs::read_to_string(&graph)
        .expect("the build wrote the graph")
        .lines()
        .skip(1)
        .map(|line| format!("{line}\n"))
        .collect();
    assert!(!body.contains("generated by agent-compose"), "{body}");
    std::fs::write(&graph, body).expect("the tree is writable");
    let said = refused(&run_as_built(&scratch.path().join("main.yml"), Some(&out), &[]).output);
    assert!(
        said.starts_with(&format!(
            "error: `{}` holds a file this compiler did not write at a name a build writes \
             (`src/graph.ts`)",
            out.display()
        )) && said.ends_with("or drop `--no-build`\n"),
        "{said}"
    );

    std::fs::remove_file(&graph).expect("the tree is writable");
    let said = refused(&run_as_built(&scratch.path().join("main.yml"), Some(&out), &[]).output);
    assert_eq!(
        said,
        format!(
            "error: `{}` is not a whole build: `src/artifact.ts` lists `src/graph.ts` that is not \
             there; run `agent-compose build` first or drop `--no-build`\n",
            out.display()
        )
    );
}

/// **The environment precondition still runs**, before anything is launched:
/// a variable the tree references and this process does not set is refused by
/// name and by the site that writes it, in the words a build-first run uses
/// (`run_refuses_before_it_launches_when_a_variable_is_missing`) — read out of
/// the tree's own `src/env.ts`, since there is no composition to walk.
#[test]
fn run_without_a_build_still_refuses_an_unset_variable_by_name_and_site() {
    let Some(project) = harness::scratch_project("no-build-unset") else {
        return;
    };
    let _cleanup = harness::Scratch::at(project.clone());
    built(&harness::fixture(FIXTURE), Some(&project));
    let provider = MockProvider::start().expect("a loopback port");
    let environment: Vec<(String, String)> = harness::environment(&provider)
        .into_iter()
        .filter(|(name, _)| name != harness::API_KEY)
        .collect();

    let run = run_as_built(
        &project.join("no-spec").join("main.yml"),
        Some(&project),
        &environment,
    );
    let said = refused(&run.output);
    assert_eq!(
        said,
        format!(
            "error: an environment variable this composition references is not set: \
             `{}` (referenced by `provider.mock.api_key`, `provider.twin.api_key`). Environment \
             references are resolved when the graph runs, never at build (PRD 5.9), so this is \
             the first point the values are needed\n",
            harness::API_KEY
        )
    );
    assert!(
        provider.requests().is_empty(),
        "nothing was launched, so nothing was called"
    );
}

/// **A hand-edited tree, restamped, runs without a build — edit and all**
/// (PRD resolved q87).
///
/// The edit is one line in the generated `src/index.ts`, after its environment
/// check, that writes a marker to stderr: the one change a run of this tree can
/// show and a run of a rebuilt one cannot. `build --restamp` moves the stamp,
/// and the run that follows answers the flow's outputs with the marker on its
/// stderr — so the restamp left a tree that still loads, and the launch ran
/// the bytes on disk rather than anything it re-emitted.
#[test]
fn a_restamped_tree_runs_without_a_build_with_its_edit() {
    let Some(project) = harness::scratch_project("no-build-restamped") else {
        return;
    };
    let _cleanup = harness::Scratch::at(project.clone());
    built(&harness::fixture(FIXTURE), Some(&project));
    let before = stamp_of(&project);

    let index = project.join("src/index.ts");
    let source = std::fs::read_to_string(&index).expect("the build wrote the barrel");
    let marker = "a hand edit ran: 3f9c";
    let edited = source.replacen(
        "readEnvironment();\n",
        &format!("readEnvironment();\nprocess.stderr.write(\"{marker}\\n\");\n"),
        1,
    );
    assert_ne!(
        edited, source,
        "the barrel calls `readEnvironment()` on a line of its own"
    );
    std::fs::write(&index, edited).expect("the tree is writable");

    let restamped = Command::new(env!("CARGO_BIN_EXE_agent-compose"))
        .arg("build")
        .arg(project.join("no-spec").join("main.yml"))
        .arg("--out")
        .arg(&project)
        .arg("--restamp")
        .env("NO_COLOR", "1")
        .output()
        .expect("the compiler runs");
    assert!(
        restamped.status.success(),
        "{}",
        String::from_utf8_lossy(&restamped.stderr)
    );
    let after = stamp_of(&project);
    assert_ne!(after, before, "the restamp moved the hash");

    let provider = MockProvider::start().expect("a loopback port");
    approving(&provider);
    let run = run_as_built(
        &project.join("no-spec").join("main.yml"),
        Some(&project),
        &harness::environment(&provider),
    );
    run.succeeded();
    assert_eq!(run.outputs()["verdict"], "approve");
    assert!(
        run.stderr().contains(marker),
        "the run launched the edited tree: {}",
        run.stderr()
    );
}

/// The `ARTIFACT_HASH` a tree carries.
fn stamp_of(tree: &Path) -> String {
    let module: PathBuf = tree.join("src/artifact.ts");
    compose_core::codegen::artifact::Stamp::parse(
        &std::fs::read_to_string(module).expect("the tree has a stamp"),
    )
    .expect("the stamp reads")
    .hash
}
