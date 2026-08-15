//! The positive corpus: every file under `examples/` must parse with zero
//! diagnostics, and the AST of a representative slice is snapshotted.
//!
//! The examples are the worked reference for the grammar (`docs/grammar.md`
//! §1.7), so a parser change that rejects one of them is a regression. The
//! snapshots go further: they lock the *shape* the parser hands to the resolver,
//! so a change to what the AST records shows up as a reviewable diff instead of
//! silently altering what every downstream pass consumes.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::ast::{Document, DocumentKind};
use compose_core::parse_file;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("manifest dir has a grandparent")
        .to_path_buf()
}

fn examples_dir() -> PathBuf {
    repo_root().join("examples")
}

fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect(dir, &mut found);
    found.sort();
    found
}

fn collect(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read directory {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            collect(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml") | Some("yaml")
        ) {
            out.push(path);
        }
    }
}

fn relative(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

/// Parse an example, asserting it is clean, and return its document.
///
/// Snapshots go through `parse_str` with the repo-relative name so that the
/// recorded spans name `examples/…` rather than wherever the checkout happens
/// to live.
fn parse_clean(relative_path: &str) -> Document {
    let path = repo_root().join(relative_path);
    let source =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let parsed = compose_core::parse_str(&source, relative_path);
    assert!(
        parsed.diagnostics.is_empty(),
        "{relative_path} produced diagnostics:\n{}",
        render(&parsed.diagnostics)
    );
    parsed
        .document
        .unwrap_or_else(|| panic!("{relative_path} produced no document"))
}

fn render(diagnostics: &[compose_core::Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} [{}] {}{}",
                diagnostic.span,
                diagnostic.code,
                diagnostic.message,
                diagnostic
                    .help
                    .as_ref()
                    .map(|help| format!("\n      help: {help}"))
                    .unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn every_example_file_parses_without_diagnostics() {
    let files = yaml_files(&examples_dir());
    assert!(
        files.len() >= 20,
        "expected the example corpus to be substantial, found {} files",
        files.len()
    );

    let mut failures = Vec::new();
    for file in &files {
        let parsed = parse_file(file);
        if !parsed.diagnostics.is_empty() {
            failures.push(format!(
                "{}\n{}",
                relative(file),
                render(&parsed.diagnostics)
            ));
        }
        if parsed.document.is_none() {
            failures.push(format!("{}: no document was produced", relative(file)));
        }
    }
    assert!(
        failures.is_empty(),
        "{} example file(s) failed to parse cleanly:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn examples_parse_as_the_document_kind_their_layout_implies() {
    for file in yaml_files(&examples_dir()) {
        let document = parse_file(&file).document.expect("a document");
        let expected = if file.components().any(|part| part.as_os_str() == "deploy") {
            DocumentKind::Deploy
        } else {
            DocumentKind::Spec
        };
        assert_eq!(
            document.kind(),
            expected,
            "{} parsed as a {}",
            relative(&file),
            document.kind().as_str()
        );
    }
}

/// Truncation fuzzing over the corpus: every prefix of every example is
/// malformed in some way, and none of them may panic. "Parse never panics" is
/// the property the whole error-UX posture rests on — a compiler that aborts
/// gives a coding agent nothing to act on (PRD G3).
#[test]
fn truncated_examples_never_panic() {
    for file in yaml_files(&examples_dir()) {
        let source = fs::read_to_string(&file).expect("readable example");
        for length in (0..source.len()).step_by(7) {
            // Truncate on a character boundary, then parse whatever is left.
            let end = (0..=length)
                .rev()
                .find(|index| source.is_char_boundary(*index))
                .unwrap_or(0);
            let parsed = compose_core::parse_str(&source[..end], relative(&file));
            // Either it parsed or it explained itself; silence is the one
            // outcome that would mean a swallowed failure.
            assert!(
                parsed.document.is_some() || !parsed.diagnostics.is_empty(),
                "{} truncated to {end} bytes yielded neither a document nor a diagnostic",
                relative(&file)
            );
        }
    }
}

#[test]
fn snapshot_review_loop_main() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/main.yml"));
}

#[test]
fn snapshot_review_loop_reviewer_agent() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/agents/reviewer.yml"));
}

#[test]
fn snapshot_review_loop_flow() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/flows/review_loop.yml"));
}

#[test]
fn snapshot_review_loop_triggers() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/triggers.yml"));
}

#[test]
fn snapshot_review_loop_tool() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/tools/web_search.yml"));
}

#[test]
fn snapshot_review_loop_models() {
    insta::assert_debug_snapshot!(parse_clean("examples/review-loop/models.yml"));
}

#[test]
fn snapshot_triage_fanout_main() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/main.yml"));
}

#[test]
fn snapshot_triage_map_flow() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/flows/triage.yml"));
}

#[test]
fn snapshot_triage_agent_with_union_output() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/agents/triage.yml"));
}

#[test]
fn snapshot_triage_stores() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/stores/docs.yml"));
}

#[test]
fn snapshot_triage_triggers() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/triggers.yml"));
}

#[test]
fn snapshot_triage_deploy_target() {
    insta::assert_debug_snapshot!(parse_clean("examples/triage-fanout/deploy/staging.yml"));
}
