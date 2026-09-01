//! Two columns, checked against each other over compositions nobody wrote.
//!
//! This project keeps two pairs of implementations in lockstep, and each pair
//! has a hand-written corpus proving they agree on the cases somebody thought
//! of:
//!
//! * grammar 3.8's **schema** table — the JSON Schema the compiler publishes and
//!   the Zod it emits — pinned by `tests/fixtures/schema-lowering/cases.json`;
//! * the two **CEL** implementations — the pinned `cel` crate and the evaluator
//!   `src/cel.ts` embeds — pinned by `tests/fixtures/cel-conformance/`.
//!
//! A hand-written corpus finds the divergence its author already suspected. This
//! finds the other kind: a **seeded generator** builds compositions, documents
//! and guard expressions, and both columns answer them. Every divergence the
//! reviewer-found class has produced so far — a constraint one column enforces
//! and the other does not, a numeric type one folds — is a thing this shape of
//! test finds by itself.
//!
//! # Determinism, and the wider run
//!
//! Every case is derived from a `u64` seed through a PRNG carried here (see
//! [`Rng`]), so a failure is reproducible from the seed printed in its message
//! and nothing depends on the machine, the clock, or a crate. CI runs
//! [`SEEDS`] of them; `AGENT_COMPOSE_PROPERTY_SEEDS=<n>` runs `n` instead, which
//! is what an afternoon of soak testing looks like.
//!
//! # Why the generator is conservative
//!
//! It emits only compositions the validator accepts, because a generated
//! composition the compiler *refuses* proves nothing about two columns that
//! never see it. Widening it is how this file grows: every constraint keyword
//! added to the generator is one more thing the two columns have to agree about
//! on inputs nobody chose.
//!
//! # Why one JavaScript engine
//!
//! The JS column runs under **Bun** only, and that is a decision rather than an
//! oversight. PRD §9.18 keeps Node a supported fallback, and what an emitted
//! `format:` regex or `src/cel.ts`'s `BigInt` arithmetic answers really does
//! belong to the engine — but the axis this file explores is the *Rust* column
//! against the JS one over shapes nobody wrote, not one engine against another.
//! The engine axis is covered where it is cheap and total: gate 15 of
//! `tests/generated_code_gates.rs` answers both hand-written corpora — every
//! spelling grammar 3.8 and grammar 4.1 have — under Node as well. Adding a
//! second runtime here would double a twelve-seed build-and-run and need a second
//! dependency install in a binary cargo already runs in parallel with those
//! gates, which is a worse trade than the gap it closes. Revisit it together with
//! gate 15's own *What is deliberately not re-run here*.

#[path = "support/goldens.rs"]
mod goldens;
#[path = "support/toolchain.rs"]
mod toolchain;

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use compose_core::codegen::{cel as codegen_cel, schema as codegen_schema};
use serde_json::{Value, json};
// The JS column runs under Bun against the same install the gates use, so both
// suites answer it with one pinned dependency set under one runtime, and the
// skip-locally/fail-in-CI rule is stated once. See `support/toolchain.rs`.
use toolchain::installed;

/// How many seeds a plain `cargo test` runs.
///
/// Small enough to stay inside the suite's time budget and large enough that a
/// one-in-a-few-shapes divergence is not a coin flip. `AGENT_COMPOSE_PROPERTY_SEEDS`
/// is the knob for a longer run.
const SEEDS: u64 = 12;

/// How many documents each generated channel is answered with.
const DOCUMENTS: usize = 6;

/// How many guard expressions each generated composition is answered with.
const EXPRESSIONS: usize = 12;

fn seeds() -> u64 {
    std::env::var("AGENT_COMPOSE_PROPERTY_SEEDS")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(SEEDS)
}

// ---------------------------------------------------------------------------
// The generator
// ---------------------------------------------------------------------------

/// SplitMix64: three lines, no dependency, and the same stream on every machine.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A number in `0..bound`.
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }

    fn pick<'a, T>(&mut self, choices: &'a [T]) -> &'a T {
        &choices[self.below(choices.len())]
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.next() % 100 < percent
    }
}

/// One generated channel: how it is written in the spec, and what its values
/// look like.
struct Channel {
    name: String,
    declaration: String,
    kind: Kind,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Glyph,
    Count,
    Flag,
    Choice,
    List,
    Shape,
}

/// A composition, generated from one seed.
struct Composition {
    source: String,
    channels: Vec<Channel>,
}

fn compose(seed: u64) -> Composition {
    let mut rng = Rng(seed);
    let count = 3 + rng.below(4);
    let mut channels = Vec::new();
    for index in 0..count {
        let name = format!("c{index}");
        let (declaration, kind) = match rng.below(7) {
            0 => {
                let mut keys = vec!["type: string".to_string()];
                if rng.chance(50) {
                    keys.push(format!("min_length: {}", 1 + rng.below(3)));
                }
                if rng.chance(50) {
                    keys.push(format!("max_length: {}", 6 + rng.below(6)));
                }
                (format!("{{ {} }}", keys.join(", ")), Kind::Text)
            }
            1 => {
                let mut keys = vec!["type: integer".to_string()];
                if rng.chance(60) {
                    keys.push(format!("minimum: {}", rng.below(3)));
                }
                if rng.chance(60) {
                    keys.push(format!("maximum: {}", 5 + rng.below(10)));
                }
                (format!("{{ {} }}", keys.join(", ")), Kind::Count)
            }
            // A string channel whose values reach past ASCII. It is a `Kind` of
            // its own rather than a wider `Kind::Text` because the two take
            // different **guards**: `size()` over a non-ASCII string is
            // `codegen::cel`'s declared `size-of-a-string-counts-code-points`
            // row, and a generator that produced it would re-find a divergence
            // somebody already signed off on instead of finding a new one.
            2 => ("{ type: string }".to_string(), Kind::Glyph),
            3 => ("{ type: boolean }".to_string(), Kind::Flag),
            4 => ("{ enum: [low, high, urgent] }".to_string(), Kind::Choice),
            5 => (
                format!(
                    "{{ type: array, max_items: {}, items: {{ type: string }} }}",
                    2 + rng.below(4)
                ),
                Kind::List,
            ),
            _ => (
                "{ type: object, properties: { at: { type: string }, n: { type: integer } }, \
                 optional: [n] }"
                    .to_string(),
                Kind::Shape,
            ),
        };
        channels.push(Channel {
            name,
            declaration,
            kind,
        });
    }

    let mut source = String::from("version: \"0.1\"\n\nstate:\n");
    for channel in &channels {
        source.push_str(&format!("  {}: {}\n", channel.name, channel.declaration));
    }
    Composition { source, channels }
}

/// The documents one channel is answered with: values it should accept, and
/// values it should not.
fn documents(rng: &mut Rng, channel: &Channel) -> Vec<Value> {
    let mut found = Vec::new();
    for _ in 0..DOCUMENTS {
        found.push(match channel.kind {
            Kind::Text => match rng.below(5) {
                0 => json!(""),
                1 => json!("a"),
                2 => json!("a moderate string"),
                3 => json!("x".repeat(1 + rng.below(14))),
                _ => json!(rng.below(4)),
            },
            Kind::Glyph => match rng.below(5) {
                0 => json!(""),
                1 => json!("héllo"),
                2 => json!("\u{FFFD}"),
                3 => json!("😀"),
                _ => json!(rng.below(4)),
            },
            Kind::Count => match rng.below(5) {
                0 => json!(0),
                1 => json!(rng.below(20)),
                2 => json!(-1),
                3 => json!("3"),
                _ => json!(1.5),
            },
            Kind::Flag => match rng.below(3) {
                0 => json!(true),
                1 => json!(false),
                _ => json!("true"),
            },
            Kind::Choice => match rng.below(4) {
                0 => json!("low"),
                1 => json!("high"),
                2 => json!("urgent"),
                _ => json!("unheard_of"),
            },
            Kind::List => match rng.below(4) {
                0 => json!([]),
                1 => json!(["a"]),
                2 => json!(["a", "b", "c", "d", "e"]),
                _ => json!(["a", 1]),
            },
            Kind::Shape => match rng.below(5) {
                0 => json!({ "at": "here" }),
                1 => json!({ "at": "here", "n": 2 }),
                2 => json!({ "n": 2 }),
                3 => json!({ "at": "here", "extra": true }),
                _ => json!({ "at": 1 }),
            },
        });
    }
    found
}

/// Text values a generated guard reads: **four ASCII, then four that are not**.
///
/// The split is what the two `Kind`s read: a `Kind::Text` channel takes the
/// first four, and a `Kind::Glyph` channel takes all eight. The second half is
/// not decoration. A string comparison reads UTF-16 **code units** in JavaScript
/// and UTF-8 **bytes** in Rust, and the two orders disagree in exactly one
/// place: a code point at or above U+10000 is a surrogate pair whose leading
/// unit is below U+E000, so `<` sorts every astral character under every BMP
/// character in U+E000..U+FFFF. A generator that produced only ASCII would never
/// ask the question — which is how the divergence `compareStrings` closes
/// survived the hand-written corpus.
const TEXTS: &[&str] = &[
    "",
    "a",
    "a draft",
    "# heading",
    "héllo",
    "\u{E000}",
    "\u{FFFD}",
    "😀",
];

/// A value of the channel's declared type, for the roots a guard reads.
fn conforming(rng: &mut Rng, channel: &Channel) -> Value {
    match channel.kind {
        Kind::Text => json!(rng.pick(&TEXTS[..4])),
        Kind::Glyph => json!(rng.pick(TEXTS)),
        Kind::Count => json!(rng.below(6)),
        Kind::Flag => json!(rng.chance(50)),
        Kind::Choice => json!(["low", "high", "urgent"][rng.below(3)]),
        Kind::List => json!(vec!["a"; rng.below(3)]),
        Kind::Shape => {
            if rng.chance(50) {
                json!({ "at": "here", "n": rng.below(4) })
            } else {
                json!({ "at": "here" })
            }
        }
    }
}

/// The `matches()` patterns a generated guard may carry.
///
/// The subset both regex engines read the same way: the crate's is `regex` and
/// the emitted evaluator's is `RegExp`, and every construct where they disagree
/// is either refused by `codegen::diagnostics` (inline flags, `(?P<…>)`,
/// look-around, a backreference) or a declared row in `codegen::cel`'s ledger
/// (`.`, `\b`, and the Perl classes over non-ASCII). Neither belongs in a
/// generator whose whole job is to find the divergences nobody declared, so what
/// is left is written out here rather than assembled from pieces.
const PATTERNS: &[&str] = &[
    "^a",
    "ft$",
    "^[a-z ]+$",
    "^[^0-9]*$",
    "(a|#) ?",
    "^(?:a|h)",
    "a{1,3}",
    "^(?<first>[a-z])",
    "\\+|\\*",
    "",
];

/// One generated guard: CEL over `state`, of the shape a router evaluates.
fn guard(rng: &mut Rng, channels: &[Channel]) -> String {
    let channel = rng.pick(channels);
    let path = format!("state.{}", channel.name);
    let atom = match channel.kind {
        Kind::Text => match rng.below(5) {
            0 => format!("{path} == ''"),
            1 => format!("{path} != ''"),
            2 => format!("size({path}) > {}", rng.below(4)),
            3 => format!("{path}.startsWith('a')"),
            _ => format!("{path}.contains('draft')"),
        },
        // The string operations whose two readings can part company, over the
        // channel whose values reach past ASCII. No `size()` among them: that
        // pair is `codegen::cel`'s declared `size-of-a-string-counts-code-points`
        // row, which the corpus states no case for and this must not generate
        // one for either.
        Kind::Glyph => match rng.below(7) {
            0 => format!("{path} == '{}'", rng.pick(TEXTS)),
            1 => format!("{path} != ''"),
            2 => format!("{path}.startsWith('{}')", rng.pick(&TEXTS[4..])),
            3 => format!("{path}.contains('{}')", rng.pick(&TEXTS[4..])),
            // Ordering: JavaScript compares UTF-16 code units and Rust compares
            // UTF-8 bytes, and the two orders disagree for exactly these values.
            4 => format!("{path} < '{}'", rng.pick(&TEXTS[4..])),
            5 => format!("{path} >= '{}'", rng.pick(&TEXTS[4..])),
            // `matches()`, over the regex vocabulary both engines read alike.
            // Every pattern is anchors, written-out classes, alternation,
            // repetition, groups and escaped metacharacters — and **no** `.`,
            // `\b`, `\w`, `\d` or `\s`, which are declared rows in the same
            // ledger rather than things a generator may produce.
            _ => format!("{path}.matches('{}')", rng.pick(PATTERNS)),
        },
        Kind::Count => match rng.below(5) {
            0 => format!("{path} > {}", rng.below(6)),
            1 => format!("{path} <= {}", rng.below(6)),
            2 => format!("{path} == {}", rng.below(6)),
            3 => format!("{path} + 1 > {}", rng.below(6)),
            _ => format!("{path} % 2 == 0"),
        },
        Kind::Flag => match rng.below(2) {
            0 => path.clone(),
            _ => format!("!{path}"),
        },
        Kind::Choice => match rng.below(4) {
            0 => format!("{path} == 'low'"),
            1 => format!("{path} != 'urgent'"),
            2 => format!("{path} in ['low', 'high']"),
            _ => format!("!({path} == 'high')"),
        },
        Kind::List => match rng.below(4) {
            0 => format!("size({path}) > 0"),
            1 => format!("'a' in {path}"),
            2 => format!("{path}.exists(item, item == 'a')"),
            _ => format!("{path}.all(item, size(item) > 0)"),
        },
        Kind::Shape => match rng.below(4) {
            0 => format!("has({path}.n)"),
            1 => format!("has({path}.n) && {path}.n > 1"),
            2 => format!("{path}.at == 'here'"),
            // A read that legitimately fails when the optional property is
            // absent: both columns must fail it, rather than one answering
            // `false` (Decision D110).
            _ => format!("{path}.n > 0"),
        },
    };
    match rng.below(4) {
        0 => atom,
        1 => format!("!({atom})"),
        2 => format!("({atom}) && {}", rng.chance(50)),
        _ => format!("({atom}) || {}", rng.chance(20)),
    }
}

// ---------------------------------------------------------------------------
// The two columns
// ---------------------------------------------------------------------------

/// Emit one generated composition into a project the JS column can import.
fn project(root: &Path, purpose: &str, seed: u64, source: &str) -> (PathBuf, compose_core::Ir) {
    let scratch = std::env::temp_dir().join(format!(
        "agent-compose-property-{}-{purpose}-{seed}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&scratch);
    fs::create_dir_all(&scratch).expect("a scratch directory");
    let entrypoint = scratch.join("main.yml");
    fs::write(&entrypoint, source).expect("the entrypoint is writable");

    let resolution = compose_core::resolve(&entrypoint);
    assert!(
        resolution.diagnostics.is_empty(),
        "seed {seed} generated a composition that does not resolve:\n{source}\n{:#?}",
        resolution.diagnostics
    );
    let ir = resolution.ir.expect("a clean resolution has an artifact");
    let refused = compose_core::check(&ir);
    assert!(
        refused.is_empty(),
        "seed {seed} generated a composition the validator refuses:\n{source}\n{refused:#?}"
    );
    assert!(
        compose_core::codegen::diagnostics(&ir).is_empty(),
        "seed {seed} generated a composition this target cannot express:\n{source}"
    );

    let out = root
        .join("projects")
        .join(format!("property-{purpose}-{seed}"));
    let _ = fs::remove_dir_all(&out);
    for file in compose_core::emit(&ir, &compose_core::Authored::none()).files() {
        let path = out.join(file.path.replace('/', std::path::MAIN_SEPARATOR_STR));
        fs::create_dir_all(path.parent().expect("a generated path has a parent"))
            .expect("the scratch area is writable");
        fs::write(&path, &file.contents).expect("the generated file is writable");
    }
    let _ = fs::remove_dir_all(&scratch);
    (out, ir)
}

/// The Rust column's verdict on one document.
fn json_column(schema: &Value, document: &Value) -> bool {
    jsonschema::options()
        .should_validate_formats(true)
        .build(schema)
        .expect("the lowering is a valid schema")
        .is_valid(document)
}

/// The Rust column's answer to one guard.
fn rust_guard(source: &str, roots: &serde_json::Map<String, Value>) -> Result<bool, String> {
    let program = cel::Program::compile(source).map_err(|error| format!("{error:?}"))?;
    let mut context = compose_core::cel::evaluation_context();
    for (name, value) in roots {
        context.add_variable_from_value(name.clone(), cel_value(value));
    }
    match program.execute(&context) {
        Ok(cel::Value::Bool(answer)) => Ok(answer),
        Ok(other) => Err(format!("not a bool: {other:?}")),
        Err(error) => Err(format!("{error}")),
    }
}

/// One JSON value as the CEL value the corpus's own rule means by it: an
/// integral number is an `int`, everything else a `double`.
fn cel_value(json: &Value) -> cel::Value {
    match json {
        Value::Null => cel::Value::Null,
        Value::Bool(value) => cel::Value::Bool(*value),
        Value::Number(number) => number.as_i64().map_or_else(
            || cel::Value::Float(number.as_f64().expect("a JSON number is i64 or f64")),
            cel::Value::Int,
        ),
        Value::String(text) => cel::Value::String(std::sync::Arc::new(text.clone())),
        Value::Array(items) => {
            cel::Value::List(std::sync::Arc::new(items.iter().map(cel_value).collect()))
        }
        Value::Object(entries) => cel::Value::Map(cel::objects::Map {
            map: std::sync::Arc::new(
                entries
                    .iter()
                    .map(|(key, value)| {
                        (
                            cel::objects::Key::String(std::sync::Arc::new(key.clone())),
                            cel_value(value),
                        )
                    })
                    .collect(),
            ),
        }),
    }
}

// ---------------------------------------------------------------------------
// The experiment
// ---------------------------------------------------------------------------

/// Grammar 3.8's two columns answer every generated document the same way.
///
/// The hand-written corpus (`tests/fixtures/schema-lowering/cases.json`) pins the
/// documents somebody chose, divergence by divergence. This asks the same
/// question of documents nobody chose, over compositions nobody wrote — and it
/// is the same question, so a divergence here is either a bug or a row the
/// ledger is missing.
#[test]
fn the_two_schema_columns_answer_generated_documents_the_same_way() {
    let Some(root) = installed() else {
        return;
    };
    let mut divergences: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for seed in 0..seeds() {
        let composition = compose(seed);
        let (out, ir) = project(root, "schemas", seed, &composition.source);
        let names = compose_core::codegen::names::Names::of(&ir);
        let mut rng = Rng(seed ^ 0x5EED_D0C5);

        let mut cases = Vec::new();
        let mut expected: Vec<Vec<bool>> = Vec::new();
        for channel in &composition.channels {
            let path = format!("state.{}", channel.name);
            let surfaces = codegen_schema::surfaces(&ir);
            let surface = surfaces
                .iter()
                .find(|surface| surface.path == path)
                .expect("every channel is a surface");
            let schema = match &surface.body {
                codegen_schema::Body::Type(ty) => codegen_schema::json_type_node(ty),
                codegen_schema::Body::Fields(fields) => codegen_schema::json_field_map(fields),
            };
            let written = documents(&mut rng, channel);
            expected.push(
                written
                    .iter()
                    .map(|document| json_column(&schema, document))
                    .collect(),
            );
            cases.push(json!({
                "export": names.value(&path),
                "documents": written
                    .iter()
                    .map(|document| serde_json::to_string(document).expect("a document serializes"))
                    .collect::<Vec<_>>(),
            }));
        }

        let answered = answer(&out, &json!({ "schemas": cases }));
        let verdicts: Vec<Vec<bool>> =
            serde_json::from_value(answered["schemas"].clone()).expect("one verdict per document");

        for (index, channel) in composition.channels.iter().enumerate() {
            for (document, (json, zod)) in expected[index].iter().zip(&verdicts[index]).enumerate()
            {
                checked += 1;
                if json != zod {
                    divergences.push(format!(
                        "seed {seed}, channel `{}` ({}), document {document}: \
                         the JSON column {} it and the Zod column {} it",
                        channel.name,
                        channel.declaration,
                        if *json { "accepts" } else { "rejects" },
                        if *zod { "accepts" } else { "rejects" },
                    ));
                }
            }
        }
        let _ = fs::remove_dir_all(&out);
    }

    assert!(checked > 0, "the generator produced no documents");
    assert!(
        divergences.is_empty(),
        "{} of {checked} generated document(s) split grammar 3.8's two columns:\n\n{}",
        divergences.len(),
        divergences.join("\n")
    );
}

/// The two CEL implementations answer every generated guard the same way.
///
/// Guards are the shape that matters: PRD 5.3 makes the router's answer the
/// transition, so a guard the two evaluators disagree about is a compiled graph
/// that takes one branch and validates as taking another. Failing the same way
/// counts as agreeing — a read of an absent `optional:` property fails the
/// execution (Decision D110), and a column that answered `false` there instead
/// would be the divergence.
#[test]
fn the_two_cel_implementations_answer_generated_guards_the_same_way() {
    let Some(root) = installed() else {
        return;
    };
    let mut divergences: Vec<String> = Vec::new();
    let mut checked = 0usize;

    for seed in 0..seeds() {
        let composition = compose(seed);
        let (out, ir) = project(root, "guards", seed, &composition.source);
        let mut rng = Rng(seed ^ 0xC0FF_EE00);

        // One `state` root per composition, and the shape the emitted graph
        // would read it through — the compiler's own lowering rather than one
        // this test derives, or the thing under test would be a copy of itself.
        let mut state = serde_json::Map::new();
        let mut properties = serde_json::Map::new();
        for channel in &composition.channels {
            state.insert(channel.name.clone(), conforming(&mut rng, channel));
            let ty = &ir
                .state
                .as_ref()
                .expect("the composition declares channels")
                .entries[&channel.name]
                .ty;
            properties.insert(channel.name.clone(), codegen_cel::shape_json(ty));
        }
        let shape = json!({ "properties": properties });

        let mut sources = Vec::new();
        let mut cases = Vec::new();
        for _ in 0..EXPRESSIONS {
            let source = guard(&mut rng, &composition.channels);
            cases.push(json!({
                "source": source,
                "roots": { "state": { "value": Value::Object(state.clone()), "shape": shape } },
            }));
            sources.push(source);
        }

        let answered = answer(&out, &json!({ "expressions": cases }));
        let verdicts = answered["expressions"]
            .as_array()
            .expect("one verdict per expression");

        let mut roots = serde_json::Map::new();
        roots.insert("state".to_string(), Value::Object(state.clone()));
        for (index, source) in sources.iter().enumerate() {
            checked += 1;
            let rust = rust_guard(source, &roots);
            let js = &verdicts[index];
            let agreed = match (&rust, js.get("value")) {
                (Ok(answer), Some(Value::Bool(other))) => answer == other,
                (Err(_), None) => true,
                _ => false,
            };
            if !agreed {
                divergences.push(format!(
                    "seed {seed}: `{source}` over {}\n  rust: {rust:?}\n  js:   {js}",
                    Value::Object(state.clone())
                ));
            }
        }
        let _ = fs::remove_dir_all(&out);
    }

    assert!(checked > 0, "the generator produced no guards");
    assert!(
        divergences.is_empty(),
        "{} of {checked} generated guard(s) split the two CEL implementations:\n\n{}",
        divergences.len(),
        divergences.join("\n\n")
    );
}

/// Run the JS column over one generated project.
fn answer(project: &Path, cases: &Value) -> Value {
    let path = project.join("property-cases.json");
    fs::write(
        &path,
        serde_json::to_string(cases).expect("the cases serialize"),
    )
    .expect("the scratch area is writable");
    let output = toolchain::runner("property-conformance.mjs")
        .arg(project)
        .arg(&path)
        .output()
        .expect("bun runs");
    assert!(
        output.status.success(),
        "the property runner failed:\n{}",
        String::from_utf8_lossy(&output.stderr),
    );
    serde_json::from_slice(&output.stdout).expect("the runner prints one JSON object")
}

/// The generator itself is deterministic, which is what makes a seed in a
/// failure message worth anything.
#[test]
fn one_seed_generates_one_composition() {
    for seed in 0..8 {
        assert_eq!(compose(seed).source, compose(seed).source);
        let mut left = Rng(seed);
        let mut right = Rng(seed);
        let channels = compose(seed).channels;
        for _ in 0..16 {
            assert_eq!(guard(&mut left, &channels), guard(&mut right, &channels));
        }
    }
    // …and different seeds do not all generate the same one, which a broken
    // PRNG would make true while every assertion above still passed.
    let distinct: BTreeSet<String> = (0..8).map(|seed| compose(seed).source).collect();
    assert!(distinct.len() > 1, "every seed generated one composition");
}
