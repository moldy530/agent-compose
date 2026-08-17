//! The generated-code checks of CLAUDE.md's *Validation strategy*, run against
//! the **real** pinned JavaScript toolchain.
//!
//! Five gates, in increasing strength. Each one exists because the one above it
//! passes on code the one below it catches:
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
//! 3. **Reduction** — the same graph is *invoked*. Names say a channel exists;
//!    only running it says the reducer appends rather than prepends and that a
//!    `default:` became an initial value. A wrong reducer type-checks, constructs,
//!    and would be committed as a correct golden, so [`REDUCTIONS`] states what
//!    every golden's state holds after two rounds of writes.
//! 4. **The environment check** — `node src/index.ts`, once with the
//!    composition's `${ENV}` variables removed and once with them supplied. The
//!    emitted `README.md` says loading the project is what checks its
//!    environment (PRD 5.9); a `readEnvironment` that was declared and never
//!    called would pass every gate above and make that sentence false.
//! 5. **Schema-lowering agreement** — the corpus under
//!    `tests/fixtures/schema-lowering/` is validated twice: against the JSON
//!    Schema this compiler lowers to (Rust, the `jsonschema` crate) and against
//!    the Zod the same module emits (Node). Grammar 3.8 is one table with two
//!    columns and this is what keeps them from drifting apart. Each document
//!    carries the verdict it *should* get, so two implementations agreeing on a
//!    wrong answer is still a failure — and where the two columns cannot agree,
//!    the document carries **both** verdicts and names one of [`DIVERGENCES`],
//!    so a difference is something a reader signed off on rather than something
//!    a thin corpus failed to notice.
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

/// What one golden's state model must *do*: two rounds of writes, and the state
/// they leave behind.
///
/// Gate 2 compares channel names, which a wrong reducer and a missing initial
/// value both survive. This is the table that does not: every entry is a value
/// the composition's own `reduce:` policy and `default:` decide (grammar 7.6.4,
/// 10.1, 10.2), written out so that changing the emitted reducer changes a
/// verdict here rather than a golden nobody re-derives.
///
/// Every golden has a row. A composition with nothing reduced still pins its
/// defaults, and pins that an unreduced channel with no `default:` starts
/// **unset** rather than `null` (Decision D78) — which is what makes a later
/// read of it fail naming the channel.
struct Reduction {
    /// The golden this describes, by directory.
    golden: &'static str,
    /// One object per node, in the order the nodes run.
    writes: &'static str,
    /// The whole state at quiescence, `messages` included.
    expected: &'static str,
}

const REDUCTIONS: &[Reduction] = &[
    Reduction {
        golden: "every-schema-form",
        // `seen` is deliberately never written: its `default: { count: 0 }` is
        // the only thing that can produce it. Neither is `draft`, which has no
        // default and must therefore be absent rather than null.
        writes: r#"[
            { "notes": "a", "totals": { "fixed": 1 }, "latest": "one" },
            { "notes": "b", "totals": { "skipped": 2 }, "latest": "two" }
        ]"#,
        expected: r#"{
            "notes": ["a", "b"],
            "totals": { "fixed": 1, "skipped": 2 },
            "latest": "two",
            "seen": { "count": 0 },
            "round": 1,
            "mood": "calm",
            "urgent": false,
            "messages": []
        }"#,
    },
    Reduction {
        golden: "review-loop",
        writes: r#"[
            { "draft": "first" },
            { "draft": "second", "feedback": "tighten it" }
        ]"#,
        expected: r#"{
            "draft": "second",
            "feedback": "tighten it",
            "messages": []
        }"#,
    },
    Reduction {
        golden: "triage-fanout",
        writes: r#"[
            { "patches": "one", "matches": ["a", "b"], "report_normalized": "r" },
            { "patches": "two" }
        ]"#,
        expected: r#"{
            "patches": ["one", "two"],
            "matches": ["a", "b"],
            "report_normalized": "r",
            "human_decision": "approve",
            "summary": "",
            "messages": []
        }"#,
    },
    Reduction {
        // The deploy layer forks per target and the state model does not, so the
        // same values are the answer under `staging` — which is the claim, not a
        // duplicate.
        golden: "triage-fanout-staging",
        writes: r#"[
            { "patches": "one", "matches": ["a", "b"], "report_normalized": "r" },
            { "patches": "two" }
        ]"#,
        expected: r#"{
            "patches": ["one", "two"],
            "matches": ["a", "b"],
            "report_normalized": "r",
            "human_decision": "approve",
            "summary": "",
            "messages": []
        }"#,
    },
];

/// One case of the schema-lowering corpus.
#[derive(serde::Deserialize)]
struct Case {
    golden: String,
    surface: String,
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    about: String,
    documents: Vec<Document>,
}

/// One document, and the verdict each lowering must reach on it.
///
/// `valid` is the verdict, and it is both columns' unless `zod` overrides it.
/// That shape is the point: agreement is the default a case does not have to say
/// anything about, and a difference is a **declaration** — it needs the other
/// verdict written down and a `divergence` naming one of [`DIVERGENCES`], so no
/// difference between grammar 3.8's two columns can exist without a reader
/// having agreed to it.
#[derive(serde::Deserialize)]
struct Document {
    #[expect(dead_code, reason = "documentation for a reader of the corpus")]
    why: String,
    valid: bool,
    #[serde(default)]
    zod: Option<bool>,
    #[serde(default)]
    divergence: Option<String>,
    document: Value,
}

impl Document {
    /// The verdict the emitted Zod must reach.
    fn zod_verdict(&self) -> bool {
        self.zod.unwrap_or(self.valid)
    }
}

/// Every difference between the two columns that this compiler has decided to
/// keep, by the id a corpus document cites.
///
/// The list is `codegen::schema`'s own divergence table, transcribed: the module
/// argues each one and this decides whether the argument is still true. A
/// divergence with no document exercising it is as much a failure as a document
/// citing a divergence that is not here — the first is a claim nothing checks,
/// the second is a difference nobody agreed to.
const DIVERGENCES: &[&str] = &[
    "integer-beyond-the-safe-range",
    "display-name-is-not-an-addr-spec",
    "leap-second-away-from-midnight",
    "duration-skips-a-designator",
    "punycode-payload-undecoded",
];

fn corpus() -> Vec<Case> {
    let path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/schema-lowering/cases.json");
    let text = fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|error| panic!("{} is not a corpus: {error}", path.display()))
}

/// Gate 2b: the emitted reducers and initial values do what the policies say.
///
/// The graph is real, the reducers are the emitted ones, and the comparison is
/// over the whole state object — so a channel that reduced correctly and one that
/// quietly acquired a value are both visible. `messages` is in the expectation
/// for the same reason: the implicit channel of grammar 10.4 starts empty, and
/// nothing here writes to it.
#[test]
fn the_emitted_state_model_reduces_the_way_its_policies_say() {
    let named: BTreeSet<&str> = REDUCTIONS.iter().map(|entry| entry.golden).collect();
    let goldens: BTreeSet<&str> = GOLDENS.iter().map(|golden| golden.directory).collect();
    assert_eq!(
        named, goldens,
        "every golden states what its state model does with two rounds of writes"
    );

    let Some(root) = installed() else {
        return;
    };

    for entry in REDUCTIONS {
        let golden = goldens::golden(entry.golden);
        let project = staged(golden, root, "reduce");
        let writes = project.join("state-writes.json");
        fs::write(&writes, entry.writes).expect("the scratch area is writable");

        let output = Command::new("node")
            .arg(root.join("state-reduction.mjs"))
            .arg(&project)
            .arg(&writes)
            .output()
            .expect("node runs");
        assert!(
            output.status.success(),
            "`{}` did not run its state model:\n{}",
            entry.golden,
            String::from_utf8_lossy(&output.stderr),
        );
        let answer: Value =
            serde_json::from_slice(&output.stdout).expect("the runner prints the state as JSON");
        let expected: Value = serde_json::from_str(entry.expected).expect("the row is JSON");
        assert_eq!(
            answer, expected,
            "`{}`'s state model does not reduce the way its `reduce:` policies and `default:`s \
             say",
            entry.golden
        );
    }
}

/// Gate 2c: loading the project is what checks its environment (PRD 5.9).
///
/// `src/env.ts` says a presence check exists and the emitted `README.md` says
/// when it runs; both are claims about a call, and a module that declared
/// `readEnvironment` and never called it would satisfy `tsc`, construct its
/// graph, and leave both sentences false. So the gate is the command the README
/// names: `node src/index.ts`, once with the composition's variables removed and
/// once with them set.
#[test]
fn the_generated_project_checks_its_environment_when_it_is_loaded() {
    let Some(root) = installed() else {
        return;
    };

    let mut checked = 0usize;
    for golden in GOLDENS {
        let project = staged(golden, root, "environment");
        let ir = artifact(golden);
        let references = compose_core::codegen::env::References::of(&ir);
        let names: Vec<&str> = references.names().collect();

        let mut sealed = Command::new("node");
        sealed.arg(project.join("src/index.ts"));
        for name in &names {
            sealed.env_remove(name);
        }
        let sealed = sealed.output().expect("node runs");

        if names.is_empty() {
            assert!(
                sealed.status.success(),
                "`{}` references no variable and still refused to load:\n{}",
                golden.directory,
                String::from_utf8_lossy(&sealed.stderr),
            );
            continue;
        }

        assert!(
            !sealed.status.success(),
            "`{}` loaded with {:?} unset; the presence check did not run",
            golden.directory,
            names,
        );
        let complaint = String::from_utf8_lossy(&sealed.stderr);
        for name in &names {
            assert!(
                complaint.contains(name),
                "`{}` did not name the missing `{name}`:\n{complaint}",
                golden.directory,
            );
            assert!(
                references
                    .sites(name)
                    .iter()
                    .all(|site| complaint.contains(site)),
                "`{}` named `{name}` without saying where it is referenced:\n{complaint}",
                golden.directory,
            );
        }

        let mut supplied = Command::new("node");
        supplied.arg(project.join("src/index.ts"));
        for name in &names {
            supplied.env(name, "supplied");
        }
        let supplied = supplied.output().expect("node runs");
        assert!(
            supplied.status.success(),
            "`{}` refused to load with every variable set:\n{}",
            golden.directory,
            String::from_utf8_lossy(&supplied.stderr),
        );
        checked += 1;
    }
    assert!(
        checked > 0,
        "no golden references an environment variable, so nothing exercised the check"
    );
}

/// Every declared divergence is exercised, and every cited one is declared.
///
/// This runs without a toolchain on purpose: it is about the corpus and the
/// module's own table, not about what either column answers, so a machine with
/// no Node still fails when a divergence is invented or abandoned.
#[test]
fn every_divergence_between_the_two_columns_is_declared_and_exercised() {
    let cases = corpus();
    let mut cited: BTreeSet<&str> = BTreeSet::new();
    for case in &cases {
        for document in &case.documents {
            match (document.zod, &document.divergence) {
                (None, None) => {}
                (Some(zod), Some(divergence)) => {
                    assert_ne!(
                        zod, document.valid,
                        "`{}` declares a divergence on `{}` and then gives both columns the same \
                         verdict",
                        case.surface, document.document
                    );
                    assert!(
                        DIVERGENCES.contains(&divergence.as_str()),
                        "`{}` cites `{divergence}`, which `codegen::schema`'s divergence table \
                         does not declare",
                        case.surface
                    );
                    cited.insert(divergence.as_str());
                }
                (Some(_), None) => panic!(
                    "`{}` gives the two columns different verdicts on `{}` without naming a \
                     divergence",
                    case.surface, document.document
                ),
                (None, Some(divergence)) => panic!(
                    "`{}` names the divergence `{divergence}` on `{}` without saying what the Zod \
                     column answers",
                    case.surface, document.document
                ),
            }
        }
    }

    let declared: BTreeSet<&str> = DIVERGENCES.iter().copied().collect();
    assert_eq!(
        declared.len(),
        DIVERGENCES.len(),
        "a divergence is declared twice"
    );
    assert_eq!(
        declared.difference(&cited).collect::<Vec<_>>(),
        Vec::<&&str>::new(),
        "a divergence is declared and no document exercises it, so nothing says it is still true"
    );
}

/// Gate 3: each column of grammar 3.8's table answers what the corpus says it
/// does — and where they differ, that the difference is the declared one.
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
                    document.zod_verdict(),
                    "the emitted Zod for `{}` {} `{}`; the corpus says it {}{} — grammar 3.8's \
                     two columns have drifted",
                    case.surface,
                    if accepted { "accepts" } else { "rejects" },
                    document.document,
                    if document.zod_verdict() {
                        "accepts"
                    } else {
                        "rejects"
                    },
                    document.divergence.as_ref().map_or_else(
                        || ", as the JSON Schema lowering does".to_string(),
                        |divergence| format!(", diverging from the JSON column as `{divergence}`"),
                    )
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
