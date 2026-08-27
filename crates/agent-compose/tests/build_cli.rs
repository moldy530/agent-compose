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
        stderr(&output).contains("wrote 16 files"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        files_under(&out),
        [
            ".gitignore",
            "README.md",
            "package.json",
            "src/cel.ts",
            "src/cli.ts",
            "src/env.ts",
            "src/graph.ts",
            "src/index.ts",
            "src/journal.ts",
            "src/runtime.ts",
            "src/schemas.ts",
            "src/serve.ts",
            "src/state.ts",
            "src/stores.ts",
            "src/triggers.ts",
            "tsconfig.json",
        ]
    );
}

/// A **warning** does not refuse an emission: the files are written, the exit
/// code is `0`, and the verdict counts the warnings onto it.
///
/// This is the severity's whole meaning, decided at the command that could most
/// easily get it wrong — a build is where "the report was not clean" is the
/// tempting place to stop. `unknown-server-tool` is what makes it load-bearing
/// (grammar 12.1, Decision D122, PRD resolved q30): a provider declaring a
/// server tool this release predates is a composition the compiler cannot fully
/// check and must not refuse, or the second tier's no-treadmill promise is void
/// — an author would be told to wait for a compiler release after all. The
/// `cli` topic states it where an author reads it, and this is what holds the
/// statement true.
#[test]
fn a_warning_does_not_refuse_an_emission() {
    let out = scratch("warned");
    let output = build(&[
        "crates/agent-compose/tests/projects/one-unverifiable-server-tool/main.yml",
        "--out",
        out.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(
        code(&output),
        0,
        "a warning is not a refusal: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("warning[unknown-server-tool]"),
        "the warning is still reported: {}",
        stderr(&output)
    );
    assert!(
        stderr(&output).contains("(target `local`), with 1 warning"),
        "…and the verdict counts it onto the emission: {}",
        stderr(&output)
    );
    assert!(
        files_under(&out).contains(&"src/runtime.ts".to_string()),
        "the project was written: {:?}",
        files_under(&out)
    );

    // The machine report says the same thing in the shape the `cli` topic
    // documents: the key that decides the verdict is empty, and the warning is
    // under its own.
    let output = build(&[
        "crates/agent-compose/tests/projects/one-unverifiable-server-tool/main.yml",
        "--out",
        out.to_str().expect("a UTF-8 scratch path"),
        "--format",
        "json",
    ]);
    assert_eq!(code(&output), 0, "{}", stderr(&output));
    let report: serde_json::Value =
        serde_json::from_str(stdout(&output)).expect("the report is JSON");
    assert_eq!(report["diagnostics"], serde_json::json!([]));
    assert_eq!(report["warnings"][0]["code"], "unknown-server-tool");
    assert_eq!(report["drift"], serde_json::json!([]));
}

/// `build` reads no environment: every `${ENV}` the composition references is
/// unset and it emits anyway, moving the check into the artifact.
///
/// This is one half of PRD 5.9's rule that "`validate` and `build` check ref
/// syntax only … Presence is a launch-time check" — PRD §9's resolved question
/// 15, decided on artifact portability: a build has to succeed on a CI box
/// holding no secrets. The other half is the emitted `src/index.ts`, which runs
/// the check at module scope (PRD §7 M1's "env-ref presence checks at process
/// start"), and `compose-core`'s
/// `tests/generated_code_gates.rs::the_generated_project_checks_its_environment_when_it_is_loaded`
/// is what decides it. Each half is a test rather than a paragraph, because a
/// build that quietly *started* reading the environment would break no other
/// test in this file.
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
        stderr(&output).contains("wrote 16 files"),
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

/// The remedy a `--check` prints is a command that would actually run.
///
/// This is the combination `a_rebuild_prunes_src_and_leaves_the_rest_alone` and
/// `a_src_directory_the_compiler_did_not_write_is_refused` cover one half of
/// each. A stale *generated* module is drift `build` settles; a file under
/// `src/` the compiler did not write is drift `build` **refuses** over; and the
/// drift report gives both the same line — "under `src/` and is not generated" —
/// because the generated-file header is what tells them apart and the reported
/// state does not carry it. A help line that named `build` unconditionally would
/// send a CI reader from an exit `1` they can act on to an exit `2` they cannot,
/// which is the hazard `build.rs`'s own module doc argues for the neighbouring
/// case.
#[test]
fn a_drift_a_rebuild_would_refuse_over_does_not_send_the_reader_to_build() {
    let out = scratch("blocked-remedy");
    let path = out.to_str().expect("a UTF-8 scratch path");
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    fs::write(out.join("src/mine.ts"), "export const mine = 1;\n").expect("writable");

    let drifted = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&drifted), 1, "{}", stderr(&drifted));
    assert!(
        stderr(&drifted).contains("`src/mine.ts` is under `src/` and is not generated"),
        "{}",
        stderr(&drifted)
    );
    assert!(
        stderr(&drifted).contains(
            "help: `agent-compose build` will not regenerate this directory: it holds a file this \
             compiler did not write (`src/mine.ts`), and the build would have replaced or removed \
             it. Point `--out` at a directory of its own, or move it aside"
        ),
        "{}",
        stderr(&drifted)
    );
    assert!(
        !stderr(&drifted).contains("help: run `agent-compose build` to regenerate"),
        "the help a rebuild cannot honour is not printed as well: {}",
        stderr(&drifted)
    );

    // …and the help is right about it: the command it refused to name is the
    // one that refuses.
    let refused = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));

    // The remedy it *did* name settles it, which is what makes the branch a
    // distinction rather than a warning.
    fs::remove_file(out.join("src/mine.ts")).expect("removable");
    fs::write(out.join("src/state.ts"), "// mine now\n").expect("writable");
    let editable = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&editable), 1, "{}", stderr(&editable));
    assert!(
        stderr(&editable).contains("help: run `agent-compose build` to regenerate"),
        "a hand-edited generated file is drift a rebuild fixes: {}",
        stderr(&editable)
    );
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
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

/// Neither is somebody's `package.json`, and `src/` is not what makes it theirs.
///
/// The scenario is a real Node project whose sources are not under `src/`, which
/// is what the `src/` scan cannot see: there is nothing under `src/` to refuse
/// over, and the four files the emitter writes at the root were replaced in
/// place — no refusal, no removal to report, exit `0`, originals gone. Nothing
/// but `git status` would have said so, and an ejected copy (PRD 5.12) is
/// exactly the directory a user points `--out` at by mistake.
#[test]
fn root_files_the_compiler_did_not_write_are_refused() {
    let out = scratch("root-not-ours");
    let path = out.to_str().expect("a UTF-8 scratch path");
    fs::write(out.join("package.json"), "{\"name\":\"my-real-app\"}\n").expect("writable");
    fs::write(out.join("README.md"), "# My real project\n").expect("writable");
    fs::create_dir_all(out.join("lib")).expect("writable");
    fs::write(out.join("lib/index.ts"), "export const mine = 1;\n").expect("writable");

    let refused = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));
    for expected in ["`README.md`", "`package.json`", "generated-file header"] {
        assert!(
            stderr(&refused).contains(expected),
            "the refusal does not carry `{expected}`: {}",
            stderr(&refused)
        );
    }
    assert_eq!(
        fs::read_to_string(out.join("package.json")).expect("readable"),
        "{\"name\":\"my-real-app\"}\n",
        "the manifest is the user's and is still theirs"
    );
    assert_eq!(
        fs::read_to_string(out.join("README.md")).expect("readable"),
        "# My real project\n"
    );
    assert_eq!(
        files_under(&out),
        ["README.md", "lib/index.ts", "package.json"],
        "the refusal is total: nothing was written"
    );

    // …and the same directory with those two files gone builds, which is what
    // makes this a refusal rather than a rule against building into a directory
    // that has anything in it.
    fs::remove_file(out.join("package.json")).expect("removable");
    fs::remove_file(out.join("README.md")).expect("removable");
    let built = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    assert!(
        out.join("lib/index.ts").is_file(),
        "and the code that was never in the way is still there"
    );
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
///
/// Two of the four are the quieter half: `^[]-]$` and `^[a[b]]$` both *parse* as
/// JavaScript and denote a different set there, so nothing fails and the
/// published schema and the emitted parse answer differently.
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
        "writes `]` unescaped inside a character class",
        "^[]-]$",
        "nests a character class",
        "^[a[b]]$",
        "main.yml:23:12",
        "main.yml:24:12",
        "main.yml:25:12",
        "main.yml:26:12",
        "is valid and cannot be compiled for `local`: 4 errors",
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
    assert!(
        !report.contains("^[\\]-]$"),
        "the escaped class is the spelling that transfers and is not reported:\n{report}"
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "a composition this target cannot express produces no project"
    );
}

/// The same question asked of a `matches()` argument, which is the other place a
/// composition writes a regular expression.
///
/// Grammar 4.1 puts CEL's standard `matches` on the expression surface and the
/// specification defines it over RE2, so a guard carries the same second
/// language a `pattern:` does — while the evaluator a compiled router embeds has
/// only `new RegExp(…)`. The mismatch is reachable from **both** sides, which is
/// what the fixture states and this pins:
///
/// * `(?i)urgent` is RE2 and a `SyntaxError` in JavaScript, so `validate` accepts
///   a guard that kills the run at the first edge that evaluates it;
/// * `a(?=b)` and `(a)\1` are JavaScript and are not RE2, so a compiled router
///   answers a guard the specification's own engine will not compile — two
///   interpreters, two answers, which is the drift the conformance corpus exists
///   to prevent;
/// * a pattern read out of a channel is one the compiler never sees, so it can
///   promise nothing about it in either direction.
///
/// The two controls are what keep this from being a check that refuses every
/// `matches()`: both standard spellings, over the vocabulary both engines share,
/// in the same file and unreported.
#[test]
fn a_matches_pattern_javascript_cannot_express_refuses_the_build_but_not_validation() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects");
    let out = scratch("matches-pattern");

    let validated = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["validate", "one-untranslatable-matches-pattern/main.yml"])
        .output()
        .expect("the command runs");
    assert_eq!(
        code(&validated),
        0,
        "the composition is valid: every guard is a well-typed CEL bool: {}",
        stderr(&validated)
    );

    let built = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args([
            "build",
            "one-untranslatable-matches-pattern/main.yml",
            "--out",
        ])
        .arg(&out)
        .output()
        .expect("the command runs");

    assert_eq!(code(&built), 1, "{}", stderr(&built));
    let report = stderr(&built);
    for expected in [
        // Each message names the surface, because a span alone does not say
        // which of an expression's calls is the one to fix.
        "`flow.probe`'s edge `read` → `act` guard calls `matches()`",
        "sets flags inline",
        "(?i)urgent",
        "look-around, including look-ahead and look-behind, is not supported",
        "a(?=b)",
        "backreferences are not supported",
        "(a)\\1",
        "the argument is computed rather than written down",
        "write the pattern as a string literal",
        "main.yml:59:36",
        "main.yml:60:36",
        "main.yml:61:36",
        "main.yml:62:36",
        "is valid and cannot be compiled for `local`: 4 errors",
    ] {
        assert!(
            report.contains(expected),
            "the report does not carry `{expected}`:\n{report}"
        );
    }
    for control in ["^[a-z]+-[0-9]{4}$", "^(?<word>[a-z]+)$"] {
        assert!(
            !report.contains(control),
            "the control pattern `{control}` uses only what both engines share and is not \
             reported:\n{report}"
        );
    }
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "a composition this target cannot express produces no project"
    );
}

/// A channel name the target cannot hold refuses the emission, and `validate` is
/// untouched by it.
///
/// The bug this pins is the quietest one the emitter had: `state:` may declare a
/// channel called `constructor` — a legal grammar 2.1 identifier, and grammar 2.5
/// reserves only the seven roots — and the project that comes out validates,
/// builds, type-checks under the pinned `tsc`, and loads under Node. It fails at
/// `new StateGraph(State)`, because LangGraph asks its channel table for
/// `constructor` and `Object.prototype` answers. Nothing a build runs would ever
/// have seen it.
#[test]
fn a_channel_name_the_target_cannot_hold_refuses_the_build_but_not_validation() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects");
    let out = scratch("channel-name");

    let validated = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["validate", "one-unrepresentable-channel-name/main.yml"])
        .output()
        .expect("the command runs");
    assert_eq!(
        code(&validated),
        0,
        "the composition is valid; `constructor` is a legal identifier: {}",
        stderr(&validated)
    );

    let built = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args([
            "build",
            "one-unrepresentable-channel-name/main.yml",
            "--out",
        ])
        .arg(&out)
        .output()
        .expect("the command runs");

    assert_eq!(code(&built), 1, "{}", stderr(&built));
    let report = stderr(&built);
    for expected in [
        "a state channel named `constructor`",
        "Object.prototype",
        "rename the channel",
        // The span is the key itself, on its own line.
        "main.yml:19:3",
        "is valid and cannot be compiled for `local`: 1 error",
    ] {
        assert!(
            report.contains(expected),
            "the report does not carry `{expected}`:\n{report}"
        );
    }
    assert!(
        !report.contains("`construct`") && !report.contains("`constructors`"),
        "the neighbours in the same section are not reported:\n{report}"
    );
    assert_eq!(
        files_under(&out),
        Vec::<String>::new(),
        "a composition this target cannot express produces no project"
    );
}

/// The same name one level down: a **schema property** and a **discriminator**
/// are keys of an emitted object shape too, and both refuse the build.
///
/// The channel rule above is about the one key `StateGraph` reads. These are the
/// keys `z.object({…})` and `z.discriminatedUnion` read, off the value being
/// parsed, and the failure is quieter than the channel one in both cases: the
/// property version type-checks, loads, constructs its graph, and then refuses a
/// document the project's own published JSON Schema accepts; the discriminator
/// version gets that far and throws out of the first `safeParse`.
#[test]
fn a_schema_key_the_target_cannot_hold_refuses_the_build_but_not_validation() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects");
    let out = scratch("schema-key");

    let validated = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["validate", "one-unrepresentable-schema-key/main.yml"])
        .output()
        .expect("the command runs");
    assert_eq!(
        code(&validated),
        0,
        "the composition is valid; `constructor` is a legal identifier: {}",
        stderr(&validated)
    );

    let built = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(&projects)
        .env("NO_COLOR", "1")
        .args(["build", "one-unrepresentable-schema-key/main.yml", "--out"])
        .arg(&out)
        .output()
        .expect("the command runs");

    assert_eq!(code(&built), 1, "{}", stderr(&built));
    let report = stderr(&built);
    for expected in [
        "a schema property named `constructor`",
        "a discriminator named `constructor`",
        "Object.prototype",
        "rename it",
        // Each span is the key itself, where it is written.
        "main.yml:28:7",
        "main.yml:30:20",
        "is valid and cannot be compiled for `local`: 2 errors",
    ] {
        assert!(
            report.contains(expected),
            "the report does not carry `{expected}`:\n{report}"
        );
    }
    assert!(
        !report.contains("`constructors`")
            && !report.contains("`construct`")
            && !report.contains("constructor_name")
            && !report.contains("named `kind`"),
        "the neighbours in the same file are not reported:\n{report}"
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
