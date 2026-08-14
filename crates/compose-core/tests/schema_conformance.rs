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
//! The schema is deliberately looser than `agent-compose validate` (no
//! cross-file reference or static-analysis checks); see `docs/grammar.md`
//! Appendix B.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use jsonschema::Validator;
use serde_json::Value;

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
/// ```
///
/// The pointer is what keeps the corpus honest: without it a fixture that
/// starts failing for an unrelated reason still passes, and the rule it was
/// written for silently stops being covered.
struct FixtureHeader {
    rule: String,
    /// JSON Pointer into the instance; the empty string is the document root.
    at: String,
}

const ROOT_POINTER: &str = "<root>";

fn fixture_header(path: &Path) -> Option<FixtureHeader> {
    let text = fs::read_to_string(path).ok()?;
    let mut rule = None;
    let mut at = None;
    for line in text.lines().take_while(|line| line.starts_with('#')) {
        if let Some(value) = line.strip_prefix("# rule:") {
            rule.get_or_insert_with(|| value.trim().to_string());
        } else if let Some(value) = line.strip_prefix("# at:") {
            at.get_or_insert_with(|| match value.trim() {
                ROOT_POINTER => String::new(),
                pointer => pointer.to_string(),
            });
        }
    }
    match (rule, at) {
        (Some(rule), Some(at)) if !rule.is_empty() => Some(FixtureHeader { rule, at }),
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
/// therefore declares *where* the violation surfaces, and the schema must
/// produce an error at exactly that location.
#[test]
fn every_invalid_fixture_fails_at_its_declared_location() {
    let validator = compile_schema();
    let files = yaml_files(&invalid_fixtures_dir());

    let mut mislocated = Vec::new();
    for file in &files {
        let header = fixture_header(file)
            .unwrap_or_else(|| panic!("{} is missing its `# rule:`/`# at:` header", display(file)));
        let instance = read_yaml_as_json(file);
        let locations: Vec<String> = validator
            .iter_errors(&instance)
            .map(|error| error.instance_path().to_string())
            .collect();
        if !locations.contains(&header.at) {
            let expected = if header.at.is_empty() {
                ROOT_POINTER.to_string()
            } else {
                header.at.clone()
            };
            mislocated.push(format!(
                "{} ({})\n  expected an error at {expected}, got: {}",
                display(file),
                header.rule,
                if locations.is_empty() {
                    "<no errors at all>".to_string()
                } else {
                    locations.join(", ")
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
        "every negative fixture must open with `# rule: <violated rule>` and \
         `# at: <json pointer>` comments; missing or incomplete in:\n  {}",
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
