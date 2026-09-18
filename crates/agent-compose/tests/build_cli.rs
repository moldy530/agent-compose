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
        stderr(&output).contains("wrote 25 files"),
        "{}",
        stderr(&output)
    );
    assert_eq!(
        files_under(&out),
        [
            ".gitignore",
            "README.md",
            "manifest.json",
            "package.json",
            "src/artifact.ts",
            "src/cel.ts",
            "src/cli.ts",
            "src/delivery.ts",
            "src/deployment.ts",
            "src/env.ts",
            "src/graph.ts",
            "src/harness.ts",
            "src/index.ts",
            "src/journal.ts",
            "src/mesh.ts",
            "src/modules.ts",
            "src/otlp.ts",
            "src/runtime.ts",
            "src/schemas.ts",
            "src/serve.ts",
            "src/state.ts",
            "src/stores.ts",
            "src/triggers.ts",
            "src/worker-node.ts",
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
        stderr(&output).contains("wrote 25 files"),
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

/// A registry credential reaches the emitted installer configuration as the
/// **name** of a variable, even when the build is run with that variable set to
/// a value (grammar §14.6, PRD resolved q59 ruling c).
///
/// The claim the ruling makes is that the token is a spelling rather than a
/// byte, so nothing secret enters the artifact and the artifact hash is stable
/// across a rotation. A unit test over the emitter cannot make it honestly —
/// mutating a test binary's environment races every thread beside it — so it is
/// made here, over a real `agent-compose build` that has the value in its
/// environment and writes the name anyway. The second half, that the hash does
/// not move, is the same build run again under a different value.
#[test]
fn build_writes_the_registry_reference_rather_than_the_token_it_resolves_to() {
    const SECRET: &str = "npm-token-that-must-not-be-written";
    let out = scratch("registry-reference");
    let path = out.to_str().expect("a UTF-8 scratch path");

    let mut command = Command::cargo_bin("agent-compose").expect("the binary under test is built");
    command
        .current_dir(repo_root())
        .env("NO_COLOR", "1")
        .env("NPM_MIRROR_TOKEN", SECRET)
        .env("NPM_CORP_TOKEN", SECRET)
        .args([
            "build",
            "examples/triage-fanout/main.yml",
            "--target",
            "staging",
            "--out",
            path,
        ]);
    let output = command.output().expect("the command runs");
    assert_eq!(code(&output), 0, "{}", stderr(&output));

    let bunfig = fs::read_to_string(out.join("bunfig.toml")).expect("`bunfig.toml` was written");
    let npmrc = fs::read_to_string(out.join(".npmrc")).expect("`.npmrc` was written");
    for (name, contents) in [("bunfig.toml", &bunfig), (".npmrc", &npmrc)] {
        assert!(
            !contents.contains(SECRET),
            "`{name}` carries the resolved value of a credential:\n{contents}"
        );
        assert!(
            contents.contains("NPM_MIRROR_TOKEN") && contents.contains("NPM_CORP_TOKEN"),
            "`{name}` does not carry the references:\n{contents}"
        );
    }
    assert!(
        bunfig.contains("token = \"$NPM_MIRROR_TOKEN\""),
        "Bun's substitution is `$VAR`:\n{bunfig}"
    );
    assert!(
        npmrc.contains(":_authToken=${NPM_MIRROR_TOKEN}"),
        "npm's is `${{VAR}}`, on a host-scoped line:\n{npmrc}"
    );

    // …and the artifact hash does not move when the value does, which is what
    // makes a token rotation not a redeployment (PRD resolved q40).
    let artifact = fs::read_to_string(out.join("src/artifact.ts")).expect("the artifact module");
    let rotated = scratch("registry-reference-rotated");
    let mut second = Command::cargo_bin("agent-compose").expect("the binary under test is built");
    second
        .current_dir(repo_root())
        .env("NO_COLOR", "1")
        .env("NPM_MIRROR_TOKEN", "a-rotated-value")
        .env("NPM_CORP_TOKEN", "another-rotated-value")
        .args([
            "build",
            "examples/triage-fanout/main.yml",
            "--target",
            "staging",
            "--out",
            rotated.to_str().expect("a UTF-8 scratch path"),
        ]);
    assert_eq!(code(&second.output().expect("the command runs")), 0);
    assert_eq!(
        fs::read_to_string(rotated.join("src/artifact.ts")).expect("the artifact module"),
        artifact,
        "rotating a registry token changed the artifact"
    );
}

/// A target that declares no `package_registry:` gets neither installer file,
/// and a `--check` of such a build does not ask for one (grammar §14.6).
#[test]
fn a_target_with_no_registry_writes_no_installer_configuration() {
    let out = scratch("no-registry");
    let path = out.to_str().expect("a UTF-8 scratch path");
    let built = build(&["examples/triage-fanout/main.yml", "--out", path]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    assert!(!out.join("bunfig.toml").exists());
    assert!(!out.join(".npmrc").exists());

    let checked = build(&["examples/triage-fanout/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&checked), 0, "{}", stderr(&checked));

    // …and a `bunfig.toml` an author left there is theirs, because the compiler
    // claims no name it does not write: the refusal below is about the target
    // that *does* declare a registry.
    fs::write(out.join("bunfig.toml"), "# mine\n").expect("writable");
    let again = build(&["examples/triage-fanout/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
}

/// An authored file already holding one of the two installer names stops a first
/// build of a target that declares a registry, naming it (grammar §14.6 rule 4,
/// PRD resolved q47).
///
/// The two files are pure-generated members of the emitted list, so the refusal
/// that protects `package.json` protects these — which is exactly the promotion
/// q59 makes: the shape it replaces is a `bunfig.toml` somebody dropped beside a
/// built project, and a build that silently replaced one would destroy the very
/// configuration this key exists to stop people hand-maintaining.
#[test]
fn an_authored_installer_file_stops_a_build_that_would_write_one() {
    for name in ["bunfig.toml", ".npmrc"] {
        let out = scratch(&format!("installer-collision-{name}"));
        let path = out.to_str().expect("a UTF-8 scratch path");
        fs::write(out.join(name), "# mine\n").expect("writable");

        let refused = build(&[
            "examples/triage-fanout/main.yml",
            "--target",
            "staging",
            "--out",
            path,
        ]);
        assert_eq!(code(&refused), 2, "{}", stderr(&refused));
        assert!(
            stderr(&refused).contains(&format!("`{name}`")),
            "the refusal does not name the file it would have replaced: {}",
            stderr(&refused)
        );
        assert_eq!(
            fs::read_to_string(out.join(name)).expect("readable"),
            "# mine\n",
            "the author's file was replaced before the scan decided"
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

/// A file the build does not emit is not the build's business — anywhere.
///
/// This is PRD resolved q47's boundary from the side that changed. `build`
/// overwrites and `--check`s exactly the file list it emits; a hand-authored
/// module under `src/`, an install, a `.env`, and even a module an older
/// compiler release wrote are all outside that list, so none of them is drift
/// and none of them is removed. Authored code living in the same tree is what
/// this buys.
#[test]
fn a_file_the_build_does_not_emit_is_left_where_it_is() {
    let out = scratch("boundary");
    let path = out.to_str().expect("a UTF-8 scratch path");
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
    fs::create_dir_all(out.join("src/tools")).expect("writable");
    fs::write(out.join("src/tools/sign.ts"), "export default 1;\n").expect("writable");
    // A module an older compiler release wrote. It carries the header, and it
    // is still not on this build's list, so this build says nothing about it.
    fs::create_dir_all(out.join("src/nodes")).expect("writable");
    fs::write(
        out.join("src/nodes/old.ts"),
        "// This file was generated by agent-compose 0.0.1 from `main.yml`.\n",
    )
    .expect("writable");
    fs::create_dir_all(out.join("node_modules/zod")).expect("writable");
    fs::write(out.join("node_modules/zod/index.js"), "//\n").expect("writable");
    fs::write(out.join(".env"), "SEARCH_HOST=example.test\n").expect("writable");

    let checked = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&checked), 0, "{}", stderr(&checked));

    let rebuilt = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&rebuilt), 0, "{}", stderr(&rebuilt));
    assert_eq!(
        fs::read_to_string(out.join("src/tools/sign.ts")).expect("readable"),
        "export default 1;\n",
        "authored code in the emitted tree survives a rebuild untouched"
    );
    assert!(out.join("src/nodes/old.ts").is_file());
    assert!(out.join("node_modules/zod/index.js").is_file());
    assert!(out.join(".env").is_file());
}

/// The remedy a `--check` prints is a command that would actually run.
///
/// Both halves in one test, because the distinction is the point. A generated
/// file somebody edited is drift `build` settles; an emitted **name** somebody
/// else's file holds, in a directory this compiler has never built into, is
/// drift `build` **refuses** over — and the report gives both the same line,
/// `differs`, because the generated-file header is what tells them apart and the
/// reported state does not carry it. A help line that named `build`
/// unconditionally would send a CI reader from an exit `1` they can act on to an
/// exit `2` they cannot.
#[test]
fn a_drift_a_rebuild_would_refuse_over_does_not_send_the_reader_to_build() {
    let out = scratch("blocked-remedy");
    let path = out.to_str().expect("a UTF-8 scratch path");
    fs::write(out.join("package.json"), "{\"name\":\"my-real-app\"}\n").expect("writable");

    let drifted = build(&["examples/review-loop/main.yml", "--out", path, "--check"]);
    assert_eq!(code(&drifted), 1, "{}", stderr(&drifted));
    assert!(
        stderr(&drifted).contains("`package.json` differs from what the spec produces"),
        "{}",
        stderr(&drifted)
    );
    assert!(
        stderr(&drifted).contains(
            "help: `agent-compose build` will not regenerate this directory: it holds a file this \
             compiler did not write at a file this build writes (`package.json`), and the build \
             would have replaced it. Point `--out` at a directory of its own, or move it aside"
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

    // The remedy it *does* name settles the other half, which is what makes the
    // branch a distinction rather than a warning.
    fs::remove_file(out.join("package.json")).expect("removable");
    assert_eq!(
        code(&build(&["examples/review-loop/main.yml", "--out", path])),
        0
    );
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

/// An emitted name somebody else's file holds stops the build.
///
/// `--out` can name any directory — an existing Node project, an ejected copy
/// being re-synced, `.` — and the compiler's claim on a name is a claim about a
/// directory it built. The evidence is the generated-file header, so a project
/// with none of its files is refused rather than overwritten. The scenario
/// covers both halves of the emitted list at once: a source module under `src/`,
/// and a `package.json` and `README.md` at the root, which no walk of `src/`
/// would ever have seen.
#[test]
fn emitted_names_the_compiler_did_not_write_are_refused() {
    let out = scratch("not-ours");
    let path = out.to_str().expect("a UTF-8 scratch path");
    fs::create_dir_all(out.join("src")).expect("writable");
    fs::write(out.join("src/graph.ts"), "export const mine = 1;\n").expect("writable");
    fs::write(out.join("package.json"), "{\"name\":\"my-real-app\"}\n").expect("writable");
    fs::write(out.join("README.md"), "# My real project\n").expect("writable");
    fs::create_dir_all(out.join("lib")).expect("writable");
    fs::write(out.join("lib/index.ts"), "export const mine = 2;\n").expect("writable");
    // Not a name the emitter writes, so not part of the refusal.
    fs::write(out.join("src/mine.ts"), "export const mine = 3;\n").expect("writable");

    let refused = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));
    for expected in [
        "`README.md`",
        "`package.json`",
        "`src/graph.ts`",
        "generated-file header",
    ] {
        assert!(
            stderr(&refused).contains(expected),
            "the refusal does not carry `{expected}`: {}",
            stderr(&refused)
        );
    }
    assert!(
        !stderr(&refused).contains("`src/mine.ts`"),
        "a file at no emitted name is nothing to refuse over: {}",
        stderr(&refused)
    );
    assert_eq!(
        fs::read_to_string(out.join("package.json")).expect("readable"),
        "{\"name\":\"my-real-app\"}\n",
        "the manifest is the user's and is still theirs"
    );
    assert_eq!(
        files_under(&out),
        [
            "README.md",
            "lib/index.ts",
            "package.json",
            "src/graph.ts",
            "src/mine.ts"
        ],
        "the refusal is total: nothing was written"
    );

    // …and the same directory with those three gone builds, which is what makes
    // this a refusal rather than a rule against building into a directory that
    // has anything in it.
    fs::remove_file(out.join("package.json")).expect("removable");
    fs::remove_file(out.join("README.md")).expect("removable");
    fs::remove_file(out.join("src/graph.ts")).expect("removable");
    let built = build(&["examples/review-loop/main.yml", "--out", path]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    assert!(
        out.join("lib/index.ts").is_file(),
        "and the code that was never in the way is still there"
    );
    assert!(
        out.join("src/mine.ts").is_file(),
        "as is the module beside the generated ones"
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
    // Every key is present whatever the outcome, so a consumer parses one
    // document: this composition binds no module, and the answer is an empty
    // array rather than a missing key.
    assert_eq!(report["scaffolded"], serde_json::json!([]));

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

/// The whole life of a `module:` binding through the command line: refused,
/// scaffolded, filled in, and then left alone (grammar 6.1, PRD resolved q47,
/// q48).
///
/// The four moments are one test because each is only meaningful against the
/// others. `validate` refusing a missing implementation is only useful if the
/// command it names writes one; `build` writing one is only safe if the next
/// build leaves it there; and `--check` refusing is only right if it says the
/// same thing `validate` did.
#[test]
fn a_module_binding_is_refused_then_scaffolded_then_left_alone() {
    let project = scratch("module-binding");
    let entrypoint = project.join("main.yml");
    fs::write(
        &entrypoint,
        r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts
"#,
    )
    .expect("the entrypoint is writable");
    let spec = entrypoint.to_str().expect("a UTF-8 scratch path");
    let out = project.join("build/local");

    let run = |verb: &str, arguments: &[&str]| {
        Command::cargo_bin("agent-compose")
            .expect("the binary under test is built")
            .env("NO_COLOR", "1")
            .arg(verb)
            .arg(spec)
            .args(arguments)
            .output()
            .expect("the command runs")
    };

    // 1. `validate` refuses, and names the command that repairs it.
    let refused = run("validate", &[]);
    assert_eq!(code(&refused), 1, "{}", stderr(&refused));
    assert!(
        stderr(&refused)
            .contains("`tool.sign` is implemented by `src/tools/sign.ts`, which does not exist"),
        "{}",
        stderr(&refused)
    );
    assert!(
        stderr(&refused).contains("run `agent-compose build`"),
        "{}",
        stderr(&refused)
    );

    // …and so does `--check`, with the same sentence: a committed project whose
    // implementation is missing does not run.
    let checked = run("build", &["--check"]);
    assert_eq!(code(&checked), 1, "{}", stderr(&checked));
    assert!(
        stderr(&checked)
            .contains("`tool.sign` is implemented by `src/tools/sign.ts`, which does not exist"),
        "{}",
        stderr(&checked)
    );
    assert!(
        !project.join("src/tools/sign.ts").exists(),
        "`--check` writes nothing"
    );

    // 2. `build` scaffolds it — in the project, beside `main.yml`, not in the
    //    output directory — and says so.
    let built = run("build", &[]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    assert!(
        stderr(&built).contains(
            "scaffolded 1 tool implementation for you to write: `src/tools/sign.ts` (`tool.sign`)"
        ),
        "{}",
        stderr(&built)
    );
    let stub = fs::read_to_string(project.join("src/tools/sign.ts")).expect("the stub is readable");
    assert!(
        stub.contains("import type { ToolSignModule } from \"../modules.ts\";"),
        "the stub types itself against the generated contract: {stub}"
    );
    assert!(
        stub.contains("const toolSign: ToolSignModule = async (input) => {"),
        "the annotation is what holds the file to that contract: {stub}"
    );
    assert!(
        stub.contains("`tool.sign` — Sign a payload."),
        "the stub names the tool and its contract: {stub}"
    );
    assert!(
        !stub.contains("generated by agent-compose"),
        "a scaffold is the author's, so it carries no generated-file header: {stub}"
    );
    // The build carried it into the artifact in the same breath, because the
    // composition references it (PRD resolved q49).
    assert!(
        stderr(&built).contains("carrying 1 authored file with them"),
        "{}",
        stderr(&built)
    );
    assert_eq!(
        fs::read_to_string(out.join("src/tools/sign.ts")).expect("the carried copy is readable"),
        stub,
        "the artifact carries the file the spec references"
    );
    let artifact = fs::read_to_string(out.join("src/artifact.ts")).expect("readable");
    assert!(
        artifact.contains("\"src/tools/sign.ts\","),
        "…and lists it, so the hub tars it for a worker: {artifact}"
    );
    let manifest = fs::read_to_string(out.join("manifest.json")).expect("readable");
    assert!(
        manifest.contains("\"authored\": [\n    \"src/tools/sign.ts\"\n  ],"),
        "…and says which half of the tree it is: {manifest}"
    );

    // …and the machine report carries the same claim, because a job reading
    // JSON is exactly the reader who cannot see a verdict line: a build that
    // wrote a file into the source tree and reported four empty arrays would
    // have a CI run commit an unreviewed stub or throw it away, and a scaffold
    // discarded is never offered again.
    let second_project = scratch("module-binding-json");
    let second_entrypoint = second_project.join("main.yml");
    fs::write(
        &second_entrypoint,
        fs::read_to_string(&entrypoint).expect("the entrypoint is readable"),
    )
    .expect("the entrypoint is writable");
    let scaffolding = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .env("NO_COLOR", "1")
        .arg("build")
        .arg(&second_entrypoint)
        .args(["--format", "json"])
        .output()
        .expect("the command runs");
    assert_eq!(code(&scaffolding), 0, "{}", stderr(&scaffolding));
    let report: serde_json::Value =
        serde_json::from_str(stdout(&scaffolding)).expect("stdout is one JSON document");
    assert_eq!(
        report["scaffolded"],
        serde_json::json!([{ "path": "src/tools/sign.ts", "tool": "tool.sign" }]),
        "the machine report names the file this build wrote into the project: {report}"
    );
    // …and the build after it reports none, for the reason the human verdict
    // says nothing: the file is the author's now.
    let quiet = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .env("NO_COLOR", "1")
        .arg("build")
        .arg(&second_entrypoint)
        .args(["--format", "json"])
        .output()
        .expect("the command runs");
    assert_eq!(code(&quiet), 0, "{}", stderr(&quiet));
    let report: serde_json::Value =
        serde_json::from_str(stdout(&quiet)).expect("stdout is one JSON document");
    assert_eq!(report["scaffolded"], serde_json::json!([]), "{report}");

    // 3. `validate` is clean now, and so is `--check`: the scaffold is not on
    //    the emitted file list, so nothing compares it against a template.
    let validated = run("validate", &[]);
    assert_eq!(code(&validated), 0, "{}", stderr(&validated));
    assert_eq!(code(&run("build", &["--check"])), 0);

    // 4. The author writes the implementation. A rebuild neither rewrites it nor
    //    mentions it, `--check` still passes, and the artifact hash has moved
    //    because the tree has.
    let before = fs::read_to_string(out.join("src/artifact.ts")).expect("readable");
    fs::write(
        project.join("src/tools/sign.ts"),
        "export default async function sign() {\n  return { signature: \"ok\" };\n}\n",
    )
    .expect("writable");
    // …and until it is rebuilt, the carried copy is stale, which `--check`
    // reports as drift like any other file of the tree.
    let stale = run("build", &["--check"]);
    assert_eq!(code(&stale), 1, "{}", stderr(&stale));
    assert!(
        stderr(&stale).contains("`src/tools/sign.ts` differs from what the spec produces"),
        "{}",
        stderr(&stale)
    );

    let again = run("build", &[]);
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert!(
        !stderr(&again).contains("scaffolded"),
        "a build that scaffolds nothing says nothing about it: {}",
        stderr(&again)
    );
    assert_eq!(
        fs::read_to_string(project.join("src/tools/sign.ts")).expect("readable"),
        "export default async function sign() {\n  return { signature: \"ok\" };\n}\n",
        "the compiler never writes that file twice"
    );
    assert_eq!(code(&run("build", &["--check"])), 0);
    assert_ne!(
        fs::read_to_string(out.join("src/artifact.ts")).expect("readable"),
        before,
        "an edited implementation is a different artifact, so a mesh redeploys"
    );
}

/// The verbs that build on the way say what the build wrote into the author's
/// tree (PRD resolved q48).
///
/// `run` and `serve` validate and build before they launch, and that build
/// scaffolds an absent `module:` implementation exactly as a plain `build` does
/// — a write into the *project*, made once and never again. A verb that made it
/// silently would leave the one file the compiler cannot rewrite to be
/// discovered by a later `git status`, and would say something different from
/// the verb beside it about the same write. Both are asserted here: the first
/// `run` announces it, and the second — with the file already there — says
/// nothing, because nothing was written.
///
/// The launch itself fails, and that is not what is under test: a scratch
/// project has no `node_modules/`, so the run stops at the precondition. The
/// notice is printed before it and on **stderr**, which is where a launch's
/// reports go — `run`'s stdout is the flow's answer.
#[test]
fn a_run_that_scaffolds_says_so_and_a_run_that_does_not_stays_quiet() {
    let project = scratch("module-binding-launch");
    let entrypoint = project.join("main.yml");
    fs::write(
        &entrypoint,
        r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module: ./src/tools/sign.ts

flow.sign:
  description: Sign one payload.
  inputs:
    payload: { type: string, min_length: 1 }
  outputs:
    signature: { type: string }
  nodes:
    sign:
      function: tool.sign
      input:
        payload: "input.payload"
      writes:
        signature: signature
  edges:
    - { from: start, to: sign }
    - { from: sign, to: end }

state:
  signature: { description: What came back., type: string, default: "" }
"#,
    )
    .expect("the entrypoint is writable");
    let spec = entrypoint.to_str().expect("a UTF-8 scratch path");

    let launch = || {
        Command::cargo_bin("agent-compose")
            .expect("the binary under test is built")
            .env("NO_COLOR", "1")
            .args(["run", spec, "flow.sign", "--input", "payload=hi"])
            .output()
            .expect("the command runs")
    };

    let first = launch();
    assert!(
        stderr(&first).contains(
            "scaffolded 1 tool implementation for you to write: `src/tools/sign.ts` (`tool.sign`)"
        ),
        "a `run` that wrote into the project says so: {}",
        stderr(&first)
    );
    assert!(
        !stdout(&first).contains("scaffolded"),
        "…on stderr, because stdout is the run's answer: {}",
        stdout(&first)
    );
    assert!(
        project.join("src/tools/sign.ts").is_file(),
        "the stub landed in the project, beside `main.yml`: {:?}",
        files_under(&project)
    );

    let second = launch();
    assert!(
        !stderr(&second).contains("scaffolded"),
        "a `run` that scaffolds nothing says nothing about it: {}",
        stderr(&second)
    );
}

/// A `module:` binding's `dependencies:` reach the generated `package.json`, and
/// an emitted name an authored file already holds stops the build (PRD resolved
/// q47, q49).
///
/// Two halves of one boundary, driven through the real command because that is
/// where they meet: the manifest is the compiler's to write and stays wholly
/// generated, and the one thing a composition can put *into* it is a pin the
/// binding declares. The refusal is the other direction — a `--out` this
/// compiler never built into, holding somebody's file at a name it emits — and
/// it names both sides rather than reporting that something went wrong.
#[test]
fn a_modules_dependencies_reach_the_manifest_and_an_occupied_name_stops_the_build() {
    let project = scratch("module-dependencies");
    let entrypoint = project.join("main.yml");
    fs::write(
        &entrypoint,
        r#"version: "0.1"

tool.sign:
  description: Sign a payload.
  input:
    payload: { type: string }
  output:
    signature: { type: string }
  module:
    path: ./src/tools/sign.ts
    dependencies:
      "@noble/hashes": "1.4.0"
"#,
    )
    .expect("the entrypoint is writable");
    let spec = entrypoint.to_str().expect("a UTF-8 scratch path");
    let out = project.join("build/local");

    let built = build(&[spec]);
    assert_eq!(code(&built), 0, "{}", stderr(&built));
    let manifest = fs::read_to_string(out.join("package.json")).expect("readable");
    assert!(
        manifest.contains("\"@noble/hashes\": \"1.4.0\""),
        "the declared pin is not in the generated manifest: {manifest}"
    );
    assert!(
        manifest.contains("\"@langchain/langgraph\""),
        "…and the runtime's own set is still there: {manifest}"
    );
    assert!(
        manifest.contains("generated by agent-compose"),
        "…in a file that is still wholly the compiler's: {manifest}"
    );

    // The other direction. A directory this compiler never built into, holding
    // one file at a name the emitter writes, is refused — and the message names
    // the directory, the file, and what the build would have done to it.
    let elsewhere = scratch("module-occupied");
    fs::create_dir_all(elsewhere.join("src")).expect("writable");
    fs::write(elsewhere.join("src/modules.ts"), "export const mine = 1;\n").expect("writable");
    let refused = build(&[
        spec,
        "--out",
        elsewhere.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&refused), 2, "{}", stderr(&refused));
    assert!(
        stderr(&refused).contains("`src/modules.ts`"),
        "{}",
        stderr(&refused)
    );
    assert_eq!(
        fs::read_to_string(elsewhere.join("src/modules.ts")).expect("readable"),
        "export const mine = 1;\n",
        "nothing is overwritten before the scan decides"
    );

    // The same refusal over a **carried** name, which is the half the sentence
    // has to be true about: `src/tools/sign.ts` is a file this build writes and
    // one it never emits, so a message saying "a file it emits" would name the
    // one distinction PRD resolved q47 exists to draw and get it backwards.
    let carried = scratch("module-carried-occupied");
    fs::create_dir_all(carried.join("src/tools")).expect("writable");
    fs::write(carried.join("src/tools/sign.ts"), "// somebody else's\n").expect("writable");
    let collided = build(&[
        spec,
        "--out",
        carried.to_str().expect("a UTF-8 scratch path"),
    ]);
    assert_eq!(code(&collided), 2, "{}", stderr(&collided));
    assert!(
        stderr(&collided).contains(
            "holds a file this compiler did not write at a file this build writes \
             (`src/tools/sign.ts`)"
        ),
        "{}",
        stderr(&collided)
    );
    assert_eq!(
        fs::read_to_string(carried.join("src/tools/sign.ts")).expect("readable"),
        "// somebody else's\n",
        "nothing is overwritten before the scan decides"
    );
}
