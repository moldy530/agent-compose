//! `agent-compose validate`, end to end through the real binary.
//!
//! Error UX is a product feature (PRD G3), so the rendered report is pinned
//! **byte for byte** rather than probed for substrings: an improvement to a
//! snippet is a reviewed diff, and a regression is a test failure. The scenarios
//! below are the shapes a report takes — a clean run, one diagnostic in one
//! file, one diagnostic drawing three sites of one file, one diagnostic spanning
//! two files, the same report from *outside* the project, and the machine format
//! — plus the three exit codes and a guard against the checks going quadratic.
//!
//! Every run sets `NO_COLOR` and reads the streams through a pipe, so nothing
//! here depends on a terminal; `annotate-snippets`' decor is ASCII either way,
//! which means the only difference a terminal makes is the colour these tests
//! turn off. That colour is the one thing they therefore cannot see, so
//! `report`'s own unit tests hold it: both verdicts styled or neither, and the
//! styled render equal to the plain one once the escapes come back out.
//!
//! One scenario reads its stream *and stops* rather than draining it, because
//! `| head` and `| less` are how this command is used and a closed pipe is the
//! one way a run ends outside the exit-code table (see `main.rs`).

use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};
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
for more about a code, run: agent-compose explain <code>
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
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 1);
}

/// One diagnostic, three sites in **one** file: the two secondary labels fold
/// into the primary snippet rather than opening snippets of their own, and the
/// lines between the node and its edges are elided to a `...`.
///
/// A graph check is where that shape comes from — the mistake is a relation
/// between declarations that sit apart in the file, so the report has to draw
/// all three at once — and `unbalanced-convergence` is the ordinary case: a
/// convergence, and the two edges of the fork that reach it at different depths
/// (grammar 7.6.2). The project is grammar 7.6.2's own worked diamond with the
/// `trivial` shortcut added, so what a reader compares this against is written
/// down. Nothing else pins a check diagnostic through the renderer:
/// `compose-core`'s corpus asserts on the `Diagnostic` values, which is the
/// wrong altitude to catch a label that stopped folding or a gap that stopped
/// eliding, and error UX is a product feature (PRD G3).
#[test]
fn a_graph_check_renders_its_two_secondary_labels_in_one_snippet() {
    let output = validate(
        &projects().join("one-unbalanced-convergence"),
        &["main.yml"],
    );
    assert_eq!(
        stderr(&output),
        "\
error[unbalanced-convergence]: node `merge` of `flow.diamond` is reached from the fork `plan` at two different depths
  --> main.yml:24:5
   |
24 |     merge: { agent: agent.writer, input: \"'the draft'\" }
   |     ^^^^^
...
27 |     - { from: plan, to: merge, when: \"plan.output.trivial\" }
   |       ------------------------------------------------------ this edge reaches it in 1 step
28 |     - { from: plan, to: draft, when: \"plan.output.need_draft\" }
   |       --------------------------------------------------------- this one in 2 steps
   |
   = help: a convergence reached in two different steps runs twice, once per arrival: route the short branch through the same depth, or make the two edges exclusive — `else: true` on one, or guards grammar 7.6.1 can prove disjoint (grammar 7.6.2, Decisions D69, D112)

error: `main.yml` is not valid (target `local`): 1 error
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 1);
}

/// The other shape a graph check takes: a `map` node as one of the two sites.
///
/// A `map` node has no output of its own, so what it writes is what its
/// dispatched instances write — and a write made by *name* has no site of its
/// own to point at, which is where an anchor goes wrong. The block a `map:` key
/// opens ends where the next node's key begins, so anchoring the label there
/// draws "the other write is here" across a node the diagnostic has nothing to
/// say about; the node's `id` is the anchor, and it is the name the message
/// itself reads. Only the rendered report shows the difference, which is why it
/// is pinned here rather than in `compose-core`'s corpus — and that corpus
/// cannot host this case anyway: a `map` that races a node on an unreduced
/// channel breaks grammar 8.6 rule 5 as well, and a fixture there pins exactly
/// one diagnostic per code.
///
/// The second diagnostic's own anchor *is* the `map:` block, grammar 8.6 rule 5
/// being about the whole dispatch — and the underline stops at the last line the
/// block actually declares, four lines drawn over four lines of `map:`. A
/// collection's span ends at its own text rather than at the token that follows
/// it (`yaml::Loader::collection_span`), which is what makes the two anchors
/// comparable here: one names a node, the other draws a block, and neither
/// reaches into anything it is not about.
#[test]
fn a_maps_write_is_labelled_on_the_node_that_makes_it() {
    let output = validate(&projects().join("one-concurrent-map-write"), &["main.yml"]);
    assert_eq!(
        stderr(&output),
        "\
error[unreduced-write]: nodes `review` and `work` of `flow.fanout` run concurrently and both write the unreduced channel `verdict`
  --> main.yml:42:5
   |
13 |   verdict: { type: string, default: \"\" }
   |   -------------------------------------- the channel is declared here
...
41 |     review: { agent: agent.reviewer, input: \"'the plan'\" }
   |     ------ the other write is here
42 |     work:
   |     ^^^^
   |
   = help: two edges of one fork that are not provably exclusive can both fire, so the branches they start are concurrent: the channel they both write needs a declared `reduce:` policy — `append`, `merge`, or an explicit `last_wins` (grammar 7.6.1, 10.2, Decision D32)

error[unreduced-write]: the `map` of node `work` writes the unreduced channel `verdict`
  --> main.yml:44:9
   |
13 |     verdict: { type: string, default: \"\" }
   |     -------------------------------------- the channel is declared here
...
44 | /         over: \"state.tasks\"
45 | |         node: agent.worker
46 | |         max_concurrency: 2
47 | |         input: { text: \"item\" }
   | |_______________________________^
   |
   = help: dispatched instances are concurrent writers, so the channel they write needs a declared `reduce:` policy — `append`, `merge`, or an explicit `last_wins` (grammar 8.6 rule 5, 10.2)

error: `main.yml` is not valid (target `local`): 2 errors
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(stdout(&output), "");
    assert_eq!(code(&output), 1);
}

/// The same two sites, reported from *outside* the project: every path in the
/// human report is the one the reader would have to type from where they ran the
/// command, entrypoint and imported file alike. A span names its file relative
/// to the project root (grammar 1.4) and the machine format keeps it that way —
/// see below — but a rendered location that cannot be opened from the shell that
/// printed it is a broken feedback loop, and the coding agent PRD G3 writes these
/// for is exactly the reader who cannot guess the missing prefix.
#[test]
fn a_report_from_outside_the_project_points_at_paths_that_open() {
    let project = "crates/agent-compose/tests/projects/duplicate-across-files";
    let output = validate(&repo_root(), &[&format!("{project}/main.yml")]);
    assert_eq!(
        stderr(&output),
        "\
error[duplicate-definition]: `model.m` is defined twice in this composition
  --> crates/agent-compose/tests/projects/duplicate-across-files/models.yml:1:1
   |
 1 | model.m:
   | ^^^^^^^
   |
  ::: crates/agent-compose/tests/projects/duplicate-across-files/main.yml:12:1
   |
12 | model.m:
   | ------- first defined here, in `main.yml`
   |
   = help: a typed address is global across the composition, whichever file declares it: rename one of the two, or drop the file that duplicates the other (grammar 2.2)

error: `crates/agent-compose/tests/projects/duplicate-across-files/main.yml` is not valid (target `local`): 1 error
for more about a code, run: agent-compose explain <code>
"
    );
    assert_eq!(code(&output), 1);

    // Every location the report drew, opened from where the command ran.
    let mut opened = 0;
    for line in stderr(&output).lines() {
        let line = line.trim_start();
        let Some(rest) = line
            .strip_prefix("--> ")
            .or_else(|| line.strip_prefix("::: "))
        else {
            continue;
        };
        let (path, _) = rest.split_once(".yml:").expect("a location names a file");
        let path = repo_root().join(format!("{path}.yml"));
        assert!(path.is_file(), "the report points at `{}`", path.display());
        opened += 1;
    }
    assert_eq!(opened, 2, "both sites were checked");

    // The machine format is the other reader, and its spans stay as the IR
    // writes them: relative to the project root, wherever the command ran.
    let output = validate(
        &repo_root(),
        &[&format!("{project}/main.yml"), "--format", "json"],
    );
    assert!(
        stdout(&output).contains("\"span\": \"models.yml:1:1..1:8\"")
            && stdout(&output).contains("\"span\": \"main.yml:12:1..12:8\""),
        "the JSON spans are project-root-relative:\n{}",
        stdout(&output)
    );
}

/// `--format json` writes one object on stdout and nothing on stderr. The shape
/// is the `Diagnostic` type itself, with a span in the one string form the IR
/// already uses — **project-root-relative**, matching the IR and unaffected by
/// where the command ran, which is what a consumer joining spans back onto a
/// checkout needs.
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

    // The same precondition, named by the verb the caller typed. `run` and
    // `serve` share the code path that checks it, and a `serve` told that
    // "`run` takes a spec entrypoint" would name a command nobody ran.
    for (command, rest) in [
        // `run` takes a flow beside the entrypoint; nothing gets as far as
        // reading it, but clap still requires it to be there.
        ("run", vec!["flow.nothing"]),
        ("serve", vec![]),
        ("build", vec![]),
    ] {
        let output = Command::cargo_bin("agent-compose")
            .expect("the binary under test is built")
            .current_dir(repo_root())
            .env("NO_COLOR", "1")
            .arg(command)
            .arg("examples")
            .args(&rest)
            .output()
            .expect("the command runs");
        assert_eq!(
            stderr(&output),
            format!(
                "error: `examples` is not a file: `{command}` takes a spec entrypoint, conventionally `main.yml`\n"
            )
        );
        assert_eq!(code(&output), 2);
    }
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

/// A project whose report is far larger than a pipe can hold: `definitions`
/// agent definitions, each with one misspelled key, for one `unknown-key`
/// diagnostic apiece.
///
/// The size is the point. A pipe holds 64 KiB on Linux, and a command whose
/// whole report fits inside it never writes into a closed one — so a report that
/// does not outgrow the buffer cannot show the failure below, whatever the
/// reader does. 400 definitions are ~190 KB of JSON and ~180 KB of rendered
/// snippets, both several times the buffer.
fn noisy_project(dir: &Path, definitions: usize) {
    let mut text = String::from(
        "version: \"0.1\"\nprovider.p:\n  kind: anthropic\n  api_key: ${K}\nmodel.m:\n  provider: provider.p\n  id: some-model\n",
    );
    for at in 0..definitions {
        text.push_str(&format!(
            "agent.a{at}:\n  model: model.m\n  prompt: Do it.\n  descriptio: Reviews a draft.\n"
        ));
    }
    fs::write(dir.join("main.yml"), text).expect("can write the entrypoint");
}

/// Run `validate`, read one byte of the stream the report goes to, close it, and
/// wait for the command to exit: the shape of `agent-compose validate … | head`.
///
/// The other stream is captured whole, so what the run has to say about the
/// closed one is still readable. The report outgrows the pipe's buffer, so the
/// command is certainly still writing when the read end goes.
fn report_into_a_closed_pipe(dir: &Path, format: &str) -> Output {
    let mut child = std::process::Command::new(env!("CARGO_BIN_EXE_agent-compose"))
        .current_dir(dir)
        .env("NO_COLOR", "1")
        .args(["validate", "main.yml", "--format", format])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("the command runs");
    let mut reading: Box<dyn Read> = if format == "json" {
        Box::new(child.stdout.take().expect("stdout is a pipe"))
    } else {
        Box::new(child.stderr.take().expect("stderr is a pipe"))
    };
    let mut first = [0u8; 1];
    reading
        .read_exact(&mut first)
        .expect("the report starts before the reader leaves");
    drop(reading);
    child.wait_with_output().expect("the command exits")
}

/// A reader that stops reading is not a failure of the command.
///
/// `| head -1`, or `| less` and then `q`: the pipe closes while the report is
/// still being written, and the write that follows fails. Answering that by
/// panicking exits `101` — a code `main.rs`'s table gives no meaning, and one no
/// consumer of `validate` can act on — and prints a Rust backtrace on the stream
/// `--format json` promises to leave empty. The verdict is what the run means,
/// and it survives the reader leaving: both formats exit `1` here, and the JSON
/// run still says nothing at all on stderr.
///
/// This is reachable only through a report bigger than the pipe's buffer, which
/// is why the project is generated: a composition grows into this failure
/// without anything about it changing.
#[test]
fn a_reader_that_stops_reading_still_gets_the_verdict() {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("closed-pipe-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    noisy_project(&dir, 400);

    let output = report_into_a_closed_pipe(&dir, "json");
    assert_eq!(
        code(&output),
        1,
        "a closed pipe exits on the verdict, not on a panic; stderr was:\n{}",
        stderr(&output)
    );
    assert_eq!(
        stderr(&output),
        "",
        "`--format json` says nothing on stderr, a closed stdout included"
    );

    let output = report_into_a_closed_pipe(&dir, "human");
    assert_eq!(code(&output), 1, "the human report, read and abandoned");
    assert_eq!(stdout(&output), "", "the human format writes no stdout");
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
