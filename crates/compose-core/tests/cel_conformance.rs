//! The CEL conformance corpus, run against the Rust evaluator.
//!
//! CLAUDE.md's validation strategy names one hazard by name: **CEL semantic
//! drift between the Rust validator and the JS runtime**, and it names the
//! mitigation — a shared corpus of fixtures executed against both
//! interpreters, with divergence failing CI. This is that corpus, and this test
//! is its first consumer. The second is M1's, which embeds a JS evaluator in
//! generated routers and has to pass these files unchanged.
//!
//! # The format
//!
//! Each file under `tests/fixtures/cel-conformance/` is a JSON array of cases:
//!
//! ```jsonc
//! {
//!   "name":       "size-of-a-list",            // unique across the corpus
//!   "expression": "size(state.items)",         // the CEL source
//!   "input":      { "state": { "items": ["a"] } },  // root name -> value
//!   "result":     1                            // the expected value, as JSON
//! }
//! ```
//!
//! A case that must *fail* to evaluate writes `"error": true` instead of
//! `"result"`. What is pinned is that evaluation fails, never the message: the
//! two implementations are held to the same accept/reject decision and to the
//! same values, not to one another's wording.
//!
//! Plain JSON data files, one loader per implementation: nothing here imports
//! anything of this crate's, so the corpus stays portable to a runner written
//! in another language.
//!
//! # What it covers
//!
//! The operators and functions grammar 4.1 puts on the supported surface, over
//! the value shapes the schema language can produce. Numeric types are
//! distinguished as CEL distinguishes them — `1` is an `int` and `1.0` a
//! `double`, and the corpus pins the arithmetic that crosses them as an
//! error — because a JS evaluator that folds the two would diverge here and
//! nowhere visible.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use cel::objects::{Key, Map};
use cel::{Context, Program, Value};
use serde_json::Value as Json;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/cel-conformance")
}

struct Case {
    file: String,
    name: String,
    expression: String,
    input: Json,
    expected: Option<Json>,
}

fn corpus() -> Vec<Case> {
    let mut files: Vec<PathBuf> = fs::read_dir(corpus_dir())
        .expect("the cel-conformance corpus directory exists")
        .map(|entry| entry.expect("readable directory entry").path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect();
    files.sort();

    let mut cases = Vec::new();
    for file in files {
        let name = file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default()
            .to_string();
        let source = fs::read_to_string(&file).expect("a readable corpus file");
        let entries: Vec<Json> = serde_json::from_str(&source)
            .unwrap_or_else(|error| panic!("{name} is not a JSON array of cases: {error}"));
        for entry in entries {
            let object = entry
                .as_object()
                .unwrap_or_else(|| panic!("{name}: a case is a JSON object"));
            let string = |key: &str| {
                object
                    .get(key)
                    .and_then(Json::as_str)
                    .unwrap_or_else(|| panic!("{name}: a case declares a string `{key}`"))
                    .to_string()
            };
            let expects_error = object.get("error").and_then(Json::as_bool) == Some(true);
            let expected = object.get("result").cloned();
            assert!(
                expects_error != expected.is_some(),
                "{name}: a case declares exactly one of `result` and `error: true`"
            );
            cases.push(Case {
                file: name.clone(),
                name: string("name"),
                expression: string("expression"),
                input: object.get("input").cloned().unwrap_or(Json::Null),
                expected,
            });
        }
    }
    cases
}

/// One JSON value as the CEL value the corpus means by it.
fn value(json: &Json) -> Value {
    match json {
        Json::Null => Value::Null,
        Json::Bool(value) => Value::Bool(*value),
        Json::Number(number) => number.as_i64().map_or_else(
            || Value::Float(number.as_f64().expect("a JSON number is i64 or f64")),
            Value::Int,
        ),
        Json::String(text) => Value::String(Arc::new(text.clone())),
        Json::Array(items) => Value::List(Arc::new(items.iter().map(value).collect())),
        Json::Object(entries) => {
            let map: HashMap<Key, Value> = entries
                .iter()
                .map(|(key, entry)| (Key::String(Arc::new(key.clone())), value(entry)))
                .collect();
            Value::Map(Map { map: Arc::new(map) })
        }
    }
}

/// One CEL value as JSON, for comparison against a case's `result`.
fn json(value: &Value) -> Json {
    match value {
        Value::Null => Json::Null,
        Value::Bool(value) => Json::Bool(*value),
        Value::Int(number) => Json::from(*number),
        Value::UInt(number) => Json::from(*number),
        Value::Float(number) => Json::from(*number),
        Value::String(text) => Json::String(text.to_string()),
        Value::List(items) => Json::Array(items.iter().map(json).collect()),
        Value::Map(map) => Json::Object(
            map.map
                .iter()
                .map(|(key, entry)| {
                    let key = match key {
                        Key::String(text) => text.to_string(),
                        Key::Int(number) => number.to_string(),
                        Key::Uint(number) => number.to_string(),
                        Key::Bool(value) => value.to_string(),
                    };
                    (key, json(entry))
                })
                .collect(),
        ),
        other => panic!("a corpus case produced a value with no JSON form: {other:?}"),
    }
}

#[test]
fn every_case_evaluates_to_what_it_declares() {
    let mut failures = Vec::new();
    for case in corpus() {
        let label = format!("{}: {}", case.file, case.name);
        let program = match Program::compile(&case.expression) {
            Ok(program) => program,
            Err(error) => {
                failures.push(format!("{label}\n  does not parse: {error}"));
                continue;
            }
        };
        let mut context = Context::default();
        if let Json::Object(roots) = &case.input {
            for (name, root) in roots {
                context.add_variable_from_value(name.clone(), value(root));
            }
        }
        match (program.execute(&context), &case.expected) {
            (Ok(actual), Some(expected)) => {
                let actual = json(&actual);
                if &actual != expected {
                    failures.push(format!(
                        "{label}\n  expected: {expected}\n  actual:   {actual}"
                    ));
                }
            }
            (Ok(actual), None) => failures.push(format!(
                "{label}\n  expected an evaluation error, got {}",
                json(&actual)
            )),
            (Err(error), Some(expected)) => {
                failures.push(format!(
                    "{label}\n  expected {expected}, got the error: {error}"
                ));
            }
            (Err(_), None) => {}
        }
    }
    assert!(
        failures.is_empty(),
        "{} conformance case(s) diverged:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn every_case_is_named_once() {
    let mut seen: HashMap<String, String> = HashMap::new();
    for case in corpus() {
        if let Some(other) = seen.insert(case.name.clone(), case.file.clone()) {
            panic!(
                "`{}` is declared in both {} and {}",
                case.name, case.file, other
            );
        }
    }
}

/// The one divergence the corpus deliberately does not pin, pinned here
/// instead.
///
/// `size(string)` is the CEL specification's count of **code points**; `cel`
/// 0.14.3 returns the count of **bytes**, and a JS evaluator over
/// `String.prototype.length` would return UTF-16 code units — three answers for
/// `'👍'` (1, 4, 2). `fixtures/cel-conformance/README.md` records why no corpus
/// case states it: pinning the specification's answer leaves a red test, and
/// pinning the crate's institutionalises the defect in the artifact whose whole
/// job is to keep two implementations honest.
///
/// A paragraph is not a check, though, and the note has to be *acted on*
/// before the JS evaluator lands. This test is what makes it impossible to
/// forget: it pins what the pinned crate does today, so an upstream fix or a
/// version bump turns it red and sends whoever bumped it back to the README —
/// where the answer is either a corpus case at last, or a `size` overload this
/// project supplies.
#[test]
fn the_size_of_a_non_ascii_string_still_diverges_from_the_specification() {
    let size = |source: &str| {
        let program = Program::compile(source).expect("the expression parses");
        match program
            .execute(&Context::default())
            .expect("the expression evaluates")
        {
            Value::Int(count) => count,
            other => panic!("`{source}` is not an integer: {other:?}"),
        }
    };
    // Specification: 5 and 1. `String.prototype.length`: 5 and 2.
    assert_eq!(size("size('héllo')"), 6, "cel 0.14.3 counted bytes");
    assert_eq!(size("size('👍')"), 4, "cel 0.14.3 counted bytes");
    // ASCII is where all three agree, which is why the corpus states only it.
    assert_eq!(size("size('abc')"), 3);
}

/// The second recorded gap, for the same reason and with the same tripwire.
///
/// CEL's standard definitions give `matches` two overloads — `s.matches(p)` and
/// the global `matches(s, p)` — and `cel` 0.14.3 implements only the first. The
/// compiler's own front-end accepts both, because grammar 4.1 puts the standard
/// function set on the surface and the specification is what defines it; that
/// leaves one expression `validate` accepts and the *validator's* evaluator
/// cannot run, which is a fact worth a red test the day it stops being true.
#[test]
fn the_global_spelling_of_matches_is_still_missing_from_the_crate() {
    let program = Program::compile("matches('a1', '^a[0-9]$')").expect("the expression parses");
    assert!(
        program.execute(&Context::default()).is_err(),
        "cel 0.14.3 gained the global `matches` overload: state it in strings.json"
    );
    // The receiver spelling, which both implementations have, is in the corpus.
    let program = Program::compile("'a1'.matches('^a[0-9]$')").expect("the expression parses");
    assert_eq!(
        program.execute(&Context::default()).expect("it evaluates"),
        Value::Bool(true)
    );
}

/// The corpus is the artifact, not this test: it has to be substantial enough
/// to be worth running a second implementation against.
#[test]
fn the_corpus_is_substantial() {
    let cases = corpus();
    assert!(
        cases.len() >= 60,
        "the conformance corpus must cover at least 60 cases, found {}",
        cases.len()
    );
    let errors = cases.iter().filter(|case| case.expected.is_none()).count();
    assert!(
        errors >= 5,
        "the corpus must pin the failing cases too, found {errors}"
    );
}
