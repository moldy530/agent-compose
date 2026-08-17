//! The generated-code checks of CLAUDE.md's *Validation strategy*, run against
//! the **real** pinned JavaScript toolchain.
//!
//! Seven gates. The first four are in increasing strength, each one existing
//! because the one above it passes on code the one below it catches; the next two
//! are about the schemas rather than the graph; the last is about a composition
//! that has no generated project at all:
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
//! 6. **What a provider would be handed** — `@langchain/core`'s own converter is
//!    run over the emitted schemas, because PRD 5.2 delivers an output schema
//!    through `withStructuredOutput` and that mechanism sends JSON Schema rather
//!    than the Zod it was given. The conversion drops every check spelled
//!    `.refine`, so the contract a model is constrained by is weaker than the
//!    parse that follows it — see `codegen::schema`'s *What a provider is handed*.
//!    Gate 5 is what makes the two columns agree; this is what says which of them
//!    a model actually sees.
//! 7. **The refusal's evidence** — `compose_core::codegen::diagnostics` refuses to
//!    build a composition whose `state:` names a channel after a property every
//!    JavaScript object carries (`constructor`). That refusal is an
//!    over-refusal until something shows the runtime really cannot take the
//!    name, and no golden can show it, because the compiler will not emit one.
//!    So this gate builds the channel table itself and makes LangGraph fail on
//!    it — and asks Node for `Object.getOwnPropertyNames(Object.prototype)`, so
//!    the Rust-side list is checked against the object model rather than against
//!    a memory of it.
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
    #[serde(default)]
    as_written: Option<String>,
}

impl Document {
    /// The verdict the emitted Zod must reach.
    fn zod_verdict(&self) -> bool {
        self.zod.unwrap_or(self.valid)
    }

    /// The exact JSON text the Zod column is handed.
    ///
    /// `serde_json` reads an object into a `BTreeMap`, so a document written in
    /// the corpus arrives here with its keys **sorted** and the order they were
    /// written in is gone. Object key order is not supposed to matter — JSON
    /// Schema compares instances — but "not supposed to" is the thing a corpus
    /// exists to check, and it cannot check it through a representation that
    /// has already normalized it. So a case that is about key order writes the
    /// document out a second time as `as_written`, which is passed through
    /// verbatim for `JSON.parse` to read in that order.
    ///
    /// # Panics
    ///
    /// Panics if `as_written` is not JSON, or is JSON denoting a different
    /// value than `document` — the two are one document and a case where they
    /// disagree would test something nobody wrote down.
    fn text(&self) -> String {
        let Some(written) = &self.as_written else {
            return serde_json::to_string(&self.document).expect("a document serializes");
        };
        let parsed: Value = serde_json::from_str(written)
            .unwrap_or_else(|error| panic!("`as_written` is not JSON: {written} ({error})"));
        assert_eq!(
            parsed, self.document,
            "`as_written` and `document` are the same document written twice"
        );
        written.clone()
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
    "omitted-default-is-the-same-item",
    "dot-matches-a-code-unit",
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

/// Gate 7: the channel names the compiler refuses are names LangGraph refuses.
///
/// `codegen::state::INHERITED_PROPERTY_NAMES` is why `agent-compose build`
/// rejects a composition declaring `state:\n  constructor: …`, which the
/// validator accepts and which produces a project that type-checks, loads, and
/// dies at `new StateGraph(State)`. Every other gate here runs over a golden;
/// this one cannot, because the emitter refuses to produce the project — so the
/// channel table is built directly and LangGraph is asked.
///
/// Two claims, both of them things that could quietly stop being true:
///
/// * the list is the object model's own — `Object.getOwnPropertyNames(
///   Object.prototype)` under the pinned Node, not a transcription of it;
/// * every name on it really breaks construction under the pinned LangGraph, and
///   two ordinary names do not. A release that fixed the lookup would fail here,
///   which is the signal to drop the refusal rather than keep it out of habit.
#[test]
fn a_channel_named_after_an_inherited_property_cannot_be_built_at_all() {
    let Some(root) = installed() else {
        return;
    };

    let refused = compose_core::codegen::state::INHERITED_PROPERTY_NAMES;
    // Two names a check matching on shape rather than on membership would take
    // with it: one ordinary channel name, and the near-miss the fixtures use.
    let controls = ["draft", "constructors"];
    let probed: Vec<&str> = refused.iter().copied().chain(controls).collect();

    let output = Command::new("node")
        .arg(root.join("inherited-channel-names.mjs"))
        .arg(serde_json::to_string(&probed).expect("the names serialize"))
        .output()
        .expect("node runs");
    assert!(
        output.status.success(),
        "the inherited-name runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let answer: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object");

    let inherited: Vec<&str> = answer["inherited"]
        .as_array()
        .expect("`inherited` is an array")
        .iter()
        .map(|name| name.as_str().expect("a name is a string"))
        .collect();
    assert_eq!(
        inherited, refused,
        "`INHERITED_PROPERTY_NAMES` is not `Object.getOwnPropertyNames(Object.prototype)` under \
         the pinned Node; the refusal is over or under what the object model actually carries"
    );

    let rejected: Vec<&str> = answer["rejected"]
        .as_array()
        .expect("`rejected` is an array")
        .iter()
        .map(|name| name.as_str().expect("a name is a string"))
        .collect();
    assert_eq!(
        rejected, refused,
        "the names `build` refuses are not the names LangGraph refuses; the controls {controls:?} \
         must construct and every inherited name must not"
    );
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

/// The corpus's key-order mechanism is used, and says the same thing twice.
///
/// [`Document::text`] asserts that a document's two spellings denote one value,
/// so walking the corpus is what checks it; and a mechanism no case uses is one
/// that could stop working without a failure, so at least one document has to
/// carry `as_written`. Today that is `state.authors`' key-order pair, which is
/// the only place object key order is the subject rather than an accident.
///
/// Runs without a toolchain: it is about the corpus, not about either column.
#[test]
fn a_document_written_out_twice_says_the_same_thing_both_times() {
    let cases = corpus();
    let mut written = 0usize;
    for case in &cases {
        for document in &case.documents {
            let _ = document.text();
            if document.as_written.is_some() {
                written += 1;
            }
        }
    }
    assert!(
        written > 0,
        "no case writes a document out as text, so nothing pins a rule about key order"
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
        let schema = match &surface.body {
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
                    // JSON *text* rather than values: the runner parses each one
                    // itself, so the key order a case wrote survives to the Zod
                    // column. See `Document::text`.
                    "documents": case
                        .documents
                        .iter()
                        .map(Document::text)
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

/// One surface whose emitted Zod says more than the schema a provider would be
/// handed for it.
struct Weakening {
    /// The surface, by canonical path — all of them from `every-schema-form`,
    /// which is the golden that reaches every spelling.
    surface: &'static str,
    /// The JSON Schema keyword this compiler's own lowering writes and the
    /// conversion of the emitted Zod does not.
    keyword: &'static str,
    /// What the check is, for the failure message.
    about: &'static str,
}

/// What `withStructuredOutput` would lose, one row per spelling that loses
/// something.
///
/// Zod models a `.refine` as an opaque predicate, and a JSON Schema conversion
/// has nowhere to put it — so every check `codegen::schema` writes out rather
/// than borrowing from a constructor disappears on the way to the model. The
/// rows are the three shapes that happens in, plus one reached through a nested
/// property, because "only the top level is affected" would be a comforting and
/// wrong reading of the first three.
const WEAKENINGS: &[Weakening] = &[
    Weakening {
        surface: "state.at",
        keyword: "format",
        about: "`format: date-time`, written out as `refine(rfc3339DateTime, …)`",
    },
    Weakening {
        surface: "state.mark",
        keyword: "maxLength",
        about: "`max_length`, written out over a code-point count",
    },
    Weakening {
        surface: "state.tags",
        keyword: "uniqueItems",
        about: "`unique_items`, which Zod has no built-in for",
    },
    Weakening {
        surface: "agent.shaper.output",
        keyword: "format",
        about: "`format: email` on a nested property, two levels in",
    },
];

/// Gate 4: what PRD 5.2's delivery mechanism would hand a provider, and what it
/// drops on the way.
///
/// `withStructuredOutput` does not send Zod to anyone: `@langchain/core` converts
/// it to JSON Schema first, and that conversion keeps what Zod models as a check
/// and drops what it models as a refinement. Six of the ten formats, both length
/// bounds and `unique_items` are refinements, so the contract a model is
/// constrained by is strictly weaker than the parse its answer then faces —
/// which is the failure `codegen::schema` was written to prevent, pointed the
/// other way.
///
/// This gate does not fix that; the schema a node fn hands over is the next PR's
/// emission, and *which* schema (a conversion of this Zod, or the lowering this
/// compiler already publishes and the corpus proves equal to the parse) is a
/// design question PRD 5.2 does not answer. What it does is keep the evidence
/// current: the conversion is the library's own, the schemas are the committed
/// goldens, and the day either half stops being true this fails and the question
/// can be answered differently.
#[test]
fn what_the_structured_output_mechanism_would_be_handed() {
    let golden = goldens::golden("every-schema-form");
    let ir = artifact(golden);
    let surfaces = compose_core::codegen::schema::surfaces(&ir);
    let names = compose_core::codegen::names::Names::of(&ir);

    // The compiler's own lowering first: it needs no toolchain, and a row whose
    // keyword this column does not even write would be checking nothing.
    for row in WEAKENINGS {
        let surface = surfaces
            .iter()
            .find(|surface| surface.path == row.surface)
            .unwrap_or_else(|| panic!("`every-schema-form` declares no `{}`", row.surface));
        let published = match &surface.body {
            compose_core::codegen::schema::Body::Fields(fields) => {
                compose_core::codegen::schema::json_field_map(fields)
            }
            compose_core::codegen::schema::Body::Type(ty) => {
                compose_core::codegen::schema::json_type_node(ty)
            }
        };
        assert!(
            mentions(&published, row.keyword),
            "this compiler's lowering of `{}` writes no `{}`, so the row says nothing about {}",
            row.surface,
            row.keyword,
            row.about,
        );
    }

    let Some(root) = installed() else {
        return;
    };
    let project = staged(golden, root, "structured-output");
    let mut runner = Command::new("node");
    runner
        .arg(root.join("structured-output-schema.mjs"))
        .arg(&project);
    for row in WEAKENINGS {
        runner.arg(names.value(row.surface));
    }
    let output = runner.output().expect("node runs");
    assert!(
        output.status.success(),
        "the emitted schemas did not convert:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    let converted: Value =
        serde_json::from_slice(&output.stdout).expect("the runner prints one schema per export");

    for row in WEAKENINGS {
        let handed = &converted[names.value(row.surface)];
        assert!(
            !mentions(handed, row.keyword),
            "`{}` reaches a provider carrying `{}` after all: {} survived the conversion, so the \
             emitted Zod and the schema a model is constrained by no longer differ here. That is \
             the premise of `codegen::schema`'s *What a provider is handed* — re-read it before \
             deleting this row.",
            row.surface,
            row.keyword,
            row.about,
        );
    }
}

/// Whether a JSON Schema writes this keyword anywhere inside it.
fn mentions(schema: &Value, keyword: &str) -> bool {
    match schema {
        Value::Object(map) => {
            map.contains_key(keyword) || map.values().any(|value| mentions(value, keyword))
        }
        Value::Array(items) => items.iter().any(|item| mentions(item, keyword)),
        _ => false,
    }
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
