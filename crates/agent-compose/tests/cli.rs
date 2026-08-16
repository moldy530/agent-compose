//! `agent-compose validate`, end to end through the real binary.
//!
//! Error UX is a product feature (PRD G3), so the rendered report is pinned
//! **byte for byte** rather than probed for substrings: an improvement to a
//! snippet is a reviewed diff, and a regression is a test failure. The four
//! scenarios below are the four shapes a report takes — a clean run, one
//! diagnostic in one file, one diagnostic spanning two files, and the machine
//! format — plus the three exit codes and a guard against the checks going
//! quadratic.
//!
//! Every run sets `NO_COLOR` and reads the streams through a pipe, so nothing
//! here depends on a terminal; `annotate-snippets`' decor is ASCII either way,
//! which means the only difference a terminal makes is the colour these tests
//! turn off.

use std::path::{Path, PathBuf};
use std::process::Output;
use std::time::{Duration, Instant};

use assert_cmd::Command;

/// The repository root: the two example projects live under it, and the paths a
/// report prints are relative to whatever directory the command ran in.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

fn projects() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects")
}

/// Run `validate` in `directory` and return what it wrote and how it exited.
fn validate(directory: &Path, arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(directory)
        .env("NO_COLOR", "1")
        .arg("validate")
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

/// Both worked projects validate clean through the real command, under the
/// built-in target and — for the one that ships a deploy file — under the named
/// one too. Human output goes to stderr and stdout stays empty.
#[test]
fn both_examples_validate_clean() {
    for (project, target, line) in [
        (
            "examples/review-loop/main.yml",
            "local",
            "`examples/review-loop/main.yml` is valid (target `local`)\n",
        ),
        (
            "examples/triage-fanout/main.yml",
            "local",
            "`examples/triage-fanout/main.yml` is valid (target `local`)\n",
        ),
        (
            "examples/triage-fanout/main.yml",
            "staging",
            "`examples/triage-fanout/main.yml` is valid (target `staging`)\n",
        ),
    ] {
        let output = validate(&repo_root(), &[project, "--target", target]);
        assert_eq!(stderr(&output), line, "{project} --target {target}");
        assert_eq!(stdout(&output), "", "{project} --target {target}");
        assert_eq!(code(&output), 0, "{project} --target {target}");
    }
}

/// One diagnostic, one file: the title carries the stable code, the snippet
/// underlines the span, and the help follows.
#[test]
fn a_parse_error_renders_one_snippet() {
    let output = validate(&projects().join("one-parse-error"), &["main.yml"]);
    assert_eq!(
        stderr(&output),
        "\
error[unknown-key]: unknown key `descriptio` in agent definition `agent.reviewer`
  --> main.yml:16:3
   |
16 |   descriptio: Reviews a draft.
   |   ^^^^^^^^^^
   |
   = help: did you mean `description`?

error: `main.yml` is not valid (target `local`): 1 error
"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 1);
}

/// One diagnostic, two files: the primary span first, then the labelled site in
/// the other file, each in its own snippet with its own path.
#[test]
fn a_cross_file_resolve_error_renders_both_sites() {
    let output = validate(&projects().join("duplicate-across-files"), &["main.yml"]);
    assert_eq!(
        stderr(&output),
        "\
error[duplicate-definition]: `model.m` is defined twice in this composition
  --> models.yml:1:1
   |
 1 | model.m:
   | ^^^^^^^
   |
  ::: main.yml:12:1
   |
12 | model.m:
   | ------- first defined here, in `main.yml`
   |
   = help: a typed address is global across the composition, whichever file declares it: rename one of the two, or drop the file that duplicates the other (grammar 2.2)

error: `main.yml` is not valid (target `local`): 1 error
"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 1);
}

/// `--format json` writes one object on stdout and nothing on stderr. The shape
/// is the `Diagnostic` type itself, with a span in the one string form the IR
/// already uses.
#[test]
fn the_json_report_is_one_object_of_diagnostics() {
    let output = validate(
        &projects().join("duplicate-across-files"),
        &["main.yml", "--format", "json"],
    );
    assert_eq!(
        stdout(&output),
        r#"{
  "diagnostics": [
    {
      "code": "duplicate-definition",
      "help": "a typed address is global across the composition, whichever file declares it: rename one of the two, or drop the file that duplicates the other (grammar 2.2)",
      "labels": [
        {
          "message": "first defined here, in `main.yml`",
          "span": "main.yml:12:1..12:8"
        }
      ],
      "message": "`model.m` is defined twice in this composition",
      "severity": "error",
      "span": "models.yml:1:1..1:8"
    }
  ]
}
"#
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(code(&output), 1);
}

/// A clean run in the machine format is still one object, so a consumer parses
/// one shape whatever the outcome.
#[test]
fn a_clean_json_report_carries_an_empty_array() {
    let output = validate(
        &repo_root(),
        &["examples/review-loop/main.yml", "--format", "json"],
    );
    assert_eq!(
        stdout(&output),
        "{\n  \"diagnostics\": []\n}\n",
        "a clean run still writes the enclosing object"
    );
    assert_eq!(stderr(&output), "");
    assert_eq!(code(&output), 0);
}

/// `2` is the code for "there was nothing to look at", as against `1` for "the
/// composition is wrong". An unreadable entrypoint is the command's own
/// precondition and has no span, so it is a plain message rather than a
/// diagnostic.
#[test]
fn an_unreadable_entrypoint_exits_two() {
    let output = validate(&repo_root(), &["no/such/main.yml"]);
    assert_eq!(
        stderr(&output),
        "error: cannot read `no/such/main.yml`: No such file or directory (os error 2)\n"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 2);

    let output = validate(&repo_root(), &["examples"]);
    assert_eq!(
        stderr(&output),
        "error: `examples` is not a file: `validate` takes a spec entrypoint, conventionally `main.yml`\n"
    );
    assert_eq!(code(&output), 2);
}

/// A usage error is the same class, and clap already exits `2` for one.
#[test]
fn a_usage_error_exits_two() {
    let output = validate(&repo_root(), &["main.yml", "--format", "yaml"]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).contains("invalid value 'yaml'"),
        "clap names the offending value:\n{}",
        stderr(&output)
    );

    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .arg("validate")
        .output()
        .expect("the command runs");
    assert_eq!(code(&output), 2, "a missing path is a usage error");
}

/// A guard against the graph analyses going quadratic in a way nobody notices:
/// `validate` is a millisecond-budget command (PRD 5.12), and both worked
/// projects are well inside a *second* even in a debug build. The bound is
/// deliberately far above the measurement — this test catches an algorithmic
/// regression, not a slow machine — and the minimum of three runs is taken so a
/// scheduling hiccup cannot fail it.
#[test]
fn validating_both_examples_stays_far_inside_a_second() {
    let budget = Duration::from_secs(1);
    for project in [
        "examples/review-loop/main.yml",
        "examples/triage-fanout/main.yml",
    ] {
        let fastest = (0..3)
            .map(|_| {
                let started = Instant::now();
                let output = validate(&repo_root(), &[project]);
                assert_eq!(code(&output), 0, "{project} validates clean");
                started.elapsed()
            })
            .min()
            .expect("three runs");
        assert!(
            fastest < budget,
            "{project} took {fastest:?}, and the budget is {budget:?}"
        );
        println!("{project}: {fastest:?}");
    }
}
