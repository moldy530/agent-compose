//! The negative corpus for the validator: one minimal *resolvable* project per
//! rule, pinned to its exact code, message, position, and — where the rule names
//! two sites — its label.
//!
//! Every fixture resolves cleanly and then fails exactly one check, which is
//! what makes the corpus evidence about that check rather than about the
//! parser or the resolver: a fixture that stops resolving is a failure of this
//! harness, not a passing test with a different cause. The expectation lives in
//! a header at the top of the project's `main.yml`, in the shape
//! `tests/resolve_invalid.rs` already uses:
//!
//! ```yaml
//! # rule: an `append` channel takes one element per write (grammar 10.2)
//! # code: type-mismatch
//! # message: `matches` of node `scan` cannot be written to `patches`: …
//! # at: main.yml:31:5
//! # label: main.yml:9:3 the channel is declared here
//! # help: <optional, asserted when present>
//! # count: <optional, total diagnostics expected; defaults to 1>
//! # target: <optional, the target to resolve for; defaults to `local`>
//! ```
//!
//! Header reading stops at the first line that is not a `# `-prefixed comment,
//! and a comment whose key is not one of the eight above is skipped — so a
//! fixture may carry as much prose after its header as the case needs.
//!
//! `# label:` is the one repeatable key: a rule that points at several sites
//! declares one line per site, in the order the diagnostic carries them, and
//! each is asserted. A fixture may declare fewer labels than the diagnostic
//! draws — pinning the first is enough for a rule whose later labels are
//! another rule's business — but never a different one.
//!
//! Error UX is a product feature (PRD G3), so a change to any of this is a test
//! failure rather than something a reviewer might miss.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, resolve_with_target};

/// Every code the validator raises. The corpus must exercise all of them.
///
/// Two rules the validator implements are deliberately unpinned, and both are
/// the same case: no v0 provider kind lacks structured output or tool use, so
/// neither the agent-model capability rule nor route capability equivalence can
/// fire on a composition this grammar admits (see `check::providers`). They are
/// unit-tested against the table instead —
/// `every_v0_kind_serves_structured_output` and
/// `every_v0_kind_publishes_the_same_inference_capabilities`, which `Evidence`
/// in `tests/static_check_inventory.rs` reads as the second rule's evidence in place of a
/// fixture — and the third rule reading that table, `embed.provider`, is pinned
/// here. Their shared code, `missing-capability`, is therefore covered below by
/// that third rule alone.
const CHECK_CODES: &[&str] = &[
    "invalid-expression",
    "unknown-root",
    "unknown-field",
    "type-mismatch",
    "undefined-channel",
    "unreduced-write",
    "conflicting-writes",
    "missing-binding",
    "unbounded-fan-out",
    "non-exhaustive",
    "detached-write",
    "unkeyed-map-write",
    "tool-name-collision",
    "missing-session-key",
    "missing-capability",
    "missing-key",
    "unknown-key",
    "unknown-variant",
    "value-out-of-range",
    "invalid-value",
    "dead-end",
    "unbounded-cycle",
    "unbalanced-convergence",
    "unreachable-node",
    "recursive-flow",
    "sync-trigger-interrupt",
    "non-dominating-source",
    "unsupported-detach",
    "conflicting-keys",
];

struct Anchor {
    file: String,
    line: u32,
    column: u32,
}

struct Expectation {
    rule: String,
    code: String,
    message: String,
    at: Anchor,
    labels: Vec<(Anchor, String)>,
    help: Option<String>,
    count: usize,
    target: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-check")
}

fn fixtures() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("the invalid-check fixture directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.is_dir())
        .collect();
    cases.sort();
    cases
}

fn name(case: &Path) -> String {
    case.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

/// Read `[<file>:]<line>:<column>`, defaulting the file to `main.yml`.
fn anchor(value: &str, case: &str) -> Anchor {
    let (head, column) = value.rsplit_once(':').unwrap_or_else(|| {
        panic!("{case}: an anchor is `[<file>:]<line>:<column>`, got `{value}`")
    });
    let (file, line) = match head.rsplit_once(':') {
        Some((file, line)) => (file.to_string(), line),
        None => ("main.yml".to_string(), head),
    };
    Anchor {
        file,
        line: line
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{case}: `{value}` has no line number")),
        column: column
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{case}: `{value}` has no column number")),
    }
}

fn header(case: &Path) -> Expectation {
    let label = name(case);
    let source = fs::read_to_string(case.join("main.yml"))
        .unwrap_or_else(|e| panic!("{label}: cannot read main.yml: {e}"));
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    let mut labels: Vec<String> = Vec::new();
    for line in source.lines() {
        let Some(comment) = line.strip_prefix("# ") else {
            break;
        };
        let Some((key, value)) = comment.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if key == "label" {
            labels.push(value.trim().to_string());
            continue;
        }
        if ["rule", "code", "message", "at", "help", "count", "target"].contains(&key) {
            values
                .entry(key)
                .or_insert_with(|| value.trim().to_string());
        }
    }

    let missing: Vec<&str> = ["rule", "code", "message", "at"]
        .into_iter()
        .filter(|key| !values.contains_key(key))
        .collect();
    assert!(
        missing.is_empty(),
        "{label} is missing header field(s): {missing:?}"
    );

    Expectation {
        rule: values["rule"].clone(),
        code: values["code"].clone(),
        message: values["message"].clone(),
        at: anchor(&values["at"], &label),
        labels: labels
            .iter()
            .map(|value| {
                let (anchored, message) = value
                    .split_once(' ')
                    .unwrap_or_else(|| panic!("{label}: `# label:` is `<anchor> <message>`"));
                (anchor(anchored, &label), message.to_string())
            })
            .collect(),
        help: values.get("help").cloned(),
        count: values
            .get("count")
            .map(|count| {
                count
                    .parse()
                    .unwrap_or_else(|_| panic!("{label}: `# count:` is not a number"))
            })
            .unwrap_or(1),
        target: values
            .get("target")
            .cloned()
            .unwrap_or_else(|| compose_core::DEFAULT_TARGET.to_string()),
    }
}

fn as_header(diagnostic: &Diagnostic) -> String {
    let mut rendered = format!(
        "    # code: {}\n    # message: {}\n    # at: {}:{}:{}",
        diagnostic.code,
        diagnostic.message,
        diagnostic.span.source,
        diagnostic.span.start.line,
        diagnostic.span.start.column
    );
    for label in &diagnostic.labels {
        rendered.push_str(&format!(
            "\n    # label: {}:{}:{} {}",
            label.span.source, label.span.start.line, label.span.start.column, label.message
        ));
    }
    if let Some(help) = &diagnostic.help {
        rendered.push_str(&format!("\n    # help: {help}"));
    }
    rendered
}

fn check_anchor(
    problems: &mut Vec<String>,
    what: &str,
    expected: &Anchor,
    actual: &compose_core::Span,
) {
    if actual.source.as_str() != expected.file
        || actual.start.line != expected.line
        || actual.start.column != expected.column
    {
        problems.push(format!(
            "{what} differs: expected {}:{}:{}, actual {}:{}:{}",
            expected.file,
            expected.line,
            expected.column,
            actual.source,
            actual.start.line,
            actual.start.column
        ));
    }
}

#[test]
fn every_fixture_produces_exactly_the_diagnostic_it_declares() {
    let mut failures = Vec::new();
    for case in fixtures() {
        let expected = header(&case);
        let resolution = resolve_with_target(case.join("main.yml"), &expected.target);
        let mut problems = Vec::new();
        let Some(ir) = resolution.ir else {
            failures.push(format!(
                "{} ({})\n  the project does not resolve, so no check ran:\n{}",
                name(&case),
                expected.rule,
                resolution
                    .diagnostics
                    .iter()
                    .map(as_header)
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
            continue;
        };
        assert!(
            resolution.diagnostics.is_empty(),
            "{}: a fixture for a data check resolves clean",
            name(&case)
        );

        let diagnostics = compose_core::check(&ir);
        if diagnostics.len() != expected.count {
            problems.push(format!(
                "expected {} diagnostic(s), got {}",
                expected.count,
                diagnostics.len()
            ));
        }

        let matching: Vec<&Diagnostic> = diagnostics
            .iter()
            .filter(|diagnostic| diagnostic.code.as_str() == expected.code)
            .collect();
        match matching.as_slice() {
            [] => problems.push(format!("no diagnostic has code `{}`", expected.code)),
            [diagnostic] => {
                if diagnostic.message != expected.message {
                    problems.push(format!(
                        "message differs\n      expected: {}\n      actual:   {}",
                        expected.message, diagnostic.message
                    ));
                }
                check_anchor(&mut problems, "position", &expected.at, &diagnostic.span);
                for (index, (anchor, message)) in expected.labels.iter().enumerate() {
                    let Some(label) = diagnostic.labels.get(index) else {
                        problems.push(format!(
                            "the diagnostic carries {} label(s), and the fixture declares {}",
                            diagnostic.labels.len(),
                            expected.labels.len()
                        ));
                        break;
                    };
                    check_anchor(
                        &mut problems,
                        &format!("label {} position", index + 1),
                        anchor,
                        &label.span,
                    );
                    if &label.message != message {
                        problems.push(format!(
                            "label {} differs\n      expected: {message}\n      actual:   {}",
                            index + 1,
                            label.message
                        ));
                    }
                }
                if let Some(help) = &expected.help
                    && diagnostic.help.as_deref() != Some(help.as_str())
                {
                    problems.push(format!(
                        "help differs\n      expected: {help}\n      actual:   {}",
                        diagnostic.help.as_deref().unwrap_or("<none>")
                    ));
                }
            }
            many => problems.push(format!(
                "{} diagnostics share code `{}`; a fixture pins exactly one",
                many.len(),
                expected.code
            )),
        }

        if !problems.is_empty() {
            failures.push(format!(
                "{} ({})\n  {}\n  actual diagnostics:\n{}",
                name(&case),
                expected.rule,
                problems.join("\n  "),
                if diagnostics.is_empty() {
                    "    <none>".to_string()
                } else {
                    diagnostics
                        .iter()
                        .map(as_header)
                        .collect::<Vec<_>>()
                        .join("\n")
                }
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "{} fixture(s) no longer produce the diagnostic they declare:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_fixture_declares_a_distinct_rule() {
    let mut rules: BTreeMap<String, String> = BTreeMap::new();
    for case in fixtures() {
        let expected = header(&case);
        if let Some(other) = rules.insert(expected.rule.clone(), name(&case)) {
            panic!(
                "{} and {} both claim the rule `{}`",
                name(&case),
                other,
                expected.rule
            );
        }
    }
}

#[test]
fn the_corpus_covers_every_diagnostic_the_checks_raise() {
    let covered: BTreeSet<String> = fixtures().iter().map(|case| header(case).code).collect();
    let missing: Vec<&str> = CHECK_CODES
        .iter()
        .copied()
        .filter(|code| !covered.contains(*code))
        .collect();
    assert!(
        missing.is_empty(),
        "no fixture pins these diagnostic codes: {missing:?}"
    );
}

#[test]
fn the_corpus_is_substantial() {
    let count = fixtures().len();
    assert!(
        count >= 80,
        "the negative corpus must cover at least 80 rules, found {count}"
    );
}

#[test]
fn every_fixture_has_an_entrypoint() {
    for case in fixtures() {
        assert!(
            case.join("main.yml").is_file(),
            "{} has no main.yml",
            name(&case)
        );
    }
}
