//! The negative corpus: one minimal file per diagnostic the parser can raise,
//! pinned to its exact code, message, and position.
//!
//! Error UX is a product feature (PRD G3, CLAUDE.md "Validation strategy"), so
//! a change in an error message has to be a test failure rather than something
//! a reviewer might miss. Each fixture opens with a header naming the grammar
//! rule it violates and the diagnostic it must produce:
//!
//! ```yaml
//! # rule: an agent declares `model:` (grammar 5)
//! # code: missing-key
//! # message: missing required key `model` in agent definition `agent.reviewer`
//! # at: 2:1
//! # help: <optional, asserted when present>
//! # count: <optional, total diagnostics expected; defaults to 1>
//! ```
//!
//! The `count` default is what keeps fixtures honest in the other direction: a
//! fixture that starts producing a second, unrelated diagnostic fails until
//! someone looks at it.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, parse_str};

/// Every code the parser raises, except `io-error`, which needs a missing file
/// rather than a fixture. The corpus must exercise all of them.
const PARSER_CODES: &[&str] = &[
    "invalid-encoding",
    "yaml-syntax",
    "empty-document",
    "multiple-documents",
    "root-not-mapping",
    "non-string-key",
    "duplicate-key",
    "merge-key",
    "yaml-tag",
    "misplaced-section",
    "unsupported-version",
    "invalid-import-path",
    "unknown-key",
    "missing-key",
    "wrong-type",
    "invalid-value",
    "value-out-of-range",
    "unknown-variant",
    "conflicting-keys",
    "missing-credential",
    "invalid-identifier",
    "invalid-reference",
    "invalid-duration",
    "invalid-path-expression",
    "invalid-env-ref",
    "unexpected-env-ref",
    "reserved-name",
];

struct Expectation {
    rule: String,
    code: String,
    message: String,
    line: u32,
    column: u32,
    help: Option<String>,
    count: usize,
}

fn fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-parse")
}

fn fixtures() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(fixtures_dir())
        .expect("the invalid-parse fixture directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            matches!(
                path.extension().and_then(|e| e.to_str()),
                Some("yml") | Some("yaml")
            )
        })
        .collect();
    files.sort();
    files
}

fn name(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_string()
}

fn read(path: &Path) -> String {
    // A fixture may deliberately be invalid UTF-8 or carry a byte-order mark;
    // read it as bytes so the harness itself never panics on one.
    let bytes = fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    String::from_utf8_lossy(&bytes).into_owned()
}

fn header(path: &Path) -> Expectation {
    let source = read(path);
    // One fixture is a byte-order mark, which the parser must reject and the
    // header parser must see past.
    let source = source.strip_prefix('\u{feff}').unwrap_or(&source);
    let mut values: BTreeMap<&str, String> = BTreeMap::new();
    for line in source.lines() {
        let Some(comment) = line.strip_prefix("# ") else {
            break;
        };
        let Some((key, value)) = comment.split_once(':') else {
            continue;
        };
        let key = key.trim();
        if ["rule", "code", "message", "at", "help", "count"].contains(&key) {
            values
                .entry(match key {
                    "rule" => "rule",
                    "code" => "code",
                    "message" => "message",
                    "at" => "at",
                    "help" => "help",
                    _ => "count",
                })
                .or_insert_with(|| value.trim().to_string());
        }
    }

    let missing: Vec<&str> = ["rule", "code", "message", "at"]
        .into_iter()
        .filter(|key| !values.contains_key(key))
        .collect();
    assert!(
        missing.is_empty(),
        "{} is missing header field(s): {missing:?}",
        name(path)
    );

    let at = values["at"].clone();
    let (line, column) = at
        .split_once(':')
        .unwrap_or_else(|| panic!("{}: `# at:` must be `<line>:<column>`", name(path)));
    Expectation {
        rule: values["rule"].clone(),
        code: values["code"].clone(),
        message: values["message"].clone(),
        line: line
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{}: `# at:` line is not a number", name(path))),
        column: column
            .trim()
            .parse()
            .unwrap_or_else(|_| panic!("{}: `# at:` column is not a number", name(path))),
        help: values.get("help").cloned(),
        count: values
            .get("count")
            .map(|count| {
                count
                    .parse()
                    .unwrap_or_else(|_| panic!("{}: `# count:` is not a number", name(path)))
            })
            .unwrap_or(1),
    }
}

fn as_header(diagnostic: &Diagnostic) -> String {
    let mut rendered = format!(
        "    # code: {}\n    # message: {}\n    # at: {}:{}",
        diagnostic.code,
        diagnostic.message,
        diagnostic.span.start.line,
        diagnostic.span.start.column
    );
    if let Some(help) = &diagnostic.help {
        rendered.push_str(&format!("\n    # help: {help}"));
    }
    rendered
}

#[test]
fn every_fixture_produces_exactly_the_diagnostic_it_declares() {
    let mut failures = Vec::new();
    for path in fixtures() {
        let expected = header(&path);
        let parsed = parse_str(&read(&path), name(&path));
        let mut problems = Vec::new();

        if parsed.diagnostics.len() != expected.count {
            problems.push(format!(
                "expected {} diagnostic(s), got {}",
                expected.count,
                parsed.diagnostics.len()
            ));
        }

        let matching: Vec<&Diagnostic> = parsed
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
                let start = &diagnostic.span.start;
                if start.line != expected.line || start.column != expected.column {
                    problems.push(format!(
                        "position differs: expected {}:{}, actual {}:{}",
                        expected.line, expected.column, start.line, start.column
                    ));
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
                name(&path),
                expected.rule,
                problems.join("\n  "),
                if parsed.diagnostics.is_empty() {
                    "    <none>".to_string()
                } else {
                    parsed
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

#[test]
fn every_fixture_declares_a_distinct_rule() {
    let mut rules: BTreeMap<String, String> = BTreeMap::new();
    for path in fixtures() {
        let expected = header(&path);
        if let Some(other) = rules.insert(expected.rule.clone(), name(&path)) {
            panic!(
                "{} and {} both claim the rule `{}`",
                name(&path),
                other,
                expected.rule
            );
        }
    }
}

#[test]
fn the_corpus_covers_every_diagnostic_the_parser_raises() {
    let covered: BTreeSet<String> = fixtures().iter().map(|path| header(path).code).collect();
    let missing: Vec<&str> = PARSER_CODES
        .iter()
        .copied()
        .filter(|code| !covered.contains(*code))
        .collect();
    assert!(
        missing.is_empty(),
        "no fixture pins these diagnostic codes: {missing:?}"
    );

    let unknown: Vec<&String> = covered
        .iter()
        .filter(|code| !PARSER_CODES.contains(&code.as_str()) && *code != "io-error")
        .collect();
    assert!(
        unknown.is_empty(),
        "fixtures declare codes the parser does not document: {unknown:?}"
    );
}

#[test]
fn the_corpus_is_substantial() {
    let count = fixtures().len();
    assert!(
        count >= 25,
        "the negative corpus must cover at least 25 rules, found {count}"
    );
}

/// The published schema is the editor-facing approximation of the parser, and
/// `docs/grammar.md` Appendix B fixes the direction of the relationship: a file
/// that fails the schema always fails `validate`. The parser is the first pass
/// of `validate` and every rule the schema encodes is decidable per file, so
/// the parser has to reject everything in the schema's own negative corpus.
///
/// What this measures is that corpus, not the whole schema: a rule neither
/// corpus covers can still drift apart unnoticed, which is exactly how the
/// routed-map, model-route-arity, input-surface-`default:` and duplicate-import
/// holes survived a round. The remedy when one turns up is to add the shape to
/// **both** corpora — `invalid-schema/` so this replay covers it, and
/// `invalid-parse/` so the diagnostic it produces is pinned exactly.
#[test]
fn the_parser_rejects_everything_the_published_schema_rejects() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-schema");
    let mut files: Vec<PathBuf> = fs::read_dir(&dir)
        .expect("the invalid-schema fixture directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| path.extension().and_then(|e| e.to_str()) == Some("yml"))
        .collect();
    files.sort();
    assert!(files.len() >= 12, "the schema corpus should be substantial");

    let accepted: Vec<String> = files
        .iter()
        .filter(|path| parse_str(&read(path), name(path)).diagnostics.is_empty())
        .map(|path| name(path))
        .collect();
    assert!(
        accepted.is_empty(),
        "the parser accepts {} file(s) the published schema rejects:\n  {}",
        accepted.len(),
        accepted.join("\n  ")
    );
}

#[test]
fn a_missing_file_is_a_diagnostic_rather_than_a_panic() {
    let parsed = compose_core::parse_file(fixtures_dir().join("does-not-exist.yml"));
    assert!(parsed.document.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].code.as_str(), "io-error");
}

#[test]
fn invalid_utf8_is_a_diagnostic_rather_than_a_panic() {
    let path = std::env::temp_dir().join("agent-compose-invalid-utf8.yml");
    fs::write(&path, [0x76, 0x3a, 0x20, 0xff, 0xfe]).expect("can write to the temp dir");
    let parsed = compose_core::parse_file(&path);
    let _ = fs::remove_file(&path);
    assert!(parsed.document.is_none());
    assert_eq!(parsed.diagnostics.len(), 1);
    assert_eq!(parsed.diagnostics[0].code.as_str(), "invalid-encoding");
}
