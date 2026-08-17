//! `agent-compose build` and `build --check`, end to end through the real
//! binary.
//!
//! The emitter itself is tested in `compose-core` — its unit tests own the
//! mapping and the golden corpus owns the bytes. What this file owns is the
//! **command**: which directory it writes to, what it refuses to do, what it
//! says, and which of the three exit codes it uses (see `main.rs`).
//!
//! Every run sets `NO_COLOR` and reads its streams through a pipe, like
//! `tests/cli.rs`, so nothing here depends on a terminal.

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

/// A directory the test writes into, removed when the process ends.
fn scratch(purpose: &str) -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let path = std::env::temp_dir().join(format!(
        "agent-compose-build-cli-{purpose}-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&path);
    fs::create_dir_all(&path).expect("a scratch directory");
    path
}

fn build(arguments: &[&str]) -> Output {
    Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(repo_root())
        .env("NO_COLOR", "1")
        .arg("build")
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

/// Every file under `root`, sorted, as `/`-separated relative paths.
fn files_under(root: &Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(directory) = queue.pop() {
        let Ok(entries) = fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                queue.push(path);
                continue;
            }
            found.push(
                path.strip_prefix(root)
                    .expect("the walk started at the root")
                    .components()
                    .map(|component| component.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
    found.sort();
    found
}

/// The documented layout, written where `--out` says.
#[test]
fn build_writes_the_project_layout_where_out_points() {
    let out = scratch("layout");
    let output = build(&[
        "examples/review-loop/main.yml",
        "--out",
        out.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stdout(&output), "", "human output goes to stderr");
    assert!(
        stderr(&output).contains("wrote 9 files"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        files_under(&out),
        [
            ".gitignore",
            "README.md",
            "package.json",
            "src/env.ts",
            "src/graph.ts",
            "src/index.ts",
            "src/schemas.ts",
            "src/state.ts",
            "tsconfig.json",
        ]
    );
}

/// `build` reads no environment: every `${ENV}` the composition references is
/// unset and it emits anyway, moving the check into the artifact.
///
/// **This pins a reading of PRD 5.9 that the PRD does not yet settle.** Its own
/// bullet lists `build` among the commands that "check presence and fail fast
/// naming the missing variable"; PRD §7 M1 asks instead for "env-ref presence
/// checks at process start", which is what the emitted `src/index.ts` does
/// (`compose-core`'s `tests/generated_code_gates.rs` runs it). This branch
/// implements the second and argues why in `compose_core::codegen::env` — a
/// build whose success depended on the building machine's environment could not
/// be reproduced on the deploying one. The two sentences still have to be
/// reconciled in the PRD; until they are, the behaviour is written down here
/// rather than left to be inferred from an absence of tests, and whichever way
/// the question resolves, this test is what has to change.
#[test]
fn build_emits_with_every_environment_reference_unset() {
    let out = scratch("sealed");
    let entrypoint = repo_root().join("examples/review-loop/main.yml");
    let resolution = compose_core::resolve_with_target(&entrypoint, compose_core::DEFAULT_TARGET);
    let ir = resolution.ir.expect("the worked example resolves");
    let references = compose_core::codegen::env::References::of(&ir);
    let names: Vec<&str> = references.names().collect();
    assert!(
        !names.is_empty(),
        "the composition references no variable, so nothing is being sealed off"
    );

    let mut command = Command::cargo_bin("agent-compose").expect("the binary under test is built");
    command
        .current_dir(repo_root())
        .env("NO_COLOR", "1")
        .args(["build", "examples/review-loop/main.yml", "--out"])
        .arg(&out);
    for name in &names {
        command.env_remove(name);
    }
    let output = command.output().expect("the command runs");
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("wrote 9 files"),
        "{}",
        stderr(&output)
    );

    // …and the check is in the emitted project rather than skipped: every name
    // reaches `src/env.ts`, which `src/index.ts` calls at module scope.
    let env_module = fs::read_to_string(out.join("src/env.ts")).expect("`src/env.ts` was written");
    for name in &names {
        assert!(
            env_module.contains(name),
            "`{name}` is referenced by the composition and not by `src/env.ts`:\n{env_module}"
        );
    }
}

/// A build whose output already matches checks clean, and a `--check` against an
/// empty directory does not.
#[test]
fn check_answers_whether_the_directory_still_matches_the_spec() {
    let out = scratch("check");
    let path = out.to_str().expect("a UTF-8 scratch path");

    let empty = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&empty), 1);
    assert!(
        stderr(&empty).contains("`src/state.ts` is not in the output directory"),
        "{}",
        stderr(&empty)
    );
    assert!(
        stderr(&empty).contains("does not match `examples/review-loop/main.yml`"),
        "{}",
        stderr(&empty)
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "`--check` writes nothing"
    );

    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    let clean = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&clean), 0, "{}", stderr(&clean));
    assert!(
        stderr(&clean).contains("is up to date"),
        "{}",
        stderr(&clean)
    );
}

/// PRD §8's risk, and the mitigation this command is: a hand-edited generated
/// file is drift, and CI learns which one.
#[test]
fn a_hand_edited_generated_file_is_drift() {
    let out = scratch("edited");
    let path = out.to_str().expect("a UTF-8 scratch path");
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    fs::write(out.join("src/state.ts"), "// mine now\n").expect("the file is writable");

    let drifted = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&drifted), 1);
    assert!(
        stderr(&drifted).contains("`src/state.ts` differs from what the spec produces"),
        "{}",
        stderr(&drifted)
    );
    assert!(
        stderr(&drifted).contains("run `agent-compose build` to regenerate"),
        "{}",
        stderr(&drifted)
    );

    // …and regenerating settles it.
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    assert_eq!(
        code(&build(&[
            "examples/review-loop/main.yml",
            "--out",
            path,
            "--check"
        ])),
        0
    );
}

/// `src/` is the compiler's directory; everything else in the output directory
/// is the user's (see `build.rs`).
#[test]
fn a_rebuild_prunes_src_and_leaves_the_rest_alone() {
    let out = scratch("prune");
    let path = out.to_str().expect("a UTF-8 scratch path");
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    fs::create_dir_all(out.join("src/nodes")).expect("writable");
    // A module an older compiler release wrote: it carries the header, which is
    // what makes it this compiler's to remove.
    fs::write(
        out.join("src/nodes/old.ts"),
        "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
    )
    .expect("writable");
    fs::create_dir_all(out.join("node_modules/zod")).expect("writable");
    fs::write(out.join("node_modules/zod/index.js"), "//\n").expect("writable");
    fs::write(out.join(".env"), "SEARCH_HOST=example.test\n").expect("writable");

    let drifted = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&drifted), 1);
    assert!(
        stderr(&drifted).contains("`src/nodes/old.ts` is under `src/` and is not generated"),
        "{}",
        stderr(&drifted)
    );

    let rebuilt = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&rebuilt), 0);
    assert!(
        stderr(&rebuilt)
            .contains("removed 1 generated file it no longer emits: `src/nodes/old.ts`"),
        "the removal is reported rather than silent: {}",
        stderr(&rebuilt)
    );
    assert!(!out.join("src/nodes").exists(), "the stale module is gone");
    assert!(
        out.join("node_modules/zod/index.js").is_file(),
        "an install is not the compiler's to remove"
    );
    assert!(out.join(".env").is_file(), "neither is a `.env`");
}

/// A `src/` full of hand-written TypeScript stops the build instead of being
/// emptied.
///
/// `--out` can name any directory — an existing Node project, an ejected copy
/// being re-synced, `.` — and the compiler's claim on `src/` is a claim about a
/// directory it made. The evidence is the generated-file header, so a `src/`
/// with none is refused rather than pruned.
#[test]
fn a_src_directory_the_compiler_did_not_write_is_refused() {
    let out = scratch("not-ours");
    let path = out.to_str().expect("a UTF-8 scratch path");
    fs::create_dir_all(out.join("src/deep")).expect("writable");
    fs::write(out.join("src/deep/mine.ts"), "export const mine = 1;\n").expect("writable");
    fs::write(out.join("README-mine.md"), "mine\n").expect("writable");

    let refused = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("`src/deep/mine.ts`"),
        "the refusal names the file it would have deleted: {}",
        stderr(&refused)
    );
    assert_eq!(
        fs::read_to_string(out.join("src/deep/mine.ts")).expect("readable"),
        "export const mine = 1;\n"
    );
    assert!(
        !out.join("src/schemas.ts").exists(),
        "the refusal is total: nothing was written either"
    );
    assert!(out.join("README-mine.md").is_file());
}

/// The target selects the deploy layer, and the emitted project says which one
/// it is (grammar 14, PRD 5.8's per-target invariant).
#[test]
fn the_target_reaches_the_emitted_project() {
    let out = scratch("target");
    let path = out.to_str().expect("a UTF-8 scratch path");
    let output = build(&[
        "examples/triage-fanout/main.yml",
        "--target",
        "staging",
        "--out",
        path,
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        stderr(&output).contains("(target `staging`)"),
        "{}",
        stderr(&output)
    );

    let env = fs::read_to_string(out.join("src/env.ts")).expect("the env module is readable");
    assert!(
        env.contains("CHROMA_URL"),
        "the staging deploy layer's own environment references are missing"
    );

    // The same directory checked against a *different* target is drift, which is
    // why the default output directory is per target.
    let crossed = build(&["examples/triage-fanout/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&crossed), 1);
}

/// An invalid composition emits nothing: generated code is a build artifact of a
/// valid spec (PRD 5.12).
#[test]
fn an_invalid_composition_is_reported_and_nothing_is_written() {
    let out = scratch("invalid");
    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects"))
        .env("NO_COLOR", "1")
        .args(["build", "one-unbalanced-convergence/main.yml", "--out"])
        .arg(&out)
        .output()
        .expect("the command runs");

    assert_eq!(code(&output), 1);
    assert!(
        stderr(&output).contains("is not valid (target `local`)"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "a rejected composition produces no project"
    );
}

/// A pattern the target cannot express refuses the emission, and `validate` is
/// untouched by it.
///
/// The two commands answer different questions. `validate` asks whether the
/// composition is well formed, which does not depend on what it is compiled to,
/// and `(?i)^abc$` is legal RE2 (Decision D12), so it says yes. `build` asks
/// whether *this* target can express it, and a JavaScript regular expression has
/// no spelling for an inline flag group — copying it into `src/schemas.ts` would
/// produce a module that fails to parse, taking every schema in it with it. The
/// bug this pins is `build` exiting `0` over exactly that.
#[test]
fn a_pattern_javascript_cannot_express_refuses_the_build_but_not_validation() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects");
    let out = scratch("pattern");

    let validated = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["validate", "one-untranslatable-pattern/main.yml"])
        .output()
        .expect("the command runs");
    assert_eq!(
        code(&validated),
        0,
        "the composition is valid; the pattern is legal RE2: {}",
        stderr(&validated)
    );

    let built = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["build", "one-untranslatable-pattern/main.yml", "--out"])
        .arg(&out)
        .output()
        .expect("the command runs");

    assert_eq!(code(&built), 1, "{}", stderr(&built));
    let report = stderr(&built);
    for expected in [
        "sets flags inline",
        "(?i)^abc$",
        "(?P<word>…)",
        "(?<word>…)",
        "main.yml:15:10",
        "main.yml:16:10",
        "is valid and cannot be compiled for `local`: 2 errors",
    ] {
        assert!(
            report.contains(expected),
            "the report does not carry `{expected}`:\n{report}"
        );
    }
    assert!(
        !report.contains("(?<word>[a-z]+)-"),
        "the control pattern uses only what both engines share and is not reported:\n{report}"
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "a composition this target cannot express produces no project"
    );
}

/// An entrypoint that is not a readable file is the command's own precondition,
/// not the composition's problem: exit `2`, no report.
#[test]
fn an_unreadable_entrypoint_exits_two() {
    let out = scratch("unreadable");
    let output = build(&[
        "examples/does-not-exist/main.yml",
        "--out",
        out.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&output), 2);
    assert!(
        stderr(&output).starts_with("error: cannot read"),
        "{}",
        stderr(&output)
    );
}

/// A directory where an entrypoint should be is the same precondition, on its
/// other branch — and the sentence names the command the user actually ran.
///
/// The two branches of `main::usable` say different things, and only one of them
/// names a command; a message that told a `build` user what `validate` takes
/// would send them to the wrong page.
#[test]
fn a_directory_as_the_entrypoint_exits_two_naming_this_command() {
    let out = scratch("directory-entrypoint");
    let output = build(&[
        "examples/review-loop",
        "--out",
        out.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&output), 2);
    assert_eq!(
        stderr(&output),
        "error: `examples/review-loop` is not a file: `build` takes a spec entrypoint, \
         conventionally `main.yml`\n"
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "the command never got as far as emitting"
    );
}

/// `--format json` is one document on stdout, with nothing on stderr — the same
/// promise `validate` makes.
#[test]
fn the_machine_format_is_one_document_on_stdout() {
    let out = scratch("json");
    let path = out.to_str().expect("a UTF-8 scratch path");
    let output = build(&[
        "examples/review-loop/main.yml",
        "--out",
        path,
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert_eq!(stderr(&output), "");
    let report: serde_json::Value =
        serde_json::from_str(stdout(&output)).expect("stdout is one JSON document");
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["drift"], serde_json::json!([]));

    fs::write(out.join("src/state.ts"), "// mine now\n").expect("writable");
    let drifted = build(&[
        "examples/review-loop/main.yml",
        "--out",
        path,
        "--check",
        "--format",
        "json",
    ]);
    assert_eq!(code(&drifted), 1);
    let report: serde_json::Value =
        serde_json::from_str(stdout(&drifted)).expect("stdout is one JSON document");
    assert_eq!(
        report["drift"],
        serde_json::json!([{ "path": "src/state.ts", "state": "differs" }])
    );
}

/// With no `--out`, the project lands under the entrypoint's own directory, per
/// target.
#[test]
fn the_default_output_directory_is_beside_the_entrypoint() {
    let project = scratch("default-out");
    fs::write(
        project.join("main.yml"),
        "version: \"0.1\"\nstate:\n  draft: { type: string }\n",
    )
    .expect("the entrypoint is writable");

    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .env("NO_COLOR", "1")
        .arg("build")
        .arg(project.join("main.yml"))
        .output()
        .expect("the command runs");
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    assert!(
        project.join("build/local/src/state.ts").is_file(),
        "wrote {:?}",
        files_under(&project)
    );
}
