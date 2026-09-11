//! `agent-compose visualize`, end to end through the real binary.
//!
//! The document itself is `compose-core`'s — its unit tests own the derivation
//! and `tests/graph_artifact_goldens.rs` owns the bytes. What this file owns is
//! the **command**: where it writes, what it refuses, what it says when it
//! refuses, and which of the exit codes it uses (see `main.rs`).
//!
//! The posture under test is PRD resolved q56's: `visualize` validates first
//! and emits only when clean, exactly as `build` does, and everything it can
//! refuse on its own is a **usage error** — rendering adds no failure class.
//!
//! Every run sets `NO_COLOR` and reads its streams through a pipe, like
//! `tests/build_cli.rs`, so nothing here depends on a terminal.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Output;
use std::sync::atomic::{AtomicU32, Ordering};

use assert_cmd::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Where the invalid-composition fixtures live, the way `tests/cli.rs` names
/// them.
fn projects() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects")
}

/// A directory the test writes into.
fn scratch(purpose: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "agent-compose-visualize-cli-{purpose}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("a scratch directory");
    path
}

/// Copy a directory tree, the way `plan_completeness.rs` plants a composition.
fn copy(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("can create a directory");
    for entry in fs::read_dir(from).expect("the source directory is readable") {
        let entry = entry.expect("a directory entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("a file type").is_dir() {
            copy(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("can copy a file");
        }
    }
}

/// `visualize`, run from the repository root.
fn visualize(arguments: &[&str]) -> Output {
    run_in(&repo_root(), arguments)
}

/// `visualize`, run from somewhere else — which is what pins the **default**
/// output path, since it is relative to the working directory.
fn run_in(directory: &Path, arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .arg("visualize")
        .args(arguments)
        .output()
        .expect("the command runs")
}

#[track_caller]
fn stdout(output: &Output) -> &str {
    std::str::from_utf8(&output.stdout).expect("stdout is UTF-8")
}

#[track_caller]
fn stderr(output: &Output) -> &str {
    std::str::from_utf8(&output.stderr).expect("stderr is UTF-8")
}

#[track_caller]
fn code(output: &Output) -> i32 {
    output.status.code().expect("the command was not signalled")
}

/// Both worked examples render, and what they render is one HTML file.
///
/// The two are deliberately different shapes — `triage-fanout` has a routed
/// `map`, a `human` node, a store and a subflow, and `review-loop` has a
/// bounded cycle and an `else:` escape — so a renderer that only ever saw one
/// topology is not what these prove.
#[test]
fn both_examples_render_to_one_self_contained_file() {
    for project in ["examples/triage-fanout", "examples/review-loop"] {
        let out = scratch("render").join("graph.html");
        let output = visualize(&[
            &format!("{project}/main.yml"),
            "-o",
            out.to_str().expect("a UTF-8 scratch path"),
        ]);
        assert_eq!(code(&output), 0, "{}", stderr(&output));
        assert_eq!(stdout(&output), "", "the page is the answer, not stdout");
        assert!(out.is_file(), "`{project}` wrote no page");

        let page = fs::read_to_string(&out).expect("the page is readable");
        assert!(
            page.starts_with("<!doctype html>"),
            "it is an HTML document"
        );
        assert!(
            page.contains("\"graph_version\": 1"),
            "it embeds the graph document"
        );
        assert!(
            stderr(&output).contains(&format!("wrote `{}`", out.display())),
            "the verdict names what it wrote: {}",
            stderr(&output)
        );
    }
}

/// With no `-o`, the page lands at `./graph.html` — in the **working
/// directory**, not beside the spec and not under `build/`.
///
/// A picture is something a person opens, not a build artifact of the
/// composition: `build --check` compares the emitted file list, and a page that
/// landed inside `--out` would be drift the next `build` refuses over (PRD
/// resolved q47).
///
/// The spec is **planted** rather than read where it lives, because the other
/// half of the claim — that nothing was written beside it — has to be asserted
/// against a directory no one else writes into. The repository root is the one
/// place this test may not look: it is where README.md and the `cli` topic tell
/// a reader to run `agent-compose visualize hello/main.yml`, so a `graph.html`
/// sitting there is a contributor having followed the documentation, not a bug
/// in the verb.
#[test]
fn the_default_path_is_graph_html_in_the_working_directory() {
    let elsewhere = scratch("default-spec");
    copy(&repo_root().join("examples/review-loop"), &elsewhere);
    let here = scratch("default");
    let spec = elsewhere.join("main.yml");
    let output = run_in(&here, &[spec.to_str().expect("a UTF-8 path")]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(here.join("graph.html").is_file(), "{}", stderr(&output));
    assert!(
        !elsewhere.join("graph.html").exists(),
        "the default path is relative to the working directory, not the spec's own"
    );
}

/// A path whose directory does not exist yet is made rather than refused.
#[test]
fn an_output_path_names_a_directory_the_command_will_make() {
    let out = scratch("nested")
        .join("reports")
        .join("deep")
        .join("g.html");
    let output = visualize(&[
        "examples/review-loop/main.yml",
        "-o",
        out.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(out.is_file(), "{}", stderr(&output));
}

/// `--format json` prints the graph document to stdout and writes no file.
#[test]
fn the_json_format_prints_the_document_to_stdout() {
    let here = scratch("json");
    let spec = repo_root().join("examples/triage-fanout/main.yml");
    let output = run_in(
        &here,
        &[spec.to_str().expect("a UTF-8 path"), "--format", "json"],
    );
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        !here.join("graph.html").exists(),
        "the document is the answer; nothing is written"
    );

    let document: serde_json::Value =
        serde_json::from_str(stdout(&output)).expect("stdout is one JSON document");
    assert_eq!(document["graph_version"], 1);
    assert_eq!(document["entrypoint"], "main.yml");
    assert_eq!(document["target"], "local");
    let flows = document["flows"].as_array().expect("a flows array");
    assert_eq!(
        flows
            .iter()
            .map(|flow| flow["address"].as_str().expect("an address"))
            .collect::<Vec<_>>(),
        ["flow.enrich", "flow.triage"],
        "the flows are sorted by address"
    );
}

/// `-o` beside `--format json` is a usage error, and it names the repair.
///
/// The two flags disagree about where the answer goes. It is refused **before**
/// the spec is read, so the report is one sentence rather than a resolution
/// spent on a command that was never going to write anything.
#[test]
fn an_output_path_beside_the_json_format_is_a_usage_error() {
    let output = visualize(&[
        "examples/review-loop/main.yml",
        "--format",
        "json",
        "-o",
        "graph.json",
    ]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "nothing was produced");
    let reported = stderr(&output);
    assert!(
        reported.contains("takes no `-o`") && reported.contains("--format html"),
        "the refusal names the repair: {reported}"
    );
}

/// `--flow` narrows the document to one canvas.
#[test]
fn the_flow_flag_narrows_the_document_to_one_canvas() {
    let output = visualize(&[
        "examples/triage-fanout/main.yml",
        "--flow",
        "flow.enrich",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let document: serde_json::Value =
        serde_json::from_str(stdout(&output)).expect("stdout is one JSON document");
    let flows = document["flows"].as_array().expect("a flows array");
    assert_eq!(flows.len(), 1, "one flow was asked for");
    assert_eq!(flows[0]["address"], "flow.enrich");
}

/// A `--flow` nobody declares is a usage error listing the flows that exist.
///
/// Exit `2` rather than `1`: a name outside a closed set is the command failing
/// to be runnable, not a composition being refused — the posture every other
/// name in this command line takes (see `main::unknown`).
#[test]
fn an_unknown_flow_is_refused_with_the_vocabulary() {
    let output = visualize(&[
        "examples/triage-fanout/main.yml",
        "--flow",
        "flow.triagee",
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "nothing was produced");
    let reported = stderr(&output);
    for expected in [
        "`flow.triagee` is not a flow this composition declares",
        "did you mean `flow.triage`?",
        "The flows are: `flow.enrich`, `flow.triage`",
    ] {
        assert!(
            reported.contains(expected),
            "the refusal does not carry `{expected}`: {reported}"
        );
    }
}

/// …and where the vocabulary is **empty**, it says so rather than trailing off.
///
/// A composition of `provider.*` and `model.*` definitions and no `flow.*` is
/// one the validator accepts, and it is the only spec that can reach the
/// refusal above with nothing to list — where the general phrasing would end at
/// "The flows are: " and name no repair, which PRD G3 does not allow. The bare
/// run afterwards is the repair being checked: dropping `--flow` really does
/// answer, rather than being advice into another refusal.
#[test]
fn a_composition_with_no_flows_is_refused_with_what_is_missing() {
    let home = scratch("flowless");
    let entrypoint = home.join("main.yml");
    fs::write(
        &entrypoint,
        r#"version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-6
"#,
    )
    .expect("the entrypoint is writable");
    let spec = entrypoint;
    let spec = spec.to_str().expect("a UTF-8 path");

    let output = run_in(&home, &[spec, "--flow", "flow.x", "--format", "json"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "nothing was produced");
    let reported = stderr(&output);
    for expected in [
        "`flow.x` cannot be drawn",
        "declares no flows at all",
        "Drop `--flow`",
    ] {
        assert!(
            reported.contains(expected),
            "the refusal does not carry `{expected}`: {reported}"
        );
    }
    assert!(
        !reported.contains("The flows are:"),
        "and it does not trail off into an empty list: {reported}"
    );

    let drawn = run_in(&home, &[spec, "--format", "json"]);
    assert_eq!(code(&drawn), 0, "{}", stderr(&drawn));
    let document: serde_json::Value =
        serde_json::from_str(stdout(&drawn)).expect("stdout is one JSON document");
    assert_eq!(document["graph_version"], 1);
    assert_eq!(
        document["flows"].as_array().expect("a flows array").len(),
        0,
        "the composition it drew is the one that has nothing to draw"
    );
}

/// A composition the validator refuses is refused here, **in the same words**.
///
/// Not "a similar report": the same bytes on the same stream, because both go
/// through `report::human` and both end in the line pointing at `explain`. PRD
/// resolved q56 states it — a `visualize`'s diagnostics are `validate`'s own —
/// and the only way to hold that is to compare the two.
#[test]
fn a_refused_composition_is_refused_in_validates_own_words() {
    let spec = "one-unbalanced-convergence/main.yml";
    let refused = run_in(&projects(), &[spec]);
    assert_eq!(code(&refused), 1, "{}", stderr(&refused));
    assert_eq!(stdout(&refused), "", "no document is produced");

    let validated = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(projects())
        .env("NO_COLOR", "1")
        .args(["validate", spec])
        .output()
        .expect("the command runs");
    assert_eq!(code(&validated), 1);
    assert_eq!(
        stderr(&refused),
        stderr(&validated),
        "the two reports have drifted apart"
    );
    assert!(
        stderr(&refused).contains("agent-compose explain "),
        "and it ends in the pointer at `explain`: {}",
        stderr(&refused)
    );
}

/// …and it writes nothing while refusing, under either format.
#[test]
fn a_refused_composition_leaves_no_artifact_behind() {
    let here = scratch("refused");
    let spec = projects().join("one-parse-error/main.yml");
    let output = run_in(&here, &[spec.to_str().expect("a UTF-8 path")]);
    assert_eq!(code(&output), 1, "{}", stderr(&output));
    assert!(
        !here.join("graph.html").exists(),
        "a refused composition draws nothing"
    );
    assert_eq!(stdout(&output), "", "and prints no document either");
}

/// An entrypoint that is not a file is the command's own precondition failing.
#[test]
fn an_unreadable_entrypoint_names_the_verb_that_was_typed() {
    let output = visualize(&["examples"]);
    assert_eq!(code(&output), 2, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("`visualize` takes a spec entrypoint"),
        "the message names the verb the caller typed: {}",
        stderr(&output)
    );
}

/// `--help` writes the usage line the documents are held to.
///
/// One required positional and no required option, which is what
/// `tests/discovery_surface_inventory.rs` reads when it checks that every
/// written `agent-compose visualize …` would actually run.
#[test]
fn the_usage_line_requires_one_positional_and_no_option() {
    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .args(["visualize", "--help"])
        .output()
        .expect("the command runs");
    let text = String::from_utf8(output.stdout).expect("help is UTF-8");
    let usage = text
        .lines()
        .find(|line| line.starts_with("Usage:"))
        .expect("a usage line");
    assert_eq!(usage, "Usage: agent-compose visualize [OPTIONS] <PATH>");
    for flag in ["--flow", "-o, --output", "--format"] {
        assert!(text.contains(flag), "`--help` names `{flag}`: {text}");
    }
}
