//! The discovery verbs, end to end through the real binary.
//!
//! `schema`, `init`, `explain`, `docs` and `skill` are the surface PRD §7 M2
//! calls progressive discovery: what a coding agent holding nothing but a
//! released binary can ask (resolved q23). Their contract is small and entirely
//! observable from outside — which stream a document goes to, what a name
//! nobody defines does, and which file a write lands in — so it is pinned here
//! rather than in a unit test that could agree with itself about a verb the
//! command line never routes to.
//!
//! Every document these verbs print is embedded, and
//! `tests/discovery_surface_inventory.rs` is what holds the documents to the
//! compiler they describe. This file holds the *command*.

use std::path::{Path, PathBuf};
use std::process::Output;

use assert_cmd::Command;

/// The repository root, which is where the published schema is committed.
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// Run the binary in `directory` with `arguments`.
fn run(directory: &Path, arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(directory)
        .env("NO_COLOR", "1")
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

/// A scratch directory of this test's own, emptied first so a rerun starts
/// where the first run did.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-discovery-cli")
        .join(name);
    let _ = std::fs::remove_dir_all(&directory);
    std::fs::create_dir_all(&directory).expect("can create a scratch directory");
    directory
}

/// `schema` writes the committed document to **stdout**, byte for byte, and
/// exits clean.
///
/// Stdout because the document is the answer — `agent-compose schema >
/// schema.json` is the whole point of the verb, and a byte of prose on that
/// stream would corrupt the file it redirects into.
#[test]
fn schema_writes_the_published_document_to_stdout() {
    let committed = std::fs::read_to_string(repo_root().join("schemas/agent-compose.schema.json"))
        .expect("the published schema is readable");
    let output = run(&repo_root(), &["schema"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stdout(&output), committed);
    assert_eq!(stderr(&output), "");
}

/// The redirect the verb exists for, performed: what lands on disk parses as
/// the schema it claims to be.
#[test]
fn a_redirected_schema_is_a_usable_file() {
    let directory = scratch("schema-redirect");
    let output = run(&directory, &["schema"]);
    let written = directory.join("schema.json");
    std::fs::write(&written, stdout(&output)).expect("can write the redirect");
    let parsed: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&written).expect("readable"))
            .expect("the redirected document is JSON");
    assert_eq!(
        parsed["$schema"], "https://json-schema.org/draft/2020-12/schema",
        "the redirect carries the dialect an editor reads"
    );
}

/// The loop's first two steps, back to back, through the real binary: `init`
/// then `validate`.
///
/// This is the one property the scaffold has, and it is worth running through
/// the command rather than over the constant — an agent's very first
/// `validate` must not be a report about this compiler's own file.
#[test]
fn a_scaffolded_project_validates_clean() {
    let directory = scratch("init-then-validate");
    let initialized = run(&directory, &["init"]);
    assert_eq!(code(&initialized), 0, "stderr: {}", stderr(&initialized));
    assert_eq!(stdout(&initialized), "", "the file is the answer");
    assert!(
        stderr(&initialized).contains("agent-compose validate"),
        "the next step is named: {}",
        stderr(&initialized)
    );

    let validated = run(&directory, &["validate", "main.yml"]);
    assert_eq!(
        code(&validated),
        0,
        "the scaffold validates: {}",
        stderr(&validated)
    );
    assert_eq!(stderr(&validated), "`main.yml` is valid (target `local`)\n");
}

/// A named directory that does not exist is created; an argument is not a
/// second way to spell the current directory.
#[test]
fn init_creates_the_directory_it_is_named() {
    let directory = scratch("init-names-a-child");
    let output = run(&directory, &["init", "summarizer"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    let scaffold = directory.join("summarizer/main.yml");
    assert!(scaffold.is_file(), "the scaffold landed under the name");
    assert_eq!(
        std::fs::read_to_string(&scaffold).expect("readable"),
        compose_core::docs::SCAFFOLD
    );
}

/// A directory holding anything at all is refused, with the exit code that
/// means *the answer is no* rather than the one that means *the command could
/// not run*, and with the remedy in the message (PRD G3).
#[test]
fn init_refuses_a_directory_that_holds_anything() {
    let directory = scratch("init-refuses-occupied");
    std::fs::write(directory.join("notes.md"), "mine\n").expect("can write");
    let output = run(&directory, &["init"]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    let reported = stderr(&output);
    assert!(
        reported.contains("`notes.md`"),
        "names what it found: {reported}"
    );
    assert!(
        reported.contains("agent-compose init <name>"),
        "names the remedy: {reported}"
    );
    assert!(
        !directory.join("main.yml").exists(),
        "a refusal writes nothing"
    );
    assert_eq!(
        std::fs::read_to_string(directory.join("notes.md")).expect("readable"),
        "mine\n",
        "and touches nothing"
    );
}

/// PRD §7 M2's exit criterion, as far as one test can carry it: a directory
/// with **nothing in it**, and every teaching verb still answers.
///
/// The point is what is *absent* — no `docs/grammar.md`, no `schemas/`, no
/// checkout of this repository anywhere the command can see. Everything these
/// verbs print is embedded, and a regression to reading a file beside the
/// binary would pass every other test in this file and fail here.
#[test]
fn every_teaching_verb_answers_from_an_empty_directory() {
    let directory = scratch("no-checkout");
    for arguments in [
        vec!["docs"],
        vec!["docs", "getting-started"],
        vec!["explain", "unbounded-cycle"],
        vec!["schema"],
        vec!["skill"],
    ] {
        let output = run(&directory, &arguments);
        assert_eq!(code(&output), 0, "`{arguments:?}` answers");
        assert!(
            stdout(&output).len() > 200,
            "`{arguments:?}` answers with a document"
        );
        assert_eq!(stderr(&output), "", "`{arguments:?}` says nothing else");
    }
    assert!(
        std::fs::read_dir(&directory)
            .expect("readable")
            .next()
            .is_none(),
        "and none of them wrote anything"
    );
}

/// `docs` with no topic prints the index, and the index is a *starting point*:
/// it names every topic and then says what to do first.
#[test]
fn docs_prints_the_index_on_stdout() {
    let output = run(&repo_root(), &["docs"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "");
    let index = stdout(&output);
    for topic in compose_core::docs::TOPICS {
        assert!(
            index.contains(topic.name),
            "the index omits `{}`",
            topic.name
        );
    }
    assert!(
        index.contains("agent-compose validate"),
        "the index closes with the loop: {index}"
    );
}

/// `docs <topic>` prints that topic's document, byte for byte.
#[test]
fn docs_prints_one_topic_verbatim() {
    let output = run(&repo_root(), &["docs", "routing"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "");
    assert_eq!(
        stdout(&output),
        compose_core::docs::topic("routing")
            .expect("the curriculum has `routing`")
            .body
    );
}

/// A topic nobody defines: exit `2`, the vocabulary, and a suggestion where the
/// spelling is close (PRD G3).
#[test]
fn docs_refuses_an_unknown_topic_with_the_list_and_a_suggestion() {
    let output = run(&repo_root(), &["docs", "routng"]);
    assert_eq!(code(&output), 2);
    assert_eq!(stdout(&output), "", "nothing lands on the answer stream");
    let reported = stderr(&output);
    assert!(reported.contains("did you mean `routing`?"), "{reported}");
    assert!(reported.contains("`getting-started`"), "{reported}");
}

/// `explain <code>` prints the explanation for the code a report names.
#[test]
fn explain_prints_the_document_for_a_code() {
    let output = run(&repo_root(), &["explain", "non-exhaustive"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "");
    assert_eq!(
        stdout(&output),
        compose_core::docs::explanation(compose_core::DiagnosticCode::NonExhaustive)
    );
}

/// Every code the compiler can report answers from the command line, which is
/// what makes the line `validate` prints safe to print unconditionally.
#[test]
fn every_diagnostic_code_has_an_answer_from_the_command_line() {
    for diagnostic in compose_core::DiagnosticCode::ALL {
        let name = diagnostic.as_str();
        let output = run(&repo_root(), &["explain", name]);
        assert_eq!(code(&output), 0, "`explain {name}` answers");
        assert!(
            stdout(&output).starts_with(&format!("# {name}")),
            "`explain {name}` prints its own document"
        );
    }
}

/// A code nobody defines: exit `2` and a suggestion — but **not** the whole
/// vocabulary, which is dozens of names the reader did not ask for.
#[test]
fn explain_refuses_an_unknown_code_with_a_suggestion_and_no_wall_of_codes() {
    let output = run(&repo_root(), &["explain", "unknwon-key"]);
    assert_eq!(code(&output), 2);
    assert_eq!(stdout(&output), "");
    let reported = stderr(&output);
    assert!(
        reported.contains("did you mean `unknown-key`?"),
        "{reported}"
    );
    assert!(
        !reported.contains("`unbalanced-convergence`"),
        "the refusal does not list every code: {reported}"
    );
}

/// `validate` ends its human report with the line that turns a code into a
/// command — **once**, however many diagnostics it reported.
#[test]
fn validate_points_at_explain_once_per_run() {
    let directory = scratch("validate-hint");
    std::fs::write(
        directory.join("main.yml"),
        "version: \"0.1\"\nstate:\n  output: { type: string }\n  item: { type: string }\n",
    )
    .expect("can write");
    let output = run(&directory, &["validate", "main.yml"]);
    assert_eq!(code(&output), 1);
    let reported = stderr(&output);
    assert!(
        reported.matches("reserved-name").count() >= 2,
        "the spec reports more than one diagnostic: {reported}"
    );
    assert_eq!(
        reported.matches("agent-compose explain <code>").count(),
        1,
        "the hint is printed once per run, not once per diagnostic: {reported}"
    );
    assert!(
        reported.ends_with("for more about a code, run: agent-compose explain <code>\n"),
        "the hint is the last line: {reported}"
    );
}

/// The verbs that *launch* end their human report with the same line.
///
/// `build`, `run` and `serve` validate before they emit anything, so a reader
/// meets a diagnostic through them as readily as through `validate` — and
/// having not typed `validate`, is further from the verb that would explain it,
/// not closer. Pinned on `build` and `run` because they are the two a reader
/// reaches first; all three print the report from one place.
#[test]
fn the_launching_verbs_point_at_explain_too() {
    let directory = scratch("launch-hint");
    std::fs::write(
        directory.join("main.yml"),
        "version: \"0.1\"\nstate:\n  output: { type: string }\n",
    )
    .expect("can write");

    for arguments in [
        ["build", "main.yml"].as_slice(),
        ["run", "main.yml", "flow.f"].as_slice(),
    ] {
        let output = run(&directory, arguments);
        let reported = stderr(&output);
        assert_eq!(code(&output), 1, "{reported}");
        assert!(
            reported.contains("reserved-name"),
            "`{}` reported the diagnostic: {reported}",
            arguments.join(" ")
        );
        assert!(
            reported.ends_with("for more about a code, run: agent-compose explain <code>\n"),
            "`{}` ends with the hint: {reported}",
            arguments.join(" ")
        );
    }
}

/// A `build` that reported nothing says nothing about `explain` either.
///
/// The guard is one `is_empty` shared by every caller, and this is the half of
/// it the launching verbs exercise: a clean `build` still writes a verdict — and
/// a list of what it wrote — so a hint appended unconditionally would show up
/// under a success.
#[test]
fn a_clean_build_prints_no_hint() {
    let directory = scratch("build-clean-hint");
    run(&directory, &["init"]);
    let output = run(&directory, &["build", "main.yml"]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        !stderr(&output).contains("explain"),
        "a clean build carries no hint: {}",
        stderr(&output)
    );
}

/// A clean run says nothing about `explain`: there is no code to explain.
#[test]
fn a_clean_validate_prints_no_hint() {
    let directory = scratch("validate-clean-hint");
    run(&directory, &["init"]);
    let output = run(&directory, &["validate", "main.yml"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "`main.yml` is valid (target `local`)\n");
}

/// The machine format is untouched: a reader holding the code as data does not
/// need to be told a verb exists.
#[test]
fn the_json_report_carries_no_hint() {
    let directory = scratch("validate-json-hint");
    std::fs::write(
        directory.join("main.yml"),
        "version: \"0.1\"\nstate:\n  output: { type: string }\n",
    )
    .expect("can write");
    let output = run(&directory, &["validate", "main.yml", "--format", "json"]);
    assert_eq!(code(&output), 1);
    assert!(
        !stdout(&output).contains("explain"),
        "the JSON report is unchanged: {}",
        stdout(&output)
    );
}

/// Bare `skill` prints the agent-agnostic document on stdout.
#[test]
fn skill_prints_the_bare_document() {
    let output = run(&repo_root(), &["skill"]);
    assert_eq!(code(&output), 0);
    assert_eq!(stderr(&output), "");
    assert_eq!(stdout(&output), compose_core::docs::SKILL);
    assert!(
        !stdout(&output).starts_with("---"),
        "no agent's frontmatter on the bare document"
    );
}

/// `--agent claude` writes the project-level path, with the frontmatter its
/// loader reads.
#[test]
fn skill_installs_for_claude_under_the_current_directory() {
    let directory = scratch("skill-claude");
    let output = run(&directory, &["skill", "--agent", "claude"]);
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert_eq!(stdout(&output), "", "the file is the answer");
    assert_eq!(
        stderr(&output),
        "wrote `.claude/skills/agent-compose/SKILL.md`\n",
        "the one line it prints is the path as a reader would type it, with no `./`"
    );
    let path = directory.join(".claude/skills/agent-compose/SKILL.md");
    assert!(path.is_file(), "the skill landed at the conventional path");
    assert_eq!(
        std::fs::read_to_string(&path).expect("readable"),
        compose_core::docs::skill::claude()
    );
}

/// A re-install of the same document is a no-op that succeeds: keeping a skill
/// in step must not be a command that fails whenever it is already in step.
#[test]
fn reinstalling_the_same_skill_is_a_clean_no_op() {
    let directory = scratch("skill-reinstall");
    assert_eq!(code(&run(&directory, &["skill", "--agent", "claude"])), 0);
    let output = run(&directory, &["skill", "--agent", "claude"]);
    assert_eq!(code(&output), 0);
    assert!(
        stderr(&output).contains("already this skill"),
        "it says nothing needed doing: {}",
        stderr(&output)
    );
}

/// A file that differs is refused, with the exit code that means *the answer is
/// no*, and is left exactly as it was.
#[test]
fn skill_refuses_to_replace_a_document_it_did_not_write() {
    let directory = scratch("skill-occupied");
    let path = directory.join(".claude/skills/agent-compose/SKILL.md");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("can create");
    std::fs::write(&path, "my own notes\n").expect("can write");

    let output = run(&directory, &["skill", "--agent", "claude"]);
    assert_eq!(code(&output), 1);
    assert_eq!(stdout(&output), "");
    assert!(
        stderr(&output).contains("SKILL.md"),
        "the refusal names the path: {}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read_to_string(&path).expect("readable"),
        "my own notes\n",
        "the refusal touched nothing"
    );
}

/// A file that is not text at all is still a file that differs, not a file the
/// command could not read.
///
/// The distinction is the exit code, and the exit code is what a supervisor
/// branches on: `2` says the command could not run and invites a retry, `1`
/// says the answer is no and sends the user to look at the path. Comparing
/// decoded text would put an existing `SKILL.md` of arbitrary bytes — a
/// truncated download, somebody's binary note-taking format — under `2`
/// forever, since no retry makes those bytes decode.
#[test]
fn skill_refuses_a_file_of_bytes_that_are_not_this_document() {
    let directory = scratch("skill-not-text");
    let path = directory.join(".claude/skills/agent-compose/SKILL.md");
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("can create");
    let held: &[u8] = &[0xff, 0xfe, 0x00, b'n', b'o', b't', 0x80];
    std::fs::write(&path, held).expect("can write");

    let output = run(&directory, &["skill", "--agent", "claude"]);
    assert_eq!(
        code(&output),
        1,
        "the answer is no, not unreadable: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("SKILL.md") && stderr(&output).contains("different document"),
        "the refusal names the path and the reason: {}",
        stderr(&output)
    );
    assert_eq!(
        std::fs::read(&path).expect("readable"),
        held,
        "the refusal touched nothing"
    );
}

/// `--global` writes under `$HOME` instead of the working directory.
#[test]
fn skill_installs_globally_under_home() {
    let directory = scratch("skill-global");
    let home = directory.join("home");
    std::fs::create_dir_all(&home).expect("can create");
    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&directory)
        .env("NO_COLOR", "1")
        .env("HOME", &home)
        .args(["skill", "--agent", "claude", "--global"])
        .output()
        .expect("the command runs");
    assert_eq!(code(&output), 0, "stderr: {}", stderr(&output));
    assert!(
        home.join(".claude/skills/agent-compose/SKILL.md").is_file(),
        "the global install lands under HOME"
    );
    assert!(
        !directory.join(".claude").exists(),
        "and not under the working directory"
    );
}

/// The Codex posture **prints**: `AGENTS.md` is the user's file, and this
/// command does not edit it.
#[test]
fn skill_for_codex_prints_and_writes_nothing() {
    let directory = scratch("skill-codex");
    let output = run(&directory, &["skill", "--agent", "codex"]);
    assert_eq!(code(&output), 0);
    let printed = stdout(&output);
    assert!(printed.contains("AGENTS.md"), "it says where to put it");
    assert!(printed.ends_with(compose_core::docs::SKILL));
    let held: Vec<_> = std::fs::read_dir(&directory)
        .expect("readable")
        .filter_map(Result::ok)
        .collect();
    assert!(held.is_empty(), "the codex posture wrote nothing");
}

/// An agent nobody has a posture for: exit `2`, listing the ones there are.
#[test]
fn skill_refuses_an_unknown_agent_with_the_list() {
    let output = run(&repo_root(), &["skill", "--agent", "nano"]);
    assert_eq!(code(&output), 2);
    let reported = stderr(&output);
    assert!(reported.contains("`claude`"), "{reported}");
    assert!(reported.contains("`codex`"), "{reported}");
}

/// `--global` beside a posture that writes nowhere is refused rather than
/// ignored: a flag silently dropped would leave a user believing they had
/// installed something under `$HOME`.
#[test]
fn global_needs_an_agent_that_writes() {
    let bare = run(&repo_root(), &["skill", "--global"]);
    assert_eq!(code(&bare), 2);
    assert!(stderr(&bare).contains("`--global` needs an `--agent`"));

    let codex = run(&repo_root(), &["skill", "--agent", "codex", "--global"]);
    assert_eq!(code(&codex), 2);
    assert!(stderr(&codex).contains("not meaningful"));
}
