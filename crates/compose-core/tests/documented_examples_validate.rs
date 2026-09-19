//! Every spec this compiler *hands out* is run through this compiler.
//!
//! The discovery surface (PRD §7 M2, resolved q23) ships YAML that a reader is
//! meant to copy: the `init` scaffold, the examples inside the topic documents,
//! and the spec each diagnostic explanation shows to demonstrate its code. A
//! published example that does not do what it claims is worse than a missing
//! one — it teaches a shape the compiler rejects, or names a code it does not
//! raise — and the reader has no way to tell. So each one is run here, by the
//! same passes `agent-compose validate` runs.
//!
//! # The marker convention
//!
//! A document needs both kinds of block: a whole spec a reader could save as
//! `main.yml`, and a fragment showing one key in isolation. They are told apart
//! by the fenced block's **info string**, which is the only place a Markdown
//! fence carries metadata:
//!
//! * ```` ```yaml spec ```` — a complete spec. Written to `main.yml` in a
//!   scratch directory of its own and required to resolve and check **clean**
//!   under the built-in `local` target.
//! * ```` ```yaml triggers ```` — a complete spec that must **report** the code
//!   its explanation file is named for. Anything else it reports is fine; a
//!   minimal example of one failure often carries a second.
//! * ```` ```yaml triggers <code> ```` — the same, in a **topic**, where there
//!   is no file name to take the code from so the marker carries it. Topics are
//!   example-led and their examples are held clean, which is exactly the wrong
//!   discipline for the one thing a topic sometimes has to teach: what a
//!   **warning** looks like. A warning's spec builds and runs, so a topic that
//!   showed one as a bare fragment would be showing an unchecked block that
//!   looks identical to a checked one.
//! * an explanation may also carry a ```` ```yaml spec ```` block, which is its
//!   **repair** written out: the triggering spec with the fix applied, held to
//!   the same clean verdict a topic's example is. A fix stated only in prose is
//!   the one part of an explanation nothing runs, and a bullet naming a
//!   spelling the compiler refuses looks exactly like a bullet that works.
//! * ```` ```yaml deploy <target> ```` — a complete **deploy file**, which is
//!   the one document kind that is not a spec and cannot be checked on its own:
//!   it is written to `deploy/<target>.yml` beside the topic's `yaml spec`
//!   block and the pair is resolved under `--target <target>`. The marker
//!   carries the name because the target is what the file is *for*, and a
//!   deploy file checked under the wrong name is not checked at all.
//!
//!   An **explanation** may carry deploy blocks too, and there they do double
//!   duty, because a deploy-layer rule has no single-file demonstration at all:
//!   the explanation's `yaml triggers` block is the composition, its **first**
//!   `yaml deploy <t>` block is the deploy layer that triggers the code, and
//!   every **further** one is a written-out repair held to a clean verdict —
//!   the deploy-layer counterpart of the `yaml spec` correction a spec-level
//!   explanation carries. Each block needs its own target name, since each
//!   becomes `deploy/<target>.yml` in one scratch project.
//! * ```` ```yaml ```` — a fragment. Skipped, and deliberately so: a block
//!   showing `retry: { max: 2 }` on its own is not a document and has no
//!   verdict to have.
//!
//! The marker starts one word after the language because it has to survive a
//! Markdown renderer: every renderer takes the first token of an info string as
//! the language, so ```` ```yaml spec ```` still highlights as YAML wherever
//! these documents are read outside the binary.
//!
//! # What this cannot check
//!
//! A *fragment that should have been a spec*: an author who marks a whole
//! document `yaml` gets no verdict and no failure. That is the residual
//! `trace_format_inventory.rs` names about prose — a check over documents is
//! worth what the documents' own discipline is worth. What it does close is the
//! direction that ships a broken example, and for the explanations it closes a
//! second one: an example that stopped triggering its code, which is a lie
//! shipped inside a binary rather than a mistake in a file somebody might
//! reread.
//!
//! [`EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE`] and
//! [`TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE`] are what keep these checks honest. A
//! document with no marked block is checked by nothing, so "how many were
//! checked" is the wrong question to ask of a corpus this size — a floor is
//! satisfied by the documents that still have their examples while the ones
//! that lost theirs go unnoticed. Both lists are therefore held to **set
//! equality** against the documents that actually lack an example: every other
//! document MUST carry one, and opting out is an edit to this file that a
//! reviewer sees.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, docs};

/// The codes whose explanation carries no runnable example, and why.
///
/// Each is a failure one `main.yml` cannot express. Four need a **second file**
/// — the harness below writes exactly one — and the fifth needs a deploy
/// target, which is the one thing about a composition that is not in the
/// composition.
///
/// An entry here is a decision, not an exemption to reach for: it says the
/// failure class has no single-file demonstration, and the explanation says the
/// same thing in prose where a reader meets it.
const EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE: &[(&str, &str)] = &[
    (
        "invalid-encoding",
        "a property of the file's first bytes, not of anything a document can say",
    ),
    (
        "duplicate-definition",
        "two files declaring one address; two in one mapping is `duplicate-key`",
    ),
    (
        "duplicate-section",
        "two files declaring one singleton section; two in one mapping is `duplicate-key`",
    ),
    (
        "version-mismatch",
        "a disagreement between an entrypoint and an imported file",
    ),
    (
        "unsupported-detach",
        "target-dependent: it cannot fire under `local`, which is the target when none is named",
    ),
];

/// The topics whose document carries no complete spec, and why.
///
/// Both are reference rather than curriculum: their code blocks are command
/// lines and a run's output document, neither of which is a composition. Every
/// other topic is example-led — that is what the topics are *for* — so a topic
/// arriving here is a decision about what kind of document it is.
const TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE: &[(&str, &str)] = &[
    (
        "cli",
        "a reference for the verbs: its blocks are command lines, not compositions",
    ),
    (
        "trace",
        "orientation on what a run writes; its block is a trace document, not a spec",
    ),
];

/// The explanations that write their repair out as a spec that runs.
///
/// [`every_explanation_example_reports_its_code`] runs the failure, not the
/// cure, so a fix bullet naming a spelling the compiler refuses ships inside a
/// binary looking exactly like a bullet that works — which is how
/// `unkeyed-map-write` came to offer `key: "execution.item_index"`, a repair
/// that satisfies its own rule and then fails type-checking. An explanation
/// closes that by carrying the repaired spec as a ```` ```yaml spec ```` block.
///
/// A set equality rather than a floor, for the reason every other list in this
/// file is one: dropping the block would otherwise be a silent loss of the only
/// check the fix has.
///
/// The entries are the fixes with a *neighbour* to fail — a repair that clears
/// the rule it is about and lands the reader on a second diagnostic. Each was a
/// shipped one: `unkeyed-map-write` offered `key: "execution.item_index"`,
/// which type-checks as an integer where a string is required;
/// `undefined-channel` offered a channel declared as an array of strings for a
/// binding into a `string`; `missing-capability` offered a provider declaration
/// with nothing repointed at it, which leaves the code firing untouched;
/// `unbalanced-convergence` offered `else: true` on one edge of a pair that
/// carried no guard at all, which is `invalid-value` (D107) — an `else:` edge
/// requires a `when:`-guarded sibling, so that spelling makes a pair exclusive
/// only where a guard was already written; `invalid-path-expression` offered
/// `over: "plan.output.ready_tasks"`, a field the producer in its own example
/// does not declare, so the repointed map cleared the path rule and landed on
/// the next one. That last is the shape a bare fragment cannot carry at all:
/// the repair is two edits in two places, and a one-line `over:` is only ever
/// the second of them.
///
/// The last three are a different case and are here for a different reason: they
/// are **warnings**, so their triggering example builds. A reader who is told
/// "correct the spelling" or "drop the key" has no diagnostic to confirm they
/// did — the report simply goes quiet — so the repaired spec is the only thing
/// that says which edit produces the quiet report.
const EXPLANATIONS_WITH_A_CORRECTED_EXAMPLE: &[&str] = &[
    "conflicting-connection-variable",
    "invalid-path-expression",
    "missing-callback-allowlist",
    "missing-capability",
    "missing-credential",
    "reserved-harness-setting",
    "unbalanced-convergence",
    "undefined-channel",
    "unkeyed-map-write",
    "unknown-harness-setting",
    "unknown-server-tool",
    "unknown-server-tool-field",
    "unsupported-connection-fact",
    "unsupported-permission-mode",
    "unsupported-provider-kind",
    "unsupported-server-tools",
    "widening-permission-mode",
];

/// The explanations whose failure lives in the **deploy layer**, and the codes
/// they teach.
///
/// A deploy-layer rule cannot be demonstrated by one `main.yml`: the sections it
/// is about are illegal in a spec file, so the example needs a second document
/// the harness above writes for it. Rather than exempt these codes from carrying
/// a runnable example — which is what
/// [`EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE`] would have meant, and would have
/// left six checks with nothing running them — the explanation carries its
/// deploy file in the document, and both halves are run: the first block must
/// report the code, and every further block must clear it.
///
/// A **set equality**, like every list in this file: an explanation that lost
/// its deploy blocks would still read as a worked example and would be checked
/// by nothing.
const EXPLANATIONS_WITH_A_DEPLOY_EXAMPLE: &[&str] = &[
    "conflicting-placement",
    "conflicting-registry-credential",
    "missing-join-token",
    "missing-registry-token",
    "process-local-store",
    "unsupported-placement",
];

/// The topics that teach a deploy file, and must keep one that resolves.
///
/// A deploy file is the one document kind a reader cannot check on its own —
/// `agent-compose validate` takes a spec — so it is the one most likely to rot
/// unwatched, and the list exists so that dropping the marker is an edit here
/// rather than a silent loss.
const TOPICS_WITH_A_DEPLOY_EXAMPLE: &[&str] = &["targets"];

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A scratch directory of this test's own, emptied first.
fn scratch(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-documented-examples")
        .join(name);
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("can create a scratch directory");
    directory
}

/// Every fenced block in `document` whose info string is exactly
/// `yaml <marker>`.
///
/// Exactly, not "begins with": `yaml spec` and `yaml triggers` are two
/// conventions with two meanings, and a block marked one of them must not be
/// read as the other. A bare ```` ```yaml ```` matches neither and is skipped.
fn blocks(document: &str, marker: &str) -> Vec<String> {
    let opening = format!("```yaml {marker}");
    let mut found = Vec::new();
    let mut lines = document.lines();
    while let Some(line) = lines.next() {
        if line.trim_end() != opening {
            continue;
        }
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_end() == "```" {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        found.push(body);
    }
    found
}

/// Every fenced block whose info string is `yaml triggers <code>`, paired with
/// the code it must report.
///
/// The explanations' `yaml triggers` takes its code from the file name; a topic
/// has no such name, so the marker carries it — the same reason
/// [`deploy_blocks`] takes an argument.
fn topic_triggers_blocks(document: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lines = document.lines();
    while let Some(line) = lines.next() {
        let Some(code) = line.trim_end().strip_prefix("```yaml triggers ") else {
            continue;
        };
        let code = code.trim();
        assert!(
            !code.is_empty() && code.chars().all(|c| c.is_ascii_lowercase() || c == '-'),
            "a topic's `yaml triggers` block names the code it reports, found `{code}`"
        );
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_end() == "```" {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        found.push((code.to_string(), body));
    }
    found
}

/// Every fenced block whose info string is `yaml deploy <target>`, paired with
/// the target it is written for.
///
/// A separate reader from [`blocks`] because this marker carries an argument:
/// the target names the file the block becomes and the name it is resolved
/// under, and both have to come from the document rather than from a convention
/// a reader of the document cannot see.
fn deploy_blocks(document: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut lines = document.lines();
    while let Some(line) = lines.next() {
        let Some(target) = line.trim_end().strip_prefix("```yaml deploy ") else {
            continue;
        };
        let target = target.trim();
        assert!(
            !target.is_empty() && target.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
            "a `yaml deploy` block names the target it is for, found `{target}`"
        );
        let mut body = String::new();
        for line in lines.by_ref() {
            if line.trim_end() == "```" {
                break;
            }
            body.push_str(line);
            body.push('\n');
        }
        found.push((target.to_string(), body));
    }
    found
}

/// Resolve and check the composition in `directory` under `target`.
fn report_in(directory: &Path, target: &str) -> Vec<Diagnostic> {
    let entrypoint = directory.join("main.yml");
    let resolution = compose_core::resolve_with_target(&entrypoint, target);
    let mut diagnostics = resolution.diagnostics;
    if let Some(ir) = &resolution.ir {
        diagnostics.extend(compose_core::check(ir));
    }
    diagnostics
}

/// Resolve and check one complete spec written as `main.yml`.
fn report(name: &str, source: &str) -> Vec<Diagnostic> {
    let directory = scratch(name);
    fs::write(directory.join("main.yml"), source).expect("can write the spec");
    report_in(&directory, compose_core::DEFAULT_TARGET)
}

/// Write one composition and every deploy file a document declares, then
/// resolve it under `target`.
///
/// Every deploy block goes into the project, not just the one being checked, so
/// that the files a reader would copy sit beside each other exactly as they do
/// in the document. Only the named target is resolved: the others are files the
/// composition simply has.
fn report_with_deploy(
    name: &str,
    spec: &str,
    deploys: &[(String, String)],
    target: &str,
) -> Vec<Diagnostic> {
    let directory = scratch(name);
    fs::write(directory.join("main.yml"), spec).expect("can write the spec");
    fs::create_dir_all(directory.join("deploy")).expect("can create `deploy/`");
    for (declared, body) in deploys {
        fs::write(
            directory.join("deploy").join(format!("{declared}.yml")),
            body,
        )
        .expect("can write the deploy file");
    }
    report_in(&directory, target)
}

/// Resolve and check one complete spec, and require it to say nothing.
#[track_caller]
fn validates(name: &str, source: &str) {
    let diagnostics = report(name, source);
    assert!(
        diagnostics.is_empty(),
        "{name} does not validate:\n{}",
        render(&diagnostics)
    );
}

fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} at {}: {}",
                diagnostic.code, diagnostic.span, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// Every topic document, by name.
fn topics() -> Vec<(String, String)> {
    let directory = repository().join("docs/topics");
    let mut found: Vec<(String, String)> = fs::read_dir(&directory)
        .expect("the topics directory is readable")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "md"))
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            (name, fs::read_to_string(&path).expect("a readable topic"))
        })
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(!found.is_empty(), "the curriculum has topics in it");
    found
}

/// Every explanation document, by the code it is named for.
fn explanations() -> Vec<(String, String)> {
    let directory = repository().join("crates/compose-core/src/docs/codes");
    let mut found: Vec<(String, String)> = fs::read_dir(&directory)
        .expect("the explanations directory is readable")
        .map(|entry| entry.expect("a directory entry").path())
        .filter(|path| path.extension().is_some_and(|held| held == "md"))
        .map(|path| {
            let name = path
                .file_stem()
                .expect("a file name")
                .to_string_lossy()
                .into_owned();
            (
                name,
                fs::read_to_string(&path).expect("a readable explanation"),
            )
        })
        .collect();
    found.sort_by(|left, right| left.0.cmp(&right.0));
    assert!(!found.is_empty(), "there are explanations to check");
    found
}

/// The scaffold `agent-compose init` writes.
///
/// Its whole contract: a reader's first command after `init` is `validate`, and
/// it must answer about their project rather than about this compiler's file.
#[test]
fn the_init_scaffold_validates_clean() {
    validates("init-scaffold", docs::SCAFFOLD);
}

/// The scaffold is also the first document anybody reads, so the commands
/// written in its comments have to be the commands that work.
#[test]
fn the_scaffold_names_a_flow_it_defines() {
    let flow = docs::SCAFFOLD
        .lines()
        .find(|line| line.starts_with("flow."))
        .expect("the scaffold defines a flow")
        .trim_end_matches(':');
    assert!(
        docs::SCAFFOLD.contains(&format!("agent-compose run main.yml {flow}")),
        "the `run` line in the comments names `{flow}`"
    );
    assert!(
        docs::SCAFFOLD.contains(&format!("    flow: {flow}")),
        "the manual trigger names `{flow}`"
    );
}

/// Every complete spec in the curriculum validates clean.
///
/// The topics are example-led on purpose — a runnable spec before any rule —
/// which only works if the spec runs. A reader who copies the block at the top
/// of a topic gets a project, not a report.
#[test]
fn every_topic_example_validates_clean() {
    let mut checked = 0;
    for (topic, document) in topics() {
        for (index, source) in blocks(&document, "spec").into_iter().enumerate() {
            validates(&format!("topic-{topic}-{index}"), &source);
            checked += 1;
        }
    }
    // A floor derived from the exemptions rather than a number somebody chose:
    // every topic that is not exempt carries at least one, which the set
    // equality below is what actually establishes.
    let floor = topics().len() - TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE.len();
    assert!(
        checked >= floor,
        "the marker convention still finds the curriculum's examples, found {checked} of at least \
         {floor}"
    );
}

/// A topic carries a runnable example unless it is one of the two that cannot.
///
/// The curriculum's shape *is* the example: a topic opens with a complete spec
/// and teaches the rules that spec demonstrates. A topic whose example quietly
/// became a fragment — a bare ```` ```yaml ```` fence — still reads as a topic
/// and is checked by nothing, which is why this is a set equality rather than a
/// count.
#[test]
fn only_the_named_topics_lack_a_runnable_example() {
    let mut without: Vec<String> = topics()
        .into_iter()
        .filter(|(_, document)| blocks(document, "spec").is_empty())
        .map(|(topic, _)| topic)
        .collect();
    without.sort();
    let mut expected: Vec<String> = TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE
        .iter()
        .map(|(topic, _)| (*topic).to_string())
        .collect();
    expected.sort();
    assert_eq!(
        without, expected,
        "a topic gained or lost a runnable example; if a topic is genuinely reference rather than \
         curriculum, add it to TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE with the reason"
    );
}

/// The topics that teach a diagnostic by showing the spec that reports it, and
/// the codes they show.
///
/// A **set equality**, like every other list here: a topic that lost its marker
/// would go unchecked while still reading as a worked example, and one that
/// gained a block nobody listed would be teaching a code no reviewer chose.
///
/// The first two entries are warnings, which is why the marker exists at all: a
/// warning's spec is a spec that *works*, so `yaml spec` — held to a clean
/// verdict — is exactly what it cannot be marked as, and a bare fence would
/// leave the blocks in the curriculum that report something checked by nothing.
/// They are the two granularities of resolved q30's second tier: a `type:` the
/// curated table does not name, and a key it does not name inside a `type:` it
/// does. The third is an error, and is here for the neighbouring reason: it is
/// the one settings key the topic teaches by showing a spec that does **not**
/// compile — `stop:` on a connection a server-tool suite moved onto the
/// Responses wire (Decision D122) — and `yaml spec` would hold that block to a
/// clean verdict it is written to fail.
const TOPICS_THAT_TRIGGER_A_CODE: &[(&str, &str)] = &[
    ("models", "unknown-server-tool"),
    ("models", "unknown-server-tool-field"),
    ("models", "unknown-key"),
    ("models", "tool-name-collision"),
];

/// A topic's `yaml triggers <code>` block reports the code it names.
///
/// The topic half of `every_explanation_example_reports_its_code`, and it exists
/// for the same reason: a published example that stopped doing what it claims
/// teaches a rule the compiler no longer has, and the reader has no way to tell.
#[test]
fn every_topic_triggers_example_reports_its_code() {
    let known: Vec<&str> = compose_core::DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    let mut carrying = Vec::new();
    for (topic, document) in topics() {
        for (index, (code, source)) in topic_triggers_blocks(&document).into_iter().enumerate() {
            assert!(
                known.contains(&code.as_str()),
                "`{topic}` names `{code}`, which is not a diagnostic code"
            );
            carrying.push((topic.clone(), code.clone()));
            let diagnostics = report(&format!("topic-{topic}-triggers-{index}"), &source);
            let reported: Vec<&str> = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect();
            assert!(
                reported.contains(&code.as_str()),
                "the block in `{topic}` does not report `{code}`; it reports {reported:?}\n{}",
                render(&diagnostics)
            );
        }
    }
    carrying.sort();
    let mut expected: Vec<(String, String)> = TOPICS_THAT_TRIGGER_A_CODE
        .iter()
        .map(|(topic, code)| ((*topic).to_string(), (*code).to_string()))
        .collect();
    expected.sort();
    assert_eq!(
        carrying, expected,
        "a topic gained or lost a `yaml triggers <code>` block; \
         TOPICS_THAT_TRIGGER_A_CODE is the list"
    );
}

/// Every deploy example resolves against the composition it sits beside.
///
/// A deploy file is the half of a project `validate` cannot be pointed at, and
/// `targets` is where a reader learns `storage_backends`, `placements` and
/// `event_sources` at all. So the block is written to `deploy/<target>.yml`
/// beside the topic's own spec and the pair is resolved under that target —
/// which also checks the thing a deploy file alone could not: that the
/// addresses `placements:` names resolve in the composition.
#[test]
fn every_deploy_example_resolves_against_its_topic() {
    let mut carrying = Vec::new();
    for (topic, document) in topics() {
        let deploys = deploy_blocks(&document);
        if deploys.is_empty() {
            continue;
        }
        carrying.push(topic.clone());

        let specs = blocks(&document, "spec");
        assert_eq!(
            specs.len(),
            1,
            "`{topic}` has exactly one composition for its deploy files to be targets of"
        );
        for (target, body) in deploys {
            let directory = scratch(&format!("deploy-{topic}-{target}"));
            fs::write(directory.join("main.yml"), &specs[0]).expect("can write the spec");
            fs::create_dir_all(directory.join("deploy")).expect("can create `deploy/`");
            fs::write(
                directory.join("deploy").join(format!("{target}.yml")),
                &body,
            )
            .expect("can write the deploy file");

            let diagnostics = report_in(&directory, &target);
            assert!(
                diagnostics.is_empty(),
                "`{topic}`'s `deploy/{target}.yml` does not validate against its own spec:\n{}",
                render(&diagnostics)
            );
        }
    }
    carrying.sort();
    let expected: Vec<String> = TOPICS_WITH_A_DEPLOY_EXAMPLE
        .iter()
        .map(|topic| (*topic).to_string())
        .collect();
    assert_eq!(
        carrying, expected,
        "a topic gained or lost its deploy example; `yaml deploy <target>` is the marker, and \
         TOPICS_WITH_A_DEPLOY_EXAMPLE is the list"
    );
}

/// Every explanation's example reports the code the explanation is about.
///
/// This is the half that keeps an explanation honest as the compiler moves. A
/// check that stopped firing on its own example — because the shape became
/// legal, or because a neighbouring rule now catches it first — leaves an
/// explanation that teaches a failure the compiler no longer has, embedded in a
/// binary a user cannot correct.
#[test]
fn every_explanation_example_reports_its_code() {
    for (code, document) in explanations() {
        let deploys = deploy_blocks(&document);
        for (index, source) in blocks(&document, "triggers").into_iter().enumerate() {
            let name = format!("explain-{code}-{index}");
            // A deploy-layer failure is the pair, not the spec: the composition
            // on its own is well-formed, and the first deploy block is what
            // makes it an error.
            let diagnostics = if deploys.is_empty() {
                report(&name, &source)
            } else {
                report_with_deploy(&name, &source, &deploys, &deploys[0].0)
            };
            let reported: Vec<&str> = diagnostics
                .iter()
                .map(|diagnostic| diagnostic.code.as_str())
                .collect();
            assert!(
                reported.contains(&code.as_str()),
                "the example in `{code}.md` does not report `{code}`; it reports {reported:?}\n{}",
                render(&diagnostics)
            );
        }
    }
}

/// An explanation whose failure is in the deploy layer carries the deploy files
/// its example needs, and every repair it writes out resolves clean.
///
/// The deploy-layer twin of
/// [`every_corrected_explanation_example_validates_clean`], and it exists for
/// the same reason: an explanation is required to *report* its code, and a fix
/// stated only in prose is the one part nothing runs. Here the fix **is** a
/// deploy file, so it is run as one — against the same composition, under its
/// own target name.
#[test]
fn every_deploy_layer_explanation_carries_a_failing_pair_and_a_repair() {
    let mut carrying = Vec::new();
    for (code, document) in explanations() {
        let deploys = deploy_blocks(&document);
        if deploys.is_empty() {
            continue;
        }
        carrying.push(code.clone());

        let specs = blocks(&document, "triggers");
        assert_eq!(
            specs.len(),
            1,
            "`{code}.md` has exactly one composition for its deploy files to be targets of"
        );
        assert!(
            deploys.len() >= 2,
            "`{code}.md` writes a deploy file that triggers the code and none that repairs it;              the second `yaml deploy <target>` block is the fix, applied"
        );
        let mut names: Vec<&str> = deploys.iter().map(|(target, _)| target.as_str()).collect();
        names.sort_unstable();
        let distinct = names.len();
        names.dedup();
        assert_eq!(
            names.len(),
            distinct,
            "`{code}.md` names one target twice; each block becomes `deploy/<target>.yml`, so              the second would overwrite the first"
        );

        for (target, _) in &deploys[1..] {
            let diagnostics = report_with_deploy(
                &format!("explain-repair-{code}-{target}"),
                &specs[0],
                &deploys,
                target,
            );
            assert!(
                diagnostics.is_empty(),
                "`{code}.md`'s repaired `deploy/{target}.yml` does not validate against its own \
                 spec:\n{}",
                render(&diagnostics)
            );
        }
    }
    carrying.sort();
    let expected: Vec<String> = EXPLANATIONS_WITH_A_DEPLOY_EXAMPLE
        .iter()
        .map(|code| (*code).to_string())
        .collect();
    assert_eq!(
        carrying, expected,
        "an explanation gained or lost its deploy example; `yaml deploy <target>` is the marker, \
         and EXPLANATIONS_WITH_A_DEPLOY_EXAMPLE is the list"
    );
}

/// Every repair an explanation writes out validates clean.
///
/// The other half of an explanation's honesty. Its example is required to
/// *report* the code; this requires the fix beside it to *clear* it — and to
/// clear every other check too, which is the part prose cannot promise. A
/// repair that satisfies the rule it is about and then fails a neighbouring one
/// is worse than no repair: the reader has followed the instruction they were
/// given and is now holding a second diagnostic.
#[test]
fn every_corrected_explanation_example_validates_clean() {
    let mut carrying = Vec::new();
    for (code, document) in explanations() {
        let corrections = blocks(&document, "spec");
        if corrections.is_empty() {
            continue;
        }
        carrying.push(code.clone());
        for (index, source) in corrections.into_iter().enumerate() {
            validates(&format!("explain-corrected-{code}-{index}"), &source);
        }
    }
    carrying.sort();
    let expected: Vec<String> = EXPLANATIONS_WITH_A_CORRECTED_EXAMPLE
        .iter()
        .map(|code| (*code).to_string())
        .collect();
    assert_eq!(
        carrying, expected,
        "an explanation gained or lost its written-out repair; `yaml spec` is the marker, and \
         EXPLANATIONS_WITH_A_CORRECTED_EXAMPLE is the list"
    );
}

/// An explanation carries a runnable example unless it is one of the five that
/// cannot.
///
/// Without this the check above is worth nothing: an explanation whose example
/// stopped triggering could be "fixed" by dropping the marker, and every code
/// added later could skip the check by never carrying one. The exemption list
/// is in this file so that opting out is an edit a reviewer sees.
#[test]
fn only_the_named_explanations_lack_a_runnable_example() {
    let exempt: Vec<&str> = EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE
        .iter()
        .map(|(code, _)| *code)
        .collect();

    let mut without = Vec::new();
    for (code, document) in explanations() {
        if blocks(&document, "triggers").is_empty() {
            without.push(code);
        }
    }
    without.sort();
    let mut expected: Vec<String> = exempt.iter().map(|code| (*code).to_string()).collect();
    expected.sort();
    assert_eq!(
        without, expected,
        "an explanation gained or lost a runnable example; if a code genuinely cannot be \
         demonstrated by one `main.yml`, add it to EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE with \
         the reason"
    );
}

/// Both explanation lists name codes that exist.
///
/// A stale entry in the exemption list would silently excuse nothing while
/// looking like it excused something; a stale entry in the corrections list
/// would require a block of a document that is not there, which is a failure
/// that names the wrong thing.
#[test]
fn every_listed_explanation_names_a_real_code() {
    let known: Vec<&str> = compose_core::DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    for (code, reason) in EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE {
        assert!(known.contains(code), "`{code}` is not a diagnostic code");
        assert!(!reason.is_empty(), "`{code}`'s exemption states a reason");
    }
    for code in EXPLANATIONS_WITH_A_CORRECTED_EXAMPLE {
        assert!(known.contains(code), "`{code}` is not a diagnostic code");
    }
    for code in EXPLANATIONS_WITH_A_DEPLOY_EXAMPLE {
        assert!(known.contains(code), "`{code}` is not a diagnostic code");
    }
}

/// The two topic lists name topics that exist.
///
/// Same residual as the codes': a stale entry would excuse nothing while
/// looking like it excused something, and a renamed topic is exactly how one
/// goes stale.
#[test]
fn every_listed_topic_is_a_registered_topic() {
    let known = docs::topics::names();
    for (topic, reason) in TOPICS_WITHOUT_A_RUNNABLE_EXAMPLE {
        assert!(known.contains(topic), "`{topic}` is not a topic");
        assert!(!reason.is_empty(), "`{topic}`'s exemption states a reason");
    }
    for topic in TOPICS_WITH_A_DEPLOY_EXAMPLE {
        assert!(known.contains(topic), "`{topic}` is not a topic");
    }
}
