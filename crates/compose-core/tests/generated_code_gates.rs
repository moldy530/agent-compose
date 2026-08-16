//! The generated-code checks of CLAUDE.md's *Validation strategy*, run against
//! the **real** pinned JavaScript toolchain.
//!
//! Three gates, in increasing strength:
//!
//! 1. **`tsc --noEmit`** — every golden project type-checks under its own strict
//!    `tsconfig.json`, against installed `@langchain/langgraph`, `@langchain/core`
//!    and `zod`. Not a stub, not a shim: the versions
//!    `compose_core::codegen::project::PINS` names, downloaded and resolved.
//! 2. **Construction** — Node runs the emitted TypeScript and builds a
//!    `StateGraph` over the state model. A channel spec LangGraph refuses is a
//!    green `tsc` and a runtime failure, so type-checking alone would not catch
//!    it. The channel names come back and are compared against the composition's
//!    own `state:` section, so a channel dropped between the IR and the emitted
//!    module fails here.
//! 3. **Schema-lowering agreement** — the corpus under
//!    `tests/fixtures/schema-lowering/` is validated twice: against the JSON
//!    Schema this compiler lowers to (Rust, the `jsonschema` crate) and against
//!    the Zod the same module emits (Node). Grammar 3.8 is one table with two
//!    columns and this is what keeps them from drifting apart. Each document also
//!    carries the verdict it *should* get, so two implementations agreeing on a
//!    wrong answer is still a failure.
//!
//! # The toolchain fixture
//!
//! `tests/toolchain/` holds a `package.json` and a committed `package-lock.json`
//! pinning exactly what the emitter pins, and `npm ci` installs it once per run.
//! One install serves every golden because the emitted dependency set is a
//! compiler constant rather than a per-project one — two projects built by one
//! compiler release declare identical versions, so installing once and
//! type-checking each against that install is the same check at a third of the
//! network cost. `the_toolchain_fixture_pins_what_the_emitter_pins` is what keeps
//! the fixture and the emitter from disagreeing.
//!
//! Each golden is **copied** into `tests/toolchain/projects/<name>/` rather than
//! checked in place, so `node_modules/` resolution finds the shared install by
//! walking up, and the committed goldens stay exactly the bytes the emitter
//! wrote.
//!
//! # When Node is missing
//!
//! **In CI these gates fail; on a developer machine they skip.** CI is where "the
//! suite is green" has to mean "the generated code is checked" (CLAUDE.md), so a
//! missing toolchain there is a broken job, not an absent one — the workflow
//! installs Node for exactly this. Locally, a contributor working on the parser
//! should not be blocked by a Node install, and the skip says so on stderr rather
//! than passing silently. `CI` (any non-empty value, which every CI provider
//! sets) is the switch.

#[path = "support/goldens.rs"]
mod goldens;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use goldens::{GOLDENS, Golden, artifact, emitted, files_under, goldens_root};
use serde_json::Value;

/// Where the pinned toolchain and the runners live.
fn toolchain() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/toolchain")
}

/// Whether a missing Node toolchain is a failure rather than a skip.
fn required() -> bool {
    std::env::var_os("CI").is_some_and(|value| !value.is_empty())
}

/// The installed toolchain, or `None` when Node is absent and this is not CI.
///
/// The install runs once per test binary. `npm ci` rather than `npm install`:
/// it installs exactly the committed lockfile and never rewrites it, so a run of
/// the suite cannot quietly change what the next one checks against.
fn installed() -> Option<&'static Path> {
    static TOOLCHAIN: OnceLock<Option<PathBuf>> = OnceLock::new();
    TOOLCHAIN
        .get_or_init(|| {
            if !runs("node") || !runs("npm") {
                assert!(
                    !required(),
                    "`node` and `npm` are required: the generated-code gates are what make \
                     `cargo test` mean the emitted TypeScript compiles and runs (CLAUDE.md). \
                     CI installs them; see .github/workflows/ci.yml."
                );
                eprintln!(
                    "warning: skipping the generated-code gates — `node`/`npm` are not on PATH. \
                     They are required in CI (`CI` is set there) and this run is not CI."
                );
                return None;
            }
            let root = toolchain();
            let install = Command::new("npm")
                .args(["ci", "--no-audit", "--no-fund"])
                .current_dir(&root)
                .output()
                .expect("npm runs");
            assert!(
                install.status.success(),
                "the pinned toolchain did not install:\n{}\n{}",
                String::from_utf8_lossy(&install.stdout),
                String::from_utf8_lossy(&install.stderr),
            );
            Some(root)
        })
        .as_deref()
}

/// Whether a command is on `PATH` and answers `--version`.
fn runs(program: &str) -> bool {
    Command::new(program)
        .arg("--version")
        .output()
        .is_ok_and(|output| output.status.success())
}

/// Copy one golden into the toolchain's scratch area and answer where it landed.
///
/// `purpose` names the gate the copy is for, because cargo runs the tests in one
/// binary on parallel threads and two of them re-staging one directory would
/// delete files out from under each other.
fn staged(golden: &Golden, root: &Path, purpose: &str) -> PathBuf {
    let destination = root.join("projects").join(purpose).join(golden.directory);
    let _ = fs::remove_dir_all(&destination);
    let source = goldens_root().join(golden.directory);
    for relative in files_under(&source) {
        let target = destination.join(relative.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(target.parent().expect("a staged path has a parent"))
            .expect("the scratch area is writable");
        fs::copy(source.join(&relative), &target).expect("a golden file is copyable");
    }
    destination
}

/// Gate 1: every golden project type-checks against the installed pins.
#[test]
fn every_generated_project_type_checks_under_the_pinned_toolchain() {
    let Some(root) = installed() else {
        return;
    };
    let tsc = root.join("node_modules/.bin/tsc");
    assert!(
        tsc.is_file(),
        "the pinned TypeScript did not install: {}",
        tsc.display()
    );

    for golden in GOLDENS {
        let project = staged(golden, root, "typecheck");
        let output = Command::new(&tsc)
            .args(["--noEmit", "-p", "."])
            .current_dir(&project)
            .output()
            .expect("tsc runs");
        assert!(
            output.status.success(),
            "`{}` does not type-check:\n{}\n{}",
            golden.directory,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr),
        );
    }
}

/// Gate 2: every golden project constructs its graph, and its state model holds
/// exactly the channels the composition declares plus the implicit history one.
#[test]
fn every_generated_project_constructs_its_state_model() {
    let Some(root) = installed() else {
        return;
    };

    for golden in GOLDENS {
        let project = staged(golden, root, "construct");
        let output = Command::new("node")
            .arg(root.join("state-channels.mjs"))
            .arg(&project)
            .output()
            .expect("node runs");
        assert!(
            output.status.success(),
            "`{}` does not construct its graph:\n{}",
            golden.directory,
            String::from_utf8_lossy(&output.stderr),
        );
        let channels: Vec<String> = serde_json::from_slice(&output.stdout)
            .expect("the runner prints the channel names as JSON");

        let ir = artifact(golden);
        let mut declared: Vec<String> = ir
            .state
            .as_ref()
            .map(|state| state.entries.keys().cloned().collect())
            .unwrap_or_default();
        // Grammar 10.4: `messages` is implicit, never declared, and always there.
        declared.push("messages".to_string());
        assert_eq!(
            channels, declared,
            "`{}`'s state model is not the composition's channel set",
            golden.directory
        );
    }
}

/// One case of the schema-lowering corpus.
#[derive(serde::Deserialize)]
struct Case {
    golden: String,
    surface: String,
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    about: String,
    documents: Vec<Document>,
}

/// One document, and the verdict both lowerings must reach on it.
#[derive(serde::Deserialize)]
struct Document {
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    why: String,
    valid: bool,
    document: Value,
}

fn corpus() -> Vec<Case> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema-lowering/cases.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not a corpus: {error}", path.display()))
}

/// Gate 3: the two columns of grammar 3.8's table accept the same documents.
#[test]
fn the_emitted_zod_agrees_with_the_json_schema_lowering() {
    let cases = corpus();
    assert!(!cases.is_empty(), "the corpus is empty");

    // The Rust column first: it needs no toolchain, so a divergence in the
    // lowering itself is reported even on a machine with no Node.
    let mut expected: Vec<Vec<bool>> = Vec::new();
    for case in &cases {
        let golden = goldens::golden(&case.golden);
        let ir = artifact(golden);
        let surfaces = compose_core::codegen::schema::surfaces(&ir);
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == case.surface)
            .unwrap_or_else(|| panic!("`{}` declares no `{}`", case.golden, case.surface));
        let schema = match surface.body {
            compose_core::codegen::schema::Body::Fields(fields) => {
                compose_core::codegen::schema::json_field_map(fields)
            }
            compose_core::codegen::schema::Body::Type(ty) => {
                compose_core::codegen::schema::json_type_node(ty)
            }
        };
        // `format` is an annotation by default in draft 2020-12; the DSL means it
        // as a constraint (grammar 3.3 lists a closed vocabulary of them), and the
        // emitted Zod enforces it, so the JSON column is asked to as well.
        let validator = jsonschema::options()
            .should_validate_formats(true)
            .build(&schema)
            .unwrap_or_else(|error| {
                panic!(
                    "`{}` does not lower to a valid schema: {error}",
                    case.surface
                )
            });

        let mut verdicts = Vec::new();
        for document in &case.documents {
            let accepted = validator.is_valid(&document.document);
            assert_eq!(
                accepted,
                document.valid,
                "the JSON Schema lowering of `{}` {} `{}`",
                case.surface,
                if accepted { "accepts" } else { "rejects" },
                document.document
            );
            verdicts.push(accepted);
        }
        expected.push(verdicts);
    }

    let Some(root) = installed() else {
        return;
    };

    // The Zod column, one Node run per golden the corpus reaches.
    let mut checked = 0usize;
    for directory in cases
        .iter()
        .map(|case| case.golden.as_str())
        .collect::<BTreeSet<_>>()
    {
        let golden = goldens::golden(directory);
        let project = staged(golden, root, "schema-lowering");
        let indices: Vec<usize> = cases
            .iter()
            .enumerate()
            .filter(|(_, case)| case.golden == directory)
            .map(|(index, _)| index)
            .collect();

        let names = compose_core::codegen::names::Names::of(&artifact(golden));
        let input: Vec<Value> = indices
            .iter()
            .map(|index| {
                let case = &cases[*index];
                serde_json::json!({
                    "export": names.value(&case.surface),
                    "documents": case
                        .documents
                        .iter()
                        .map(|document| document.document.clone())
                        .collect::<Vec<_>>(),
                })
            })
            .collect();
        let input_path = project.join("schema-lowering-cases.json");
        fs::write(
            &input_path,
            serde_json::to_string(&input).expect("the corpus serializes"),
        )
        .expect("the scratch area is writable");

        let output = Command::new("node")
            .arg(root.join("zod-conformance.mjs"))
            .arg(&input_path)
            .arg(&project)
            .output()
            .expect("node runs");
        assert!(
            output.status.success(),
            "the Zod corpus did not run against `{directory}`:\n{}",
            String::from_utf8_lossy(&output.stderr),
        );
        let verdicts: Vec<Vec<bool>> = serde_json::from_slice(&output.stdout)
            .expect("the runner prints one array of verdicts per case");
        assert_eq!(verdicts.len(), indices.len());

        for (verdict, index) in verdicts.into_iter().zip(indices) {
            let case = &cases[index];
            for (accepted, document) in verdict.into_iter().zip(&case.documents) {
                assert_eq!(
                    accepted,
                    document.valid,
                    "the emitted Zod for `{}` {} `{}`, and the JSON Schema lowering does not — \
                     grammar 3.8's two columns have drifted",
                    case.surface,
                    if accepted { "accepts" } else { "rejects" },
                    document.document
                );
                checked += 1;
            }
        }
    }
    assert_eq!(
        checked,
        expected.iter().map(Vec::len).sum::<usize>(),
        "not every document reached both columns"
    );
}

/// The fixture and the emitter pin the same versions.
///
/// Without this the gates would keep passing against whatever was installed the
/// last time someone touched the fixture, while `build` emitted a manifest for
/// something else — a green suite about the wrong toolchain.
#[test]
fn the_toolchain_fixture_pins_what_the_emitter_pins() {
    let path = toolchain().join("package.json");
    let text = fs::read_to_string(&path).expect("the toolchain fixture is readable");
    let manifest: Value = serde_json::from_str(&text).expect("the fixture is JSON");

    for (section, pins) in [
        ("dependencies", compose_core::codegen::project::PINS),
        ("devDependencies", compose_core::codegen::project::DEV_PINS),
    ] {
        let block = manifest[section]
            .as_object()
            .unwrap_or_else(|| panic!("the fixture declares no `{section}`"));
        assert_eq!(
            block.len(),
            pins.len(),
            "`{section}` in the fixture and in the emitter differ in size"
        );
        for (package, version) in pins {
            assert_eq!(
                block.get(*package).and_then(Value::as_str),
                Some(*version),
                "the fixture does not pin `{package}` at the version the emitter does"
            );
        }
    }

    // The lockfile is what `npm ci` installs; a fixture whose lockfile predates a
    // pin bump would install the old version and the gates would check it.
    let lock = fs::read_to_string(toolchain().join("package-lock.json"))
        .expect("the toolchain lockfile is committed");
    let lock: Value = serde_json::from_str(&lock).expect("the lockfile is JSON");
    let root = &lock["packages"][""];
    for (section, pins) in [
        ("dependencies", compose_core::codegen::project::PINS),
        ("devDependencies", compose_core::codegen::project::DEV_PINS),
    ] {
        for (package, version) in pins {
            assert_eq!(
                root[section][*package].as_str(),
                Some(*version),
                "the committed lockfile does not pin `{package}` at `{version}`; \
                 run `npm install --package-lock-only` in `tests/toolchain`"
            );
        }
    }
}

/// The staged copy is the golden, byte for byte.
///
/// The gates check what they copied, so a copy that lost or rewrote a file would
/// be checking something the compiler never emitted.
#[test]
fn the_staged_copy_is_the_golden_it_came_from() {
    let Some(root) = installed() else {
        return;
    };
    for golden in GOLDENS {
        let project = staged(golden, root, "staging");
        let emitted = emitted(golden);
        assert_eq!(
            files_under(&project),
            emitted.paths().map(str::to_string).collect::<Vec<_>>()
        );
        for file in emitted.files() {
            let staged = fs::read_to_string(
                project.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR)),
            )
            .expect("a staged file is readable");
            assert_eq!(staged, file.contents, "`{}` was not copied", file.path);
        }
    }
}
