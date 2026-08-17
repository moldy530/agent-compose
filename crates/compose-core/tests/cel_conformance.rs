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
//! A case that must *fail* writes `"error": true` instead of `"result"`. What
//! is pinned is that the expression produces no value, never the message or the
//! stage: the two implementations are held to the same accept/reject decision
//! and to the same values, not to one another's wording. **Refusing to parse
//! counts** — `'\0'` is an escape neither CEL nor either implementation has, and
//! the Rust column reports that from `Program::compile` where the JS column
//! reports it from its own lexer. Pinning only the *evaluation* errors would
//! leave every lexical rule unstatable in the artifact whose job is to keep the
//! two readings together, which is how the escape table came to disagree.
//! A case declaring `"result"` still has to parse: there, a compile failure is
//! a divergence like any other.
//!
//! Plain JSON data files, one loader per implementation: the **fixtures**
//! import nothing and describe nothing but data, so the corpus stays portable
//! to a runner written in another language — `crates/agent-compose/tests/
//! compiled_graph_acceptance/cel-conformance.mjs` is the second one, over the
//! evaluator a generated project embeds.
//!
//! The runner does reach into this crate for one thing:
//! [`compose_core::cel::evaluation_context`], the context the Rust column
//! evaluates under. That decision — which standard overloads this project
//! supplies where the pinned crate is missing one — belongs to the project
//! rather than to a test, or the two runs of "the Rust column" in this
//! repository could be configured differently.
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
            // A refusal is a refusal: a case that declares `error: true` is
            // satisfied by one, and only a case expecting a value is failed by
            // it. See the module comment.
            Err(error) => {
                if case.expected.is_some() {
                    failures.push(format!("{label}\n  does not parse: {error}"));
                }
                continue;
            }
        };
        let mut context = compose_core::cel::evaluation_context();
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
/// `'👍'` (1, 4, 2). The emitted evaluator spreads the string, so it answers the
/// specification's; `fixtures/cel-conformance/README.md` records why no corpus
/// case states the pair, why the `matches` fix does not transfer (a registered
/// function does not override a built-in), and why the gap is safe to leave —
/// the compiler never evaluates, so the crate is the corpus's reference column
/// rather than a runtime.
///
/// A paragraph is not a check. This test pins what the pinned crate does today,
/// so an upstream fix or a version bump turns it red and sends whoever bumped it
/// back to the README — where the answer is a corpus case at last.
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

/// The second recorded gap, **closed** — and pinned here so it stays closed for
/// the reason it was closed rather than by accident.
///
/// CEL's standard definitions give `matches` two overloads — `s.matches(p)` and
/// the global `matches(s, p)` — and `cel` 0.14.3 implements only the first. The
/// compiler's own front-end accepts both, because grammar 4.1 puts the standard
/// function set on the surface and the specification is what defines it, which
/// left one expression `validate` accepts and this project's Rust evaluator
/// could not run. [`compose_core::cel::evaluation_context`] supplies the missing
/// overload — the crate's *own* implementation registered under the global name,
/// so the two spellings cannot answer differently — and `strings.json` now
/// states both.
///
/// Two claims, because either could stop being true on its own: the stock
/// context still lacks the overload (so the registration is doing work rather
/// than shadowing something), and the shared context has it.
#[test]
fn the_global_spelling_of_matches_is_supplied_where_the_crate_lacks_it() {
    let program = Program::compile("matches('a1', '^a[0-9]$')").expect("the expression parses");
    assert!(
        program.execute(&Context::default()).is_err(),
        "cel 0.14.3 gained the global `matches` overload: `evaluation_context` no longer has to \
         supply it, and the registration can go"
    );
    assert_eq!(
        program
            .execute(&compose_core::cel::evaluation_context())
            .expect("the shared context supplies it"),
        Value::Bool(true)
    );
    // The receiver spelling is the one the crate carries, and it still answers
    // the same under the shared context.
    let program = Program::compile("'a1'.matches('^a[0-9]$')").expect("the expression parses");
    assert_eq!(
        program
            .execute(&compose_core::cel::evaluation_context())
            .expect("it evaluates"),
        Value::Bool(true)
    );
}

/// The third recorded gap: the constructs a `matches()` pattern may carry that
/// **both** engines accept and read differently.
///
/// `codegen::diagnostics` refuses a `matches()` pattern only one of the two
/// engines can *parse* — inline flags, `(?P<…>)`, look-around, a backreference —
/// so what is left is the patterns both parse. Four constructs among those still
/// mean different things, and every one of them is the same difference: the
/// crate's engine is **Unicode-aware** and `RegExp` without a `u` flag is not.
///
/// * `.` is one **code point** here and one UTF-16 **code unit** there, and it
///   excludes LF here where ECMA-262 excludes every line terminator;
/// * `\b` is a boundary between **Unicode** word characters here and ASCII ones
///   there;
/// * `\w` and `\d` are the **Unicode** classes here and `[A-Za-z0-9_]` and
///   `[0-9]` there. This is the one difference `codegen::schema`'s ledger does
///   **not** already carry: a `pattern:` is read by a JSON Schema validator,
///   which translates the Perl classes because JSON Schema *defines* `pattern`
///   as ECMA-262 — so `^\d+$` refuses `١٢٣` in both of those columns. Nothing
///   translates for `matches()`: the crate runs `regex` directly.
///
/// No corpus case can state any of them — the columns disagree, which is what a
/// corpus case is forbidden to record — so this pins **this** column's answers
/// and `codegen::cel`'s ledger carries the rows. The emitted evaluator's answers
/// are written beside each one; they are `RegExp`'s, and
/// `crates/compose-core/src/codegen/pattern.rs` says why no flag closes the gap.
#[test]
fn the_unicode_aware_constructs_of_a_matches_pattern_still_read_differently() {
    let answer = |source: &str| {
        let program = Program::compile(source).expect("the expression parses");
        match program
            .execute(&compose_core::cel::evaluation_context())
            .expect("the expression evaluates")
        {
            Value::Bool(value) => value,
            other => panic!("`{source}` is not a bool: {other:?}"),
        }
    };
    // `/^.$/.test("👍")` is `false`: two code units, one `.`.
    assert!(answer("'👍'.matches('^.$')"), "one code point, one `.`");
    // `/^.$/.test("\r")` is `false`: CR is a line terminator to ECMA-262.
    assert!(answer("'\\r'.matches('^.$')"), "`.` excludes LF alone here");
    // `/\bcat\b/.test("caté")` is `true`: `é` is not an ASCII word character.
    assert!(
        !answer("'caté'.matches('\\\\bcat\\\\b')"),
        "`\\b` is a boundary between Unicode word characters here"
    );
    // `/^\w$/.test("é")` and `/^\d$/.test("١")` are both `false`.
    assert!(
        answer("'é'.matches('^\\\\w$')"),
        "`\\w` is the Unicode word class here"
    );
    assert!(
        answer("'١'.matches('^\\\\d$')"),
        "`\\d` is the Unicode decimal class here"
    );
    // ASCII, with the classes written out, is where the two agree — which is
    // what the corpus states and what the property harness generates.
    assert!(answer("'a1'.matches('^[a-z][0-9]$')"));
}

/// The fourth recorded gap, and the only one the **escape table** has left:
/// a quote escaped inside the *other* quote's literal.
///
/// CEL's escape table admits `\"` and `\'` in either kind of literal and gives
/// each one meaning — the quote. `cel` 0.14.3 keeps the backslash for the
/// redundant spelling (`parse_quoted_string` sets `push_escape_character` when
/// the escaped quote is not the literal's own), so `'\"'` is **two** characters
/// there and one by the specification. The emitted evaluator reads the
/// specification's, which is the same choice `size-of-a-string-counts-code-points`
/// records and for the same reason: the compiler never evaluates, so the crate
/// is the corpus's reference column rather than a runtime.
///
/// No corpus case can state it — a case is forbidden to record a disagreement —
/// so `escapes.json` states the two *non*-redundant spellings, where the columns
/// agree, and the divergence is pinned here and carried as
/// `a-cross-quote-escape-keeps-its-backslash` in `codegen::cel`'s ledger.
#[test]
fn a_quote_escaped_inside_the_other_quotes_literal_still_keeps_its_backslash() {
    let text = |source: &str| {
        let program = Program::compile(source).expect("the expression parses");
        match program
            .execute(&compose_core::cel::evaluation_context())
            .expect("the expression evaluates")
        {
            Value::String(text) => text.to_string(),
            other => panic!("`{source}` is not a string: {other:?}"),
        }
    };
    // The emitted evaluator answers `"` and `'`: one character each.
    assert_eq!(text(r#"'\"'"#), "\\\"", "cel 0.14.3 kept the backslash");
    assert_eq!(text(r#""\'""#), "\\'", "cel 0.14.3 kept the backslash");
    // A quote escaped inside its **own** literal is where the two agree, which
    // is what `escapes.json` states.
    assert_eq!(text(r"'\''"), "'");
    assert_eq!(text(r#""\"""#), "\"");
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
