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
//! * ```` ```yaml ```` — a fragment. Skipped, and deliberately so: a block
//!   showing `retry: { max: 2 }` on its own is not a document and has no
//!   verdict to have.
//!
//! The marker is one word after the language because it has to survive a
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
//! [`EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE`] is what keeps that second check
//! honest. Five codes cannot be demonstrated by one `main.yml`, and each is
//! listed there with the reason; every other explanation MUST carry a
//! `yaml triggers` block, so a new code cannot quietly opt out of being checked
//! without editing this file.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, docs, resolve};

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

/// Resolve and check one complete spec written as `main.yml`.
fn report(name: &str, source: &str) -> Vec<Diagnostic> {
    let directory = scratch(name);
    let entrypoint = directory.join("main.yml");
    fs::write(&entrypoint, source).expect("can write the spec");

    let resolution = resolve(&entrypoint);
    let mut diagnostics = resolution.diagnostics;
    if let Some(ir) = &resolution.ir {
        diagnostics.extend(compose_core::check(ir));
    }
    diagnostics
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
    assert!(
        checked >= 10,
        "the marker convention still finds the curriculum's examples, found {checked}"
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
        for (index, source) in blocks(&document, "triggers").into_iter().enumerate() {
            let diagnostics = report(&format!("explain-{code}-{index}"), &source);
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

/// The exemption list names codes that exist.
///
/// A stale entry would silently excuse nothing while looking like it excused
/// something.
#[test]
fn every_exempt_explanation_names_a_real_code() {
    let known: Vec<&str> = compose_core::DiagnosticCode::ALL
        .iter()
        .map(|code| code.as_str())
        .collect();
    for (code, reason) in EXPLANATIONS_WITHOUT_A_RUNNABLE_EXAMPLE {
        assert!(known.contains(code), "`{code}` is not a diagnostic code");
        assert!(!reason.is_empty(), "`{code}`'s exemption states a reason");
    }
}
