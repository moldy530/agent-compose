//! Conformance harness for the published editor schema
//! (`schemas/agent-compose.schema.json`, specified by `docs/grammar.md`).
//!
//! Two corpora keep the schema honest in both directions:
//!
//! * every YAML file under `examples/` must validate — the examples are the
//!   worked reference for the grammar, so a schema change that breaks them is a
//!   regression;
//! * every fixture under `tests/fixtures/invalid-schema/` must be rejected —
//!   each is a minimal file violating one named grammar rule.
//!
//! A rule the example corpus happens not to exercise is covered by neither, and
//! then only its negative half is pinned. Where that matters the accepted shape
//! is asserted directly — see
//! [`the_published_schema_accepts_a_keyless_provider_that_names_its_endpoint`].
//!
//! The schema is deliberately looser than `agent-compose validate` (no
//! cross-file reference or static-analysis checks); see `docs/grammar.md`
//! Appendix B.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::Validator;
use serde_json::{Value, json};

/// Node kind keys the grammar defines (grammar 7.1 / PRD 5.5, 5.8). The example
/// corpus is required to exercise all of them.
const NODE_KINDS: &[&str] = &[
    "agent", "exec", "http", "function", "flow", "map", "human", "store",
];

fn repo_root() -> PathBuf {
    // crates/compose-core -> crates -> <repo root>
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("manifest dir has a grandparent")
        .to_path_buf()
}

fn schema_path() -> PathBuf {
    repo_root().join("schemas/agent-compose.schema.json")
}

fn read_schema() -> Value {
    let path = schema_path();
    let text =
        fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid JSON: {e}", path.display()))
}

fn compile_schema() -> Validator {
    let schema = read_schema();
    jsonschema::validator_for(&schema)
        .unwrap_or_else(|e| panic!("{} does not compile: {e}", schema_path().display()))
}

/// Every `.yml`/`.yaml` file under `dir`, recursively, in a stable order.
fn yaml_files(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    collect_yaml(dir, &mut found);
    found.sort();
    found
}

fn collect_yaml(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read directory {}: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("readable directory entry").path();
        if path.is_dir() {
            collect_yaml(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("yml") | Some("yaml")
        ) {
            out.push(path);
        }
    }
}

fn read_yaml_as_json(path: &Path) -> Value {
    let text =
        fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    serde_yaml_ng::from_str(&text)
        .unwrap_or_else(|e| panic!("{} is not valid YAML: {e}", path.display()))
}

fn display(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

fn validation_errors(validator: &Validator, instance: &Value) -> Vec<String> {
    validator
        .iter_errors(instance)
        .map(|error| format!("  at {}: {error}", error.instance_path()))
        .collect()
}

/// The header every negative fixture must carry:
///
/// ```yaml
/// # rule: <the grammar rule the fixture violates>
/// # at: <JSON Pointer where the violation must surface, or `<root>`>
/// # via: <JSON Pointer to the schema keyword that must reject it>
/// ```
///
/// The two pointers are what keep the corpus honest. `at` alone is too weak:
/// it is satisfied by *any* error at that location, so a fixture that starts
/// failing for an unrelated reason keeps the suite green while quietly ceasing
/// to cover its rule. `via` pins the other half — the keyword doing the
/// rejecting — so the pair identifies one specific rule firing on one specific
/// value.
struct FixtureHeader {
    rule: String,
    /// JSON Pointer into the instance; the empty string is the document root.
    at: String,
    /// JSON Pointer into the schema, as `jsonschema` reports it: the location
    /// of the keyword that produced the error.
    via: String,
}

const ROOT_POINTER: &str = "<root>";

fn fixture_header(path: &Path) -> Option<FixtureHeader> {
    let text = fs::read_to_string(path).ok()?;
    let mut rule = None;
    let mut at = None;
    let mut via = None;
    for line in text.lines().take_while(|line| line.starts_with('#')) {
        if let Some(value) = line.strip_prefix("# rule:") {
            rule.get_or_insert_with(|| value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("# at:") {
            at.get_or_insert_with(|| match value.trim() {
                ROOT_POINTER => String::new(),
                pointer => pointer.to_string(),
            });
        } else if let Some(value) = line.strip_prefix("# via:") {
            via.get_or_insert_with(|| value.trim().to_string());
        }
    }
    match (rule, at, via) {
        (Some(rule), Some(at), Some(via)) if !rule.is_empty() && !via.is_empty() => {
            Some(FixtureHeader { rule, at, via })
        }
        _ => None,
    }
}

fn examples_dir() -> PathBuf {
    repo_root().join("examples")
}

fn invalid_fixtures_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/invalid-schema")
}

#[test]
fn published_schema_is_a_valid_json_schema_2020_12() {
    let schema = read_schema();
    assert_eq!(
        schema.get("$schema").and_then(Value::as_str),
        Some("https://json-schema.org/draft/2020-12/schema"),
        "the published schema must declare draft 2020-12"
    );
    if let Err(error) = jsonschema::meta::validate(&schema) {
        panic!(
            "{} is not a valid JSON Schema: {error}",
            display(&schema_path())
        );
    }
    // Compiling is a stronger check than meta-validation: it also resolves
    // every internal `$ref`.
    compile_schema();
}

/// Every `pattern` keyword in `schema`, paired with the JSON Pointer that
/// locates it, so a failure can name the offending keyword.
fn collect_patterns(node: &Value, path: &str, out: &mut Vec<(String, String)>) {
    match node {
        Value::Object(map) => {
            for (key, value) in map {
                let child = format!("{path}/{key}");
                match (key.as_str(), value.as_str()) {
                    ("pattern", Some(pattern)) => out.push((child, pattern.to_string())),
                    _ => collect_patterns(value, &child, out),
                }
            }
        }
        Value::Array(items) => {
            for (index, value) in items.iter().enumerate() {
                collect_patterns(value, &format!("{path}/{index}"), out);
            }
        }
        _ => {}
    }
}

/// Constructs outside the interoperable regex subset (Decision D108), each with
/// the reason a failure should quote.
fn non_portable_regex_construct(pattern: &str) -> Option<&'static str> {
    const LOOKAROUND: &str =
        "lookaround is outside both the ECMA-262 subset JSON Schema recommends and RE2";
    const BACKREFERENCE: &str = "a backreference is outside RE2";

    if ["(?=", "(?!", "(?<"]
        .iter()
        .any(|opener| pattern.contains(opener))
    {
        return Some(LOOKAROUND);
    }

    let bytes = pattern.as_bytes();
    for (index, byte) in bytes.iter().enumerate() {
        if *byte != b'\\' {
            continue;
        }
        // `\1`..`\9` is a group reference; `\0` is a NUL escape. A backslash
        // that is itself escaped (`\\1`) introduces neither, so the run of
        // backslashes before this one has to be even for it to count.
        let escaped_by = bytes[..index].iter().rev().take_while(|b| **b == b'\\');
        let digit = bytes.get(index + 1).copied();
        if digit.is_some_and(|d| d.is_ascii_digit() && d != b'0') && escaped_by.count() % 2 == 0 {
            return Some(BACKREFERENCE);
        }
    }
    None
}

/// Decision D108: the published schema is read by every editor and CI validator
/// a project points at it, so its own patterns are held to the portability
/// standard D12 imposes on a spec author's `pattern:`. The failure this guards
/// is worse than a divergence — an RE2-backed engine cannot *compile* a schema
/// containing `(?<!...)`, so it rejects every file with a regex error at
/// `$defs`, including the correct ones.
#[test]
fn published_schema_patterns_stay_in_the_interoperable_regex_subset() {
    let mut patterns = Vec::new();
    collect_patterns(&read_schema(), "", &mut patterns);
    assert!(
        patterns.len() >= 20,
        "expected the schema to constrain many values with `pattern`, found {}",
        patterns.len()
    );

    let offenders: Vec<String> = patterns
        .iter()
        .filter_map(|(path, pattern)| {
            non_portable_regex_construct(pattern)
                .map(|reason| format!("  at {path}: {pattern}\n    {reason}"))
        })
        .collect();
    assert!(
        offenders.is_empty(),
        "{} schema pattern(s) leave the interoperable regex subset (Decision D108):\n{}",
        offenders.len(),
        offenders.join("\n")
    );
}

#[test]
fn published_schema_version_enum_matches_supported_spec_versions() {
    let schema = read_schema();
    let published: Vec<String> = schema["properties"]["version"]["enum"]
        .as_array()
        .expect("properties.version.enum is an array")
        .iter()
        .map(|v| v.as_str().expect("spec versions are strings").to_string())
        .collect();
    let supported: Vec<String> = compose_core::SUPPORTED_SPEC_VERSIONS
        .iter()
        .map(|v| (*v).to_string())
        .collect();
    assert_eq!(
        published, supported,
        "schemas/agent-compose.schema.json must accept exactly the spec versions \
         compose-core supports (SUPPORTED_SPEC_VERSIONS)"
    );
}

#[test]
fn every_example_file_matches_the_published_schema() {
    let validator = compile_schema();
    let files = yaml_files(&examples_dir());
    assert!(
        files.len() >= 20,
        "expected the example corpus to be substantial, found {} files",
        files.len()
    );

    let mut failures = Vec::new();
    for file in &files {
        let instance = read_yaml_as_json(file);
        let errors = validation_errors(&validator, &instance);
        if !errors.is_empty() {
            failures.push(format!("{}\n{}", display(file), errors.join("\n")));
        }
    }
    assert!(
        failures.is_empty(),
        "{} example file(s) failed schema validation:\n\n{}",
        failures.len(),
        failures.join("\n\n")
    );
}

#[test]
fn examples_cover_both_document_kinds() {
    let files = yaml_files(&examples_dir());
    let mut entrypoints = 0;
    let mut deploy_files = 0;
    for file in &files {
        let instance = read_yaml_as_json(file);
        let object = instance.as_object().expect("spec files are mappings");
        if object.contains_key("imports") {
            entrypoints += 1;
        }
        if object.contains_key("placements")
            || object.contains_key("storage_backends")
            || object.contains_key("event_sources")
        {
            deploy_files += 1;
        }
    }
    assert!(
        entrypoints >= 2,
        "expected an entrypoint (a file with `imports:`) per example project, found {entrypoints}"
    );
    assert!(
        deploy_files >= 1,
        "expected at least one deploy/<target>.yml in the example corpus"
    );
}

#[test]
fn examples_exercise_every_node_kind() {
    let mut seen = BTreeSet::new();
    for file in yaml_files(&examples_dir()) {
        let instance = read_yaml_as_json(&file);
        for (key, definition) in instance.as_object().expect("spec files are mappings") {
            if !key.starts_with("flow.") {
                continue;
            }
            let Some(nodes) = definition.get("nodes").and_then(Value::as_object) else {
                continue;
            };
            for node in nodes.values() {
                let node = node.as_object().expect("nodes are mappings");
                for kind in NODE_KINDS {
                    if node.contains_key(*kind) {
                        seen.insert((*kind).to_string());
                    }
                }
            }
        }
    }
    let missing: Vec<&str> = NODE_KINDS
        .iter()
        .copied()
        .filter(|kind| !seen.contains(*kind))
        .collect();
    assert!(
        missing.is_empty(),
        "the example corpus must exercise every node kind; missing: {missing:?}"
    );
}

#[test]
fn every_invalid_fixture_is_rejected_by_the_published_schema() {
    let validator = compile_schema();
    let files = yaml_files(&invalid_fixtures_dir());

    let mut accepted = Vec::new();
    for file in &files {
        let instance = read_yaml_as_json(file);
        if validator.is_valid(&instance) {
            let rule = fixture_header(file)
                .map(|header| header.rule)
                .unwrap_or_else(|| "<undeclared rule>".to_string());
            accepted.push(format!("{} ({rule})", display(file)));
        }
    }
    assert!(
        accepted.is_empty(),
        "{} invalid fixture(s) were accepted by the schema:\n  {}",
        accepted.len(),
        accepted.join("\n  ")
    );
}

/// Rejection alone is too weak: a fixture that starts failing somewhere else
/// keeps the suite green while quietly ceasing to cover its rule. Each fixture
/// therefore declares both halves of its violation — the instance location and
/// the schema keyword that must reject it — and one single error has to match
/// both. Matching only the location would let any unrelated failure at the same
/// place stand in for the rule (a `oneOf` branch, say, that collapses every
/// sub-error onto its own pointer).
#[test]
fn every_invalid_fixture_fails_at_its_declared_location() {
    let validator = compile_schema();
    let files = yaml_files(&invalid_fixtures_dir());

    let mut mislocated = Vec::new();
    for file in &files {
        let header = fixture_header(file).unwrap_or_else(|| {
            panic!(
                "{} is missing its `# rule:`/`# at:`/`# via:` header",
                display(file)
            )
        });
        let instance = read_yaml_as_json(file);
        let reported: Vec<(String, String)> = validator
            .iter_errors(&instance)
            .map(|error| {
                (
                    error.instance_path().to_string(),
                    error.schema_path().to_string(),
                )
            })
            .collect();
        let expected = (header.at.clone(), header.via.clone());
        if !reported.contains(&expected) {
            let shown = if header.at.is_empty() {
                ROOT_POINTER
            } else {
                &header.at
            };
            mislocated.push(format!(
                "{} ({})\n  expected an error at {shown} from {}, got: {}",
                display(file),
                header.rule,
                header.via,
                if reported.is_empty() {
                    "<no errors at all>".to_string()
                } else {
                    reported
                        .iter()
                        .map(|(at, via)| {
                            format!(
                                "{} from {via}",
                                if at.is_empty() { ROOT_POINTER } else { at }
                            )
                        })
                        .collect::<Vec<_>>()
                        .join(", ")
                }
            ));
        }
    }
    assert!(
        mislocated.is_empty(),
        "{} invalid fixture(s) no longer fail for the rule they name:\n\n{}",
        mislocated.len(),
        mislocated.join("\n\n")
    );
}

#[test]
fn invalid_fixtures_declare_the_rule_they_violate() {
    let files = yaml_files(&invalid_fixtures_dir());
    let mut undeclared = Vec::new();
    let mut rules = BTreeSet::new();
    for file in &files {
        match fixture_header(file) {
            Some(header) => {
                rules.insert(header.rule);
            }
            None => undeclared.push(display(file)),
        }
    }
    assert!(
        undeclared.is_empty(),
        "every negative fixture must open with `# rule: <violated rule>`, \
         `# at: <instance pointer>` and `# via: <schema pointer>` comments; \
         missing or incomplete in:\n  {}",
        undeclared.join("\n  ")
    );
    assert_eq!(
        rules.len(),
        files.len(),
        "each negative fixture must name a distinct rule"
    );
}

#[test]
fn invalid_fixture_corpus_covers_at_least_twelve_rules() {
    let count = yaml_files(&invalid_fixtures_dir()).len();
    assert!(
        count >= 12,
        "the negative corpus must cover at least 12 rules, found {count}"
    );
}

/// Decision D120's conditional in the direction neither corpus reaches: the
/// shapes the schema must **accept**.
///
/// Every `anthropic` or `openai` provider under `examples/` declares an
/// `api_key:`, and both keyless fixtures in `invalid-schema/` omit the
/// `base_url:` as well — so the connection the rule exists to admit, keyless
/// because it names its gateway, is pinned by neither. Hoisting
/// `required: ["api_key"]` back out of the inner `then` would then make the
/// published schema refuse every legal gateway provider with the workspace
/// green, and since the schema is what editors read, the only place that would
/// show is a red squiggle on correct YAML in someone's editor.
///
/// Asserted per kind because the conditional is hand-duplicated per kind —
/// grammar 12.1 gives each row its own closed key list, so the two branches
/// cannot share one subschema and one instance would only ever prove one of
/// them.
#[test]
fn the_published_schema_accepts_a_keyless_provider_that_names_its_endpoint() {
    let validator = compile_schema();
    for kind in ["anthropic", "openai"] {
        let legal = [
            // The gateway supplies the vendor credential itself.
            json!({ "kind": kind, "base_url": "${LLM_GATEWAY}" }),
            // …and wants a token of its own, which rides `headers:`.
            json!({
                "kind": kind,
                "base_url": "${LLM_GATEWAY}",
                "headers": { "authorization": "Bearer ${PROXY_TOKEN}" },
            }),
            // The other repair the diagnostic offers, and the vendor-endpoint
            // shape the two negative fixtures are the counter-example to.
            json!({ "kind": kind, "api_key": "${VENDOR_API_KEY}" }),
        ];
        for provider in legal {
            let instance = json!({ "version": "0.1", "provider.gateway": provider });
            let errors = validation_errors(&validator, &instance);
            assert!(
                errors.is_empty(),
                "the published schema must accept this legal `{kind}` provider:\n{}\n{}",
                serde_json::to_string_pretty(&instance).expect("a printable instance"),
                errors.join("\n")
            );
        }
    }
}
