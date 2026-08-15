//! The negative corpus for resolution: one minimal *project* per rule the
//! resolver owns, pinned to its exact code, message, position, and — where the
//! rule names two sites — its label.
//!
//! Every fixture is a directory, because everything this pass decides needs
//! more than one file to state: an import that does not resolve, an address
//! defined twice, a section declared in two places, a reference to a definition
//! nobody wrote. The expectation lives in a header at the top of the project's
//! `main.yml`, in the shape `tests/parse_invalid.rs` already uses:
//!
//! ```yaml
//! # rule: an imported file may not declare `imports:` (Decision D1)
//! # code: misplaced-section
//! # message: `middle.yml` is imported and may not declare `imports:`
//! # at: middle.yml:4:3
//! # label: main.yml:9:5 imported here
//! # help: <optional, asserted when present>
//! # count: <optional, total diagnostics expected; defaults to 1>
//! # target: <optional, the target to resolve for; defaults to `local`>
//! ```
//!
//! Header reading stops at the first line that is not a `# `-prefixed comment,
//! and a comment whose key is not one of the eight above is skipped — so a
//! fixture may carry as much prose after its header as the case needs, which
//! several of them do.
//!
//! `at:` and `label:` take an optional file prefix, defaulting to `main.yml` —
//! the only file every fixture has. Error UX is a product feature (PRD G3), so
//! a change to any of this is a test failure rather than something a reviewer
//! might miss.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, resolve_with_target};

/// Every code the resolver raises that a fixture can reach. The corpus must
/// exercise all of them.
///
/// Three are shared with the parser — an unreadable file, an import path that
/// is not one, and a section in a file that may not carry it — because they are
/// the same failure class wherever it is noticed.
///
/// Two of the resolver's codes are deliberately absent:
///
/// * `invalid-encoding` needs a file that is not UTF-8, which no readable
///   fixture can carry; `tests/parse_invalid.rs` pins it on the parser side.
/// * `version-mismatch` cannot be reached at all while
///   `SUPPORTED_SPEC_VERSIONS` has one member, because a value outside the
///   accepted set is refused by the parser before the resolver sees it and any
///   two accepted values are then equal. The rule is implemented and unit-tested
///   in `resolve::index`, and it becomes reachable — and belongs here — the
///   moment a second version ships.
const RESOLVER_CODES: &[&str] = &[
    "io-error",
    "invalid-import-path",
    "invalid-value",
    "misplaced-section",
    "missing-key",
    "duplicate-definition",
    "duplicate-section",
    "undefined-reference",
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
    label: Option<(Anchor, String)>,
    help: Option<String>,
    count: usize,
    target: String,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-resolve")
}

fn fixtures() -> Vec<PathBuf> {
    let mut cases: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("the invalid-resolve fixture directory exists")
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
    for line in source.lines() {
        let Some(comment) = line.strip_prefix("# ") else {
            break;
        };
        let Some((key, value)) = comment.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if [
            "rule", "code", "message", "at", "label", "help", "count", "target",
        ]
        .contains(&key)
        {
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
        label: values.get("label").map(|value| {
            let (anchored, message) = value
                .split_once(' ')
                .unwrap_or_else(|| panic!("{label}: `# label:` is `<anchor> <message>`"));
            (anchor(anchored, &label), message.to_string())
        }),
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

#[test]
fn every_fixture_produces_exactly_the_diagnostic_it_declares() {
    let mut failures = Vec::new();
    for case in fixtures() {
        let expected = header(&case);
        let resolution = resolve_with_target(case.join("main.yml"), &expected.target);
        let mut problems = Vec::new();

        if resolution.ir.is_some() {
            problems.push("an artifact was produced for a rejected composition".to_string());
        }
        if resolution.diagnostics.len() != expected.count {
            problems.push(format!(
                "expected {} diagnostic(s), got {}",
                expected.count,
                resolution.diagnostics.len()
            ));
        }

        let matching: Vec<&Diagnostic> = resolution
            .diagnostics
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
                match (&expected.label, diagnostic.labels.as_slice()) {
                    (None, _) => {}
                    (Some((anchor, message)), [label, ..]) => {
                        check_anchor(&mut problems, "label position", anchor, &label.span);
                        if &label.message != message {
                            problems.push(format!(
                                "label differs\n      expected: {message}\n      actual:   {}",
                                label.message
                            ));
                        }
                    }
                    (Some(_), []) => problems.push("the diagnostic carries no label".to_string()),
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
                if resolution.diagnostics.is_empty() {
                    "    <none>".to_string()
                } else {
                    resolution
                        .diagnostics
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
fn the_corpus_covers_every_diagnostic_the_resolver_raises() {
    let covered: BTreeSet<String> = fixtures().iter().map(|case| header(case).code).collect();
    let missing: Vec<&str> = RESOLVER_CODES
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
        count >= 15,
        "the negative corpus must cover at least 15 rules, found {count}"
    );
}

/// Every fixture's own files must be readable as a project: the harness reads
/// the header out of `main.yml`, so a case without one is a silently skipped
/// test rather than a failing one.
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
