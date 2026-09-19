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

use compose_core::ast::definition::{Builtin, ProviderKind};
use compose_core::ast::{HmacAlgorithm, SignatureEncoding};
use jsonschema::Validator;
use serde_json::{Value, json};

/// Node kind keys the grammar defines (grammar 7.1 / PRD 5.5, 5.8, resolved
/// q57). The example corpus is required to exercise all of them.
const NODE_KINDS: &[&str] = &[
    "agent", "exec", "http", "function", "flow", "map", "human", "store", "coder",
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
/// them. The kinds are *derived* from
/// [`ProviderKind::default_endpoint`](compose_core::ast::definition::ProviderKind::default_endpoint)
/// rather than listed, for the same reason: a seventh kind with a vendor
/// endpoint of its own has to grow a conditional in the published schema too,
/// and a hardcoded pair would let that ship accepting a credential-less
/// provider with the workspace green.
#[test]
fn the_published_schema_accepts_a_keyless_provider_that_names_its_endpoint() {
    let validator = compile_schema();
    let defaulted: Vec<&str> = ProviderKind::ALL
        .iter()
        .filter(|kind| kind.default_endpoint().is_some())
        .map(|kind| kind.as_str())
        .collect();
    assert!(
        !defaulted.is_empty(),
        "the conditional credential rule is about the kinds with a default endpoint"
    );
    for kind in defaulted {
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

/// **The published schema takes every `permission_mode:` the grammar spells,
/// and no other** (grammar 8.9, Decision D146, PRD resolved q60 ruling a).
///
/// The one surface this key could have shipped broken with the whole suite
/// green. `coderNode` closes its properties, so a key the compiler learned and
/// this file did not is correct YAML an editor underlines in red — and a reader
/// following a diagnostic that says *write `permission_mode:` instead* would be
/// sent from a compile error to a squiggle. The direction is what makes it
/// silent: nothing in the Rust workspace reads this document, and the
/// `invalid-schema/` corpus can only pin refusals.
///
/// The members are **derived** from the grammar's own enumeration rather than
/// listed here, for the reason the provider-kind test above derives its kinds: a
/// seventh mode arriving with an SDK pin has to reach this file too, and a
/// hardcoded six would let that ship with an editor refusing the new spelling.
/// The refusing direction is asserted beside it, because an `enum` silently
/// widened to a bare `string` would pass every assertion above.
#[test]
fn the_published_schema_takes_every_permission_mode_the_grammar_spells() {
    use compose_core::ast::flow::PermissionMode;

    let validator = compile_schema();
    let node = |mode: Option<&str>| {
        let mut coder = serde_json::Map::new();
        coder.insert("harness".to_string(), json!("cc"));
        coder.insert("model".to_string(), json!("model.smart"));
        coder.insert("workspace".to_string(), json!("${REPO_ROOT}"));
        coder.insert("access".to_string(), json!("full_access"));
        if let Some(mode) = mode {
            coder.insert("permission_mode".to_string(), json!(mode));
        }
        coder.insert("prompt".to_string(), json!("Do the work."));
        coder.insert(
            "output".to_string(),
            json!({ "summary": { "type": "string" } }),
        );
        json!({
            "version": "0.1",
            "flow.main": {
                "outputs": { "summary": { "type": "string" } },
                "nodes": { "build": { "coder": coder, "input": "'go'" } },
                "edges": [
                    { "from": "start", "to": "build" },
                    { "from": "build", "to": "end" },
                ],
            },
        })
    };

    // `full_access` admits all six, so every member is a legal instance of this
    // one node — which is what keeps this a statement about the *schema* rather
    // than about the widening bound `validate` owns.
    for mode in PermissionMode::ALL {
        let instance = node(Some(mode.as_str()));
        let errors = validation_errors(&validator, &instance);
        assert!(
            errors.is_empty(),
            "the published schema refuses `permission_mode: {}`, which grammar 8.9 spells — an \
             editor would underline correct YAML, and the `reserved-harness-setting` diagnostic \
             sends authors to exactly this key:\n{}",
            mode.as_str(),
            errors.join("\n")
        );
    }
    // …and the key is optional, which is what every composition written before
    // it existed relies on.
    assert!(validation_errors(&validator, &node(None)).is_empty());
    // …and closed, or the enum would be a `string` and this test would be
    // asserting nothing.
    assert!(
        !validation_errors(&validator, &node(Some("acceptedits"))).is_empty(),
        "the published schema takes a `permission_mode:` outside the closed set, so an author's \
         editor accepts a spelling the parser answers with `unknown-variant`"
    );
}

/// Grammar 13.3's authentication surface in the direction the negative corpus
/// cannot reach: the shapes the schema must **accept**.
///
/// The `trigger-auth-*`, `trigger-callback-*` and `trigger-hmac-*` fixtures
/// under `invalid-schema/` pin the refusals — the env-ref rule on every secret,
/// the header and prefix shapes, the closed sets of `algorithm:`/`encoding:`,
/// the allowlist entry pattern, and four conditionals: `inboundAuth`'s `oneOf`,
/// `callbackAuth`'s `anyOf`, and the two `if`/`then` pairs that bind the
/// callback keys. The conditionals are where the accepting direction breaks
/// silently. Tighten `callbackAuth`'s `anyOf` into `inboundAuth`'s `oneOf` by a
/// copy-paste slip and the schema
/// refuses a trigger that signs *and* tokens its deliveries, which resolved q33
/// spells "and/or"; hoist `callback_allow` out of its `if` and the schema
/// refuses every unauthenticated callback, which is the documented test posture.
/// Both leave this workspace green and put a red squiggle on correct YAML in
/// somebody's editor.
///
/// Each instance is also run through the parser, because Appendix B's
/// relationship binds in this direction too: the schema is the editor-facing
/// approximation of `validate`, and a shape the compiler accepts and the schema
/// refuses is the pair disagreeing about the language.
#[test]
fn the_published_schema_accepts_the_whole_trigger_auth_surface() {
    let validator = compile_schema();
    let legal = [
        // Inbound: each scheme alone, one bare and one with every optional key.
        json!({ "type": "http", "flow": "flow.f", "auth": { "bearer": { "token": "${WEBHOOK_TOKEN}" } } }),
        json!({
            "type": "http",
            "flow": "flow.f",
            "auth": { "bearer": {
                "token": "${WEBHOOK_TOKEN}",
                "header": "X-Delivery-Token",
                "prefix": "Token ",
            } },
        }),
        json!({ "type": "http", "flow": "flow.f", "auth": { "hmac": { "secret": "${WEBHOOK_SECRET}" } } }),
        json!({
            "type": "http",
            "flow": "flow.f",
            "auth": { "hmac": {
                "secret": "${WEBHOOK_SECRET}",
                "header": "X-Hub-Signature-256",
                "algorithm": "sha512",
                "encoding": "base64",
                "prefix": "sha512=",
            } },
        }),
        // Inbound auth is orthogonal to the response mode.
        json!({
            "type": "http",
            "flow": "flow.f",
            "respond": "sync",
            "timeout": "30s",
            "auth": { "hmac": { "secret": "${WEBHOOK_SECRET}" } },
        }),
        // The `X-AgentCompose-` namespace is reserved *outbound* only: a trigger
        // that receives another deployment's deliveries verifies them by naming
        // the delivery's own signature header, exactly as it would name any
        // other vendor's (grammar 13.3, Decision D127).
        json!({
            "type": "http",
            "flow": "flow.f",
            "auth": { "hmac": {
                "secret": "${WEBHOOK_SECRET}",
                "header": "X-AgentCompose-Signature",
                "prefix": "sha256=",
            } },
        }),
        // Outbound: either scheme, and — the asymmetry with `auth:` — both.
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_auth": { "hmac": { "secret": "${CALLBACK_SECRET}" } },
            "callback_allow": ["https://hooks.example.com/*"],
        }),
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_auth": {
                "bearer": { "token": "${CALLBACK_TOKEN}" },
                "hmac": { "secret": "${CALLBACK_SECRET}" },
            },
            "callback_allow": ["https://hooks.example.com/*", "http://localhost:9000/*"],
        }),
        // The reservation on an outbound `header:` is one prefix and three whole
        // names (Decision D127), and a receiver's own header that merely looks
        // like one of them collides with nothing: a pattern written without its
        // `$` refuses these, and only this direction sees it.
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_auth": {
                "bearer": { "token": "${CALLBACK_TOKEN}", "header": "X-Content-Type" },
            },
            "callback_allow": ["https://hooks.example.com/*"],
        }),
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_auth": {
                "bearer": { "token": "${CALLBACK_TOKEN}", "header": "Content-Type-Signature" },
            },
            "callback_allow": ["https://hooks.example.com/*"],
        }),
        // The documented test posture: a callback that signs nothing, and so
        // needs no allowlist…
        json!({ "type": "http", "flow": "flow.f", "callback": "payload.body.callback_url" }),
        // …and an allowlist without outbound auth, which is legal in the
        // direction the mandatory rule does not run.
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_allow": ["https://hooks.example.com/*"],
        }),
        // The allowlist shapes the entry pattern must not take with it
        // (Decision D127): a port, a query string, an exact URL carrying no
        // wildcard at all, and a wildcard inside the host — legal grammar, whose
        // meaning §13.3 states rather than the pattern refusing the shape. The
        // refusals are fixtures; a pattern written one character too tight
        // refuses these instead, and only this direction sees it.
        json!({
            "type": "http",
            "flow": "flow.f",
            "callback": "payload.body.callback_url",
            "callback_allow": [
                "https://hooks.example.com:9000/*",
                "https://hooks.example.com?tenant=*",
                "https://hooks.example.com/webhooks/intake",
                "https://*.hooks.example.com/*",
                "https://hooks.example.com*",
            ],
        }),
    ];
    for trigger in legal {
        let instance = json!({ "triggers": { "intake": trigger } });
        let errors = validation_errors(&validator, &instance);
        assert!(
            errors.is_empty(),
            "the published schema must accept this legal trigger:\n{}\n{}",
            serde_json::to_string_pretty(&instance).expect("a printable instance"),
            errors.join("\n")
        );
        let source = serde_yaml_ng::to_string(&instance).expect("a printable document");
        let parsed = compose_core::parse_str(&source, "main.yml");
        assert!(
            parsed.diagnostics.is_empty(),
            "the parser must accept what the published schema accepts:\n{source}\n{}",
            parsed
                .diagnostics
                .iter()
                .map(|diagnostic| format!("  [{}] {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// Grammar 14.5's sink, in the direction the negative corpus cannot reach: the
/// shapes the schema must **accept**.
///
/// Five fixtures under `invalid-schema/` pin the refusals — the missing `url:`,
/// the wildcard in one, the closed `format:`, the closed key set, and the
/// at-least-one-scheme count the `auth:` block inherits. Every one of those is a
/// keyword whose *accepting* direction breaks silently: hoist `format` out of its
/// `enum` arm into a `const`, close `auth:` the way inbound `auth:` is closed,
/// or write the URL pattern one character tighter, and the schema refuses a sink
/// `agent-compose validate` accepts. That leaves this workspace green and puts a
/// red squiggle on correct YAML in somebody's editor, which is the asymmetry
/// Appendix B forbids outright.
///
/// The example corpus reaches exactly one of these shapes — `deploy/staging.yml`
/// carries a bearer-signed sink at the default format — so the rest are here.
/// Each instance is run through the parser too, for the reason
/// [`the_published_schema_accepts_the_whole_trigger_auth_surface`] does it: a
/// shape the compiler accepts and the schema refuses is the pair disagreeing
/// about the language.
///
/// The `format:` values are derived from
/// [`TraceSinkFormat::ALL`](compose_core::ast::TraceSinkFormat) rather than
/// listed, so a third rendering added to the compiler arrives with the editor
/// either knowing it or failing here.
#[test]
fn the_published_schema_accepts_the_whole_trace_sink_surface() {
    let validator = compile_schema();
    let mut legal = vec![
        // The minimal form: an address and nothing else. `format:` defaults and
        // a collector on a private network wants no credential.
        json!({ "url": "https://collector.internal.example/v1/traces" }),
        // `http` and a port, which is what an OTel Collector on the same host
        // is reached at — a pattern written one character too tight refuses it.
        json!({ "url": "http://localhost:4318/v1/traces" }),
        // Outbound signing, in each of the three shapes grammar 13.3 admits:
        // either scheme alone, and — the asymmetry with an inbound `auth:` —
        // both together.
        json!({
            "url": "https://collector.internal.example/v1/traces",
            "auth": { "bearer": { "token": "${TRACE_SINK_TOKEN}", "header": "X-Collector-Token", "prefix": "Token " } },
        }),
        json!({
            "url": "https://collector.internal.example/v1/traces",
            "auth": { "hmac": { "secret": "${TRACE_SINK_SECRET}" } },
        }),
        json!({
            "url": "https://collector.internal.example/v1/traces",
            "auth": {
                "bearer": { "token": "${TRACE_SINK_TOKEN}" },
                "hmac": { "secret": "${TRACE_SINK_SECRET}" },
            },
        }),
    ];
    assert!(
        !compose_core::ast::TraceSinkFormat::ALL.is_empty(),
        "the format table is what this test quantifies over"
    );
    for format in compose_core::ast::TraceSinkFormat::ALL {
        legal.push(json!({
            "url": "https://collector.internal.example/v1/traces",
            "format": format.as_str(),
        }));
    }

    for sink in legal {
        let instance = json!({ "version": "0.1", "trace_sink": sink });
        let errors = validation_errors(&validator, &instance);
        assert!(
            errors.is_empty(),
            "the published schema must accept this legal trace sink:\n{}\n{}",
            serde_json::to_string_pretty(&instance).expect("a printable instance"),
            errors.join("\n")
        );
        let source = serde_yaml_ng::to_string(&instance).expect("a printable document");
        let parsed = compose_core::parse_str(&source, "deploy/staging.yml");
        assert!(
            parsed.diagnostics.is_empty(),
            "the parser must accept what the published schema accepts:\n{source}\n{}",
            parsed
                .diagnostics
                .iter()
                .map(|diagnostic| format!("  [{}] {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// Grammar 14.6's registry, in the direction the negative corpus cannot reach:
/// the shapes the schema must **accept**.
///
/// Eight fixtures under `invalid-schema/` pin the refusals — the missing `url:`,
/// the closed key set on the block and on a scope entry, the `@` a scopes key is
/// spelled with, the `${ENV}` a `token:` is, the credential a `url:` authority
/// may not carry, and the two spellings the readers of the emitted files
/// disagree about: a query, which ends the path a WHATWG `URL` parses, and the
/// `;` npm's ini parser ends an `.npmrc` line on. Each of those is a keyword
/// whose *accepting*
/// direction breaks silently: delete `additionalProperties: false` from
/// `packageRegistry`, loosen `propertyNames` to drop the `@`, swap `token`'s
/// `$ref` off `envRef`, or write the URL pattern one character tighter, and the
/// published schema disagrees with `agent-compose validate` while this workspace
/// stays green. Since the schema is what editors read, the only place that shows
/// is a red squiggle on correct YAML in somebody's editor, or a blessing on YAML
/// the compiler refuses — the asymmetry `docs/grammar.md` Appendix B forbids
/// outright.
///
/// The example corpus reaches exactly one of these shapes —
/// `examples/triage-fanout/deploy/staging.yml` carries a mirror with a token and
/// one tokened scope — so the rest are here. Every instance is run through the
/// parser too, for the reason
/// [`the_published_schema_accepts_the_whole_trace_sink_surface`] does it: a
/// shape the compiler accepts and the schema refuses is the pair disagreeing
/// about the language.
#[test]
fn the_published_schema_accepts_the_whole_package_registry_surface() {
    let validator = compile_schema();
    let legal = vec![
        // The minimal form: an address and nothing else. A read-through mirror
        // on a private network wants no credential.
        json!({ "url": "https://npm.internal.example/repository/npm-group/" }),
        // `http` and a port, which is what a Verdaccio on the same host is
        // reached at — a pattern written one character too tight refuses it.
        json!({ "url": "http://localhost:4873/" }),
        // The host root, with and without its trailing slash.
        json!({ "url": "https://npm.internal.example" }),
        json!({ "url": "https://npm.internal.example/" }),
        // An `@` in the *path* is a scope's own directory on the mirror, and it
        // is the shape the authority rule must not reach.
        json!({ "url": "https://npm.internal.example/repository/@corp/" }),
        // The address rule (grammar 14.6 rules 1 and 5) refuses a query, a
        // fragment, every character a WHATWG `URL` respells, and the `;` and
        // `=` npm's ini parser reads as line syntax — so these are the shapes a
        // pattern written one character too tight takes down with them: a
        // mirror at an IPv4 address, a host with an underscore, and the path
        // punctuation that survives *both* readers exactly as written.
        json!({ "url": "https://10.0.0.5:8443/repository/npm-group/" }),
        json!({ "url": "https://npm_mirror.internal.example/repo/" }),
        json!({ "url": "https://npm.internal.example/a%2Fb/c'd[e]|f^g~h/" }),
        // A credential, which is the whole point of the key on a mirror that
        // wants one.
        json!({
            "url": "https://npm.internal.example/repository/npm-group/",
            "token": "${NPM_MIRROR_TOKEN}",
        }),
        // A scope with a credential of its own, at an address the default
        // registry's does not reach.
        json!({
            "url": "https://npm.internal.example/repository/npm-group/",
            "token": "${NPM_MIRROR_TOKEN}",
            "scopes": {
                "@corp": {
                    "url": "https://npm.internal.example/repository/corp/",
                    "token": "${NPM_CORP_TOKEN}",
                },
            },
        }),
        // …and a scope that reads through without one.
        json!({
            "url": "https://npm.internal.example/repository/npm-group/",
            "scopes": {
                "@corp": { "url": "https://npm.internal.example/repository/corp/" },
            },
        }),
        // Every shape a scope key takes: a bare letter, a digit, and the three
        // punctuation marks npm admits after the first character.
        json!({
            "url": "https://npm.internal.example/repository/npm-group/",
            "scopes": {
                "@a": { "url": "https://npm.internal.example/repository/a/" },
                "@corp": { "url": "https://npm.internal.example/repository/corp/" },
                "@my-org": { "url": "https://npm.internal.example/repository/my-org/" },
                "@acme.co": { "url": "https://npm.internal.example/repository/acme-co/" },
                "@org_2": { "url": "https://npm.internal.example/repository/org-2/" },
                "@2fa": { "url": "https://npm.internal.example/repository/2fa/" },
            },
        }),
    ];

    for registry in legal {
        let instance = json!({ "version": "0.1", "package_registry": registry });
        let errors = validation_errors(&validator, &instance);
        assert!(
            errors.is_empty(),
            "the published schema must accept this legal package registry:\n{}\n{}",
            serde_json::to_string_pretty(&instance).expect("a printable instance"),
            errors.join("\n")
        );
        let source = serde_yaml_ng::to_string(&instance).expect("a printable document");
        let parsed = compose_core::parse_str(&source, "deploy/staging.yml");
        assert!(
            parsed.diagnostics.is_empty(),
            "the parser must accept what the published schema accepts:\n{source}\n{}",
            parsed
                .diagnostics
                .iter()
                .map(|diagnostic| format!("  [{}] {}", diagnostic.code, diagnostic.message))
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
}

/// The two closed sets an inbound `hmac:` chooses between are one table each,
/// written twice: once as the parser's keywords and once as an `enum` in the
/// published schema (grammar 13.3, PRD resolved q32).
///
/// Neither corpus can see them drift. `md5` and `base64url` are pinned as
/// fixtures, so the *narrowing* direction is covered — but a third digest or a
/// third encoding added to `HmacAlgorithm::ALL` or `SignatureEncoding::ALL` and
/// forgotten here leaves `agent-compose validate` accepting a value the schema
/// squiggles in the author's editor, which is the direction Appendix B forbids
/// outright; and a value added to the `enum` alone leaves the editor blessing
/// YAML the compiler refuses. Both are invisible to a corpus of examples: the
/// keyword that would have caught them is the one nobody wrote yet.
///
/// So the sets are compared as sets, and then each keyword is put through both
/// authorities — the tables are what a signature is verified with, and a
/// keyword either half cannot read is one a deployment cannot use.
#[test]
fn the_published_schema_pins_the_signature_keywords_the_compilers_tables_do() {
    let schema = read_schema();
    let validator = compile_schema();
    let hmac = |key: &str, value: &str| {
        json!({ "triggers": { "intake": {
            "type": "http",
            "flow": "flow.f",
            "auth": { "hmac": { "secret": "${WEBHOOK_SECRET}", key: value } },
        } } })
    };

    let algorithms: BTreeSet<String> = HmacAlgorithm::ALL
        .iter()
        .map(|algorithm| algorithm.as_str().to_string())
        .collect();
    let encodings: BTreeSet<String> = SignatureEncoding::ALL
        .iter()
        .map(|encoding| encoding.as_str().to_string())
        .collect();

    for (key, table, outside) in [
        ("algorithm", &algorithms, "md5"),
        ("encoding", &encodings, "base64url"),
    ] {
        assert!(
            !table.is_empty(),
            "the `{key}` table is what this test quantifies over"
        );
        let published = variants(&schema["$defs"]["inboundHmac"]["properties"][key])
            .unwrap_or_else(|| panic!("`{key}` must stay a closed set in the published schema"));
        assert_eq!(
            &published, table,
            "the published schema and the compiler's table disagree about `{key}`"
        );

        for keyword in table {
            let instance = hmac(key, keyword);
            let errors = validation_errors(&validator, &instance);
            assert!(
                errors.is_empty(),
                "`{key}: {keyword}` is one the compiler reads, so the editor must not \
                 squiggle it:\n{}",
                errors.join("\n")
            );
            let source = serde_yaml_ng::to_string(&instance).expect("a printable document");
            let parsed = compose_core::parse_str(&source, "main.yml");
            assert!(
                parsed.diagnostics.is_empty(),
                "`{key}: {keyword}` is one the published schema accepts, so the parser must \
                 too:\n{source}\n{}",
                parsed
                    .diagnostics
                    .iter()
                    .map(|diagnostic| format!("  [{}] {}", diagnostic.code, diagnostic.message))
                    .collect::<Vec<_>>()
                    .join("\n")
            );
        }

        // And the closure holds in both authorities on the same spelling: a
        // digest neither can verify with, an encoding neither can read.
        assert!(
            !table.contains(outside),
            "`{outside}` is the spelling this half needs to be outside the set"
        );
        let instance = hmac(key, outside);
        assert!(
            !validation_errors(&validator, &instance).is_empty(),
            "`{key}: {outside}` names nothing the runtime has, so the published schema must \
             refuse it"
        );
        let source = serde_yaml_ng::to_string(&instance).expect("a printable document");
        assert!(
            !compose_core::parse_str(&source, "main.yml")
                .diagnostics
                .is_empty(),
            "`{key}: {outside}` names nothing the runtime has, so the parser must refuse it"
        );
    }
}

/// Every built-in the compiler admits is one the published schema admits, in
/// both spellings and with the bounds that name takes (grammar 5.5, 6.1,
/// Decision D135).
///
/// The negative half is six fixtures under `invalid-schema/`, and it cannot
/// carry the positive one: a closed `enum` over two names is a shape where the
/// *accepting* direction is the one that breaks silently. A third built-in added
/// to the compiler and forgotten here would leave an editor underlining a
/// composition `validate` accepts — the failure mode Appendix B exists to
/// prevent, and the one nothing in this workspace would otherwise see.
///
/// The names are derived from [`Builtin::ALL`] rather than listed, so the two
/// tables cannot fall out of step, and the bounds are keyed on the same
/// predicate the parser reads (`runs_a_command`).
#[test]
fn the_published_schema_accepts_every_builtin_with_its_own_bounds() {
    let validator = compile_schema();
    assert!(
        !Builtin::ALL.is_empty(),
        "the built-in set is what this test quantifies over"
    );
    for builtin in Builtin::ALL {
        let mut bound = serde_json::Map::new();
        bound.insert("builtin".to_string(), json!(builtin.keyword()));
        bound.insert("workspace".to_string(), json!("${WORKSPACE}"));
        if builtin.runs_a_command() {
            bound.insert("timeout".to_string(), json!("30s"));
            bound.insert("env".to_string(), json!({ "PATH": "/usr/bin" }));
            bound.insert("inherit_env".to_string(), json!(false));
        }
        let instance = json!({
            "version": "0.1",
            "tool.sandbox": Value::Object(bound),
            "agent.fixer": {
                "model": "model.smart",
                "prompt": "Fix it.",
                // Both spellings in one list: the configured tool, and the
                // shorthand that takes every default.
                "tools": ["tool.repo_grep", "tool.sandbox", builtin.address()],
                "output": { "summary": { "type": "string" } },
            },
        });
        let errors = validation_errors(&validator, &instance);
        assert!(
            errors.is_empty(),
            "the published schema must accept `{}` in both spellings:\n{}\n{}",
            builtin.address(),
            serde_json::to_string_pretty(&instance).expect("a printable instance"),
            errors.join("\n")
        );
    }
}

/// The published schema knows every server tool the compiler's table does, and
/// checks each one's shape the same way (grammar 12.1, Decision D122).
///
/// The two halves of resolved q30's first tier have to agree, and they are two
/// hand-written documents: `ast::server_tools` is what `agent-compose validate`
/// reads, and `schemas/agent-compose.schema.json` is what an editor reads. A
/// tool in one and not the other is a red squiggle on correct YAML, or a green
/// editor on a config the compiler refuses — and both look to an author like
/// the tool being unsupported.
///
/// So the schema is **derived from** the table rather than compared to a list:
/// the branch is found by the `type` it accepts, and then every field, every
/// `required`, every range, every closed choice and every mutually exclusive
/// pair is read out of the table and asserted against that branch. A tool added
/// with no mirrored `if`/`then` fails here on the row it was added for, and so
/// does a *field* added to a row that already has one — which is the drift the
/// weaker "the type string appears somewhere" reading could not see.
#[test]
fn the_published_schema_knows_every_server_tool_the_table_does() {
    let schema = read_schema();
    let defs = schema["$defs"]
        .as_object()
        .expect("the schema declares `$defs`");
    for (kind, def) in [
        (ProviderKind::Anthropic, "anthropicServerTool"),
        (ProviderKind::OpenAi, "openaiServerTool"),
    ] {
        let published = defs
            .get(def)
            .unwrap_or_else(|| panic!("the schema declares `{def}`"));
        let branches = published["allOf"]
            .as_array()
            .unwrap_or_else(|| panic!("`{def}` is an `allOf` of one `if`/`then` per tool"));
        let known = compose_core::ast::server_tools::known(kind);
        assert!(
            !known.is_empty(),
            "`{}` is one of the kinds with a curated table",
            kind.as_str()
        );
        for tool in known {
            let branch = branches
                .iter()
                .find(|branch| gates_on(branch, tool.type_name))
                .unwrap_or_else(|| {
                    panic!(
                        "`{}`'s `{}` is in the compiler's table and not in the published \
                         schema's `{def}`",
                        kind.as_str(),
                        tool.type_name
                    )
                });
            let subject = format!("`{def}`'s `{}`", tool.type_name);
            let then = &branch["then"];
            // …and **open**, as the compiler's tier is. `agent-compose validate`
            // carries a key the row does not name with a warning rather than
            // refusing it (`unknown-server-tool-field`), so a schema that closed
            // the object would squiggle exactly the config the compiler ships to
            // the wire — the editor claiming an error where the authority has
            // none.
            assert_eq!(
                then["additionalProperties"],
                Value::Null,
                "{subject} must stay open: a field this release predates is carried, not refused"
            );
            assert!(
                required_of(then).contains("type"),
                "{subject} requires its own `type`"
            );
            same_object(&subject, then, tool.shape);
        }
        // …and the other direction: a branch gating on a `type` the table does
        // not have would squiggle-check a tool `agent-compose validate` carries
        // unchecked, which is the second tier's own promise broken from the
        // editor's side.
        for branch in branches {
            for gated in gated_types(branch) {
                assert!(
                    compose_core::ast::server_tools::lookup(kind, &gated).is_some(),
                    "`{def}` checks `{gated}`, which is not in the compiler's `{}` table",
                    kind.as_str()
                );
            }
        }
    }
}

/// The `type` strings one `if`/`then` branch of a server-tool subschema gates
/// on. Empty for the `$ref` branch that carries the shared shape.
fn gated_types(branch: &Value) -> Vec<String> {
    branch["if"]["properties"]["type"]["enum"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// Whether this branch is the one that checks `type_name`.
fn gates_on(branch: &Value, type_name: &str) -> bool {
    gated_types(branch).iter().any(|name| name == type_name)
}

/// The `required` list of one subschema, as a set.
fn required_of(node: &Value) -> BTreeSet<String> {
    node["required"]
        .as_array()
        .map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default()
}

/// One tool's `then` branch against the table's [`ObjectShape`] for it.
///
/// `type` is the row's **identity** rather than one of its fields, so it is
/// excluded on the schema side here; every nested object goes through
/// [`same_nested`] instead, where `type` is an ordinary field.
fn same_object(subject: &str, node: &Value, shape: &compose_core::ast::server_tools::ObjectShape) {
    let published: BTreeSet<String> = node["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{subject} declares `properties`"))
        .keys()
        .filter(|key| *key != "type")
        .cloned()
        .collect();
    let tabled: BTreeSet<String> = shape
        .names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        published, tabled,
        "{subject} and the compiler's table disagree about which fields the tool has"
    );
    let published_required: BTreeSet<String> = required_of(node)
        .into_iter()
        .filter(|key| key != "type")
        .collect();
    let tabled_required: BTreeSet<String> = shape
        .required
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        published_required, tabled_required,
        "{subject} and the compiler's table disagree about which fields are required"
    );
    // The mutually exclusive pairs, which the schema spells as a `not`/`required`
    // beside the properties.
    let published_pairs: BTreeSet<Vec<String>> = node["allOf"]
        .as_array()
        .map(|clauses| {
            clauses
                .iter()
                .filter_map(|clause| {
                    let pair: Vec<String> = required_of(&clause["not"]).into_iter().collect();
                    (!pair.is_empty()).then_some(pair)
                })
                .collect()
        })
        .unwrap_or_default();
    let tabled_pairs: BTreeSet<Vec<String>> = shape
        .exclusive
        .iter()
        .map(|[left, right]| {
            let mut pair = vec![(*left).to_string(), (*right).to_string()];
            pair.sort();
            pair
        })
        .collect();
    assert_eq!(
        published_pairs, tabled_pairs,
        "{subject} and the compiler's table disagree about which fields exclude each other"
    );
    for field in shape.fields {
        same_field(
            &format!("{subject}'s `{}`", field.name),
            &node["properties"][field.name],
            field.shape,
        );
    }
}

/// A nested config object, where `type` **is** an ordinary field —
/// `user_location: { type: approximate }`.
fn same_nested(subject: &str, node: &Value, shape: &compose_core::ast::server_tools::ObjectShape) {
    assert_eq!(node["type"], json!("object"), "{subject} is a mapping");
    assert_eq!(
        node["additionalProperties"],
        Value::Null,
        "{subject} stays open, as the compiler's tier is one level down too"
    );
    let published: BTreeSet<String> = node["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("{subject} declares `properties`"))
        .keys()
        .cloned()
        .collect();
    let tabled: BTreeSet<String> = shape
        .names()
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        published, tabled,
        "{subject} and the compiler's table disagree about which keys it has"
    );
    let tabled_required: BTreeSet<String> = shape
        .required
        .iter()
        .map(|name| (*name).to_string())
        .collect();
    assert_eq!(
        required_of(node),
        tabled_required,
        "{subject} and the compiler's table disagree about which keys are required"
    );
    for field in shape.fields {
        same_field(
            &format!("{subject}.`{}`", field.name),
            &node["properties"][field.name],
            field.shape,
        );
    }
}

/// One field's subschema against the [`FieldShape`] the compiler checks it with.
fn same_field(subject: &str, node: &Value, shape: compose_core::ast::server_tools::FieldShape) {
    use compose_core::ast::server_tools::FieldShape;
    match shape {
        FieldShape::Integer(low, high) => {
            assert_eq!(node["type"], json!("integer"), "{subject} is an integer");
            same_bound(
                subject,
                node,
                "minimum",
                (low != i64::MIN).then_some(low as f64),
            );
            same_bound(
                subject,
                node,
                "maximum",
                (high != i64::MAX).then_some(high as f64),
            );
        }
        FieldShape::Number(low, high) => {
            assert_eq!(node["type"], json!("number"), "{subject} is a number");
            same_bound(subject, node, "minimum", Some(low));
            same_bound(subject, node, "maximum", Some(high));
        }
        FieldShape::Boolean => {
            assert_eq!(node["type"], json!("boolean"), "{subject} is a boolean");
        }
        // Interpolable, like every other non-secret provider value (grammar 4.3
        // class 2), which on the schema side is the shared `interpolatedString`.
        FieldShape::Text => {
            assert_eq!(
                node["$ref"],
                json!("#/$defs/interpolatedString"),
                "{subject} is an interpolable string"
            );
        }
        // A closed set, published in one of two shapes — and which of the two
        // is the whole of what the compiler does with an `${ENV}` written here.
        //
        // A **pin** (one member) is a bare `const`/`enum`: the value is decided
        // by the entry's own `type:`, so a reference there is refused rather
        // than carried (`unexpected-env-ref`) and the schema must not offer an
        // arm for one. A **knob** (more than one) is class 2 like every other
        // provider string, so the compiler leaves an interpolated value
        // undecided — and a schema that published only the `enum` would put a
        // red squiggle on YAML `validate` accepts, which is the asymmetry this
        // whole file exists to refuse (see the module header).
        FieldShape::Choice(choices) => {
            let tabled: BTreeSet<String> = choices.iter().map(|name| (*name).to_string()).collect();
            if shape.pinned().is_some() {
                let published = variants(node).unwrap_or_else(|| {
                    panic!("{subject} is a closed set of one string in the schema")
                });
                assert_eq!(
                    published, tabled,
                    "{subject} and the compiler's table disagree about the accepted values"
                );
                assert_eq!(
                    node["anyOf"],
                    Value::Null,
                    "{subject} is pinned to one value the compiler reads at compile time, so \
                     the schema must not offer an interpolation arm the compiler would refuse"
                );
                return;
            }
            let arms = node["anyOf"].as_array().unwrap_or_else(|| {
                panic!("{subject} is a closed set of strings **or** an `${{ENV}}` reference")
            });
            let published = arms
                .iter()
                .find_map(variants)
                .unwrap_or_else(|| panic!("{subject} publishes the closed set as one arm"));
            assert_eq!(
                published, tabled,
                "{subject} and the compiler's table disagree about the accepted values"
            );
            assert!(
                arms.iter()
                    .any(|arm| arm["$ref"] == json!("#/$defs/envRefString")),
                "{subject} is interpolable, like every other class 2 provider string: \
                 `validate` does not decide a value carrying an `${{ENV}}` reference, and the \
                 schema must not either"
            );
        }
        FieldShape::Strings => {
            assert_eq!(node["type"], json!("array"), "{subject} is an array");
            assert_eq!(
                node["items"]["$ref"],
                json!("#/$defs/interpolatedString"),
                "{subject} holds interpolable strings"
            );
        }
        // A field the table knows the tool has and nothing more (a recursive or
        // union shape). The schema says the same thing: a property with a
        // description and no constraint at all, so an editor offers the key,
        // completes nothing inside it, and refuses nothing either.
        FieldShape::Opaque => {
            assert!(
                node["description"].is_string(),
                "{subject} says what it is, since it constrains nothing"
            );
            for keyword in [
                "type",
                "enum",
                "const",
                "$ref",
                "anyOf",
                "allOf",
                "oneOf",
                "properties",
                "items",
                "required",
            ] {
                assert_eq!(
                    node[keyword],
                    Value::Null,
                    "{subject} is carried verbatim by the compiler and must not be constrained \
                     here by `{keyword}`"
                );
            }
        }
        FieldShape::Object(nested) => same_nested(subject, node, nested),
        FieldShape::TextOrObject(nested) => {
            let arms = node["anyOf"]
                .as_array()
                .unwrap_or_else(|| panic!("{subject} is a string **or** a mapping"));
            assert!(
                arms.iter()
                    .any(|arm| arm["$ref"] == json!("#/$defs/interpolatedString")),
                "{subject} takes an interpolable string"
            );
            let object = arms
                .iter()
                .find(|arm| arm["type"] == json!("object"))
                .unwrap_or_else(|| panic!("{subject} takes a mapping too"));
            same_nested(subject, object, nested);
        }
    }
}

/// One numeric bound, present exactly when the table states one.
fn same_bound(subject: &str, node: &Value, keyword: &str, expected: Option<f64>) {
    let published = node[keyword].as_f64();
    assert_eq!(
        published, expected,
        "{subject} and the compiler's table disagree about `{keyword}`"
    );
}

/// The strings a closed subschema accepts, however it spells the closure.
fn variants(node: &Value) -> Option<BTreeSet<String>> {
    if let Some(only) = node["const"].as_str() {
        return Some(BTreeSet::from([only.to_string()]));
    }
    node["enum"].as_array().map(|names| {
        names
            .iter()
            .filter_map(|name| name.as_str().map(str::to_string))
            .collect()
    })
}

/// Every kind the compiler admits `server_tools:` on accepts one in the
/// published schema, and every kind it refuses the key on is refused there too
/// (grammar 12.1, Decision D122).
///
/// Derived from `ProviderKind::serves_server_tools` for the reason
/// [`the_published_schema_accepts_a_keyless_provider_that_names_its_endpoint`]
/// derives its kinds: the gate is hand-duplicated per kind in the schema, so a
/// seventh kind — or a fourth kind taught the shape — has to grow a branch
/// there, and a hardcoded list would let that ship silently disagreeing.
#[test]
fn the_published_schema_gates_server_tools_by_kind() {
    let validator = compile_schema();
    for kind in ProviderKind::ALL {
        let mut provider = serde_json::Map::new();
        provider.insert("kind".to_string(), json!(kind.as_str()));
        // Whatever else this kind requires, so the only thing under test is the
        // one key.
        for key in kind.required_keys() {
            provider.insert((*key).to_string(), json!("${SOMETHING}"));
        }
        if kind.default_endpoint().is_some() {
            provider.insert("api_key".to_string(), json!("${VENDOR_API_KEY}"));
        }
        provider.insert(
            "server_tools".to_string(),
            // A type outside every table: the second tier, which is exactly what
            // an editor must not squiggle on a kind that takes the key.
            json!([{ "type": "a_tool_shipped_after_this_release" }]),
        );
        let instance = json!({ "version": "0.1", "provider.p": Value::Object(provider) });
        let errors = validation_errors(&validator, &instance);
        assert_eq!(
            errors.is_empty(),
            kind.serves_server_tools(),
            "the published schema and `{}`'s key row disagree about `server_tools:`:\n{}",
            kind.as_str(),
            errors.join("\n")
        );
    }
}

/// The strict tier really is strict in the schema too: a known tool's field
/// given a value the vendor refuses is rejected, and what neither half of the
/// compiler can verify is left alone.
///
/// This is the property the two tiers are *made of*, and it is the one an editor
/// shows: the config the provider will refuse is squiggled where it was written,
/// and the config nobody can verify — an unknown `type:`, or a key outside a
/// known one's row — is not squiggled at all, because `agent-compose validate`
/// carries both to the wire.
#[test]
fn the_published_schema_checks_a_known_server_tool_and_carries_an_unknown_one() {
    let validator = compile_schema();
    let strict = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{ "type": "web_search_20250305", "name": "web_search", "max_uses": 0 }],
        },
    });
    assert!(
        !validation_errors(&validator, &strict).is_empty(),
        "a value outside the bound the table states is refused"
    );

    let wrong_name = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{ "type": "web_search_20250305", "name": "search_the_web" }],
        },
    });
    assert!(
        !validation_errors(&validator, &wrong_name).is_empty(),
        "the canonical `name` the Messages wire pairs with the `type` is fixed"
    );

    // The field tier, from the editor's side: the compiler warns and carries, so
    // the schema must not refuse — squiggling a key `agent-compose validate`
    // ships to the wire is the editor claiming an error the authority does not
    // have (`unknown-server-tool-field`).
    let newer_than_this_release = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{
                "type": "web_search_20250305",
                "name": "web_search",
                "max_uses": 5,
                "result_freshness": "week",
                "user_location": { "type": "approximate", "postcode": "SW1A" },
            }],
        },
    });
    let errors = validation_errors(&validator, &newer_than_this_release);
    assert!(
        errors.is_empty(),
        "a parameter the vendor added after this release is carried, nested or not:\n{}",
        errors.join("\n")
    );

    let both_lists = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{
                "type": "web_search_20250305",
                "name": "web_search",
                "allowed_domains": ["docs.example.com"],
                "blocked_domains": ["ads.example.com"],
            }],
        },
    });
    assert!(
        !validation_errors(&validator, &both_lists).is_empty(),
        "an allow-list beside a deny-list is refused"
    );

    let unverifiable = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{ "type": "web_search_20260101", "name": "web_search", "max_usages": 5 }],
        },
    });
    let errors = validation_errors(&validator, &unverifiable);
    assert!(
        errors.is_empty(),
        "a tool outside the table travels to the wire as written, squiggle-free:\n{}",
        errors.join("\n")
    );

    let missing_store = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "openai",
            "api_key": "${OPENAI_API_KEY}",
            "server_tools": [{ "type": "file_search" }],
        },
    });
    assert!(
        !validation_errors(&validator, &missing_store).is_empty(),
        "a `file_search` with no `vector_store_ids` is refused"
    );

    let legal = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "openai",
            "api_key": "${OPENAI_API_KEY}",
            "server_tools": [
                {
                    "type": "file_search",
                    "vector_store_ids": ["${HANDBOOK_STORE}"],
                    "max_num_results": 5,
                    // A documented parameter whose interior is a union the
                    // compiler's vocabulary does not state, so neither half of
                    // it constrains the value — but both know the field exists.
                    "filters": { "type": "eq", "key": "kind", "value": "handbook" },
                },
                { "type": "code_interpreter", "container": { "type": "auto" } },
                { "type": "image_generation", "input_fidelity": "high" },
            ],
        },
    });
    let errors = validation_errors(&validator, &legal);
    assert!(
        errors.is_empty(),
        "a well-formed Responses suite is accepted, `${{ENV}}` values and all:\n{}",
        errors.join("\n")
    );

    let bad_fidelity = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "openai",
            "api_key": "${OPENAI_API_KEY}",
            "server_tools": [{ "type": "image_generation", "input_fidelity": "highest" }],
        },
    });
    assert!(
        !validation_errors(&validator, &bad_fidelity).is_empty(),
        "a tabled field's closed set is still closed"
    );
}

/// A closed set and an `${ENV}` reference: the one place the two authorities
/// could disagree about the same key and each look right on its own.
///
/// `server_tools:` is provider config, so grammar 4.3 class 2 promises its
/// **string** values interpolate, and a closed set of strings is a string.
/// `agent-compose validate` therefore leaves `search_context_size:
/// ${SEARCH_DEPTH}` undecided — a staging deployment searching shallowly is a
/// legal thing to want — and a schema publishing only the `enum` would put a red
/// squiggle on YAML the compiler accepts and CI passes. That asymmetry is what
/// this file exists to refuse (see the module header).
///
/// The **pin** is the other half of the same sentence and inverts every clause:
/// a set of one is not a knob, its value is decided by the entry's own `type:`,
/// and the compiler refuses a reference there (`unexpected-env-ref`,
/// `check::providers`'s `plugin`). So the schema must refuse it too, or the
/// squiggle goes missing on the one field whose whole purpose is to catch a
/// copied-and-edited config before the wire answers 400.
///
/// `same_field` holds the *shapes* to the table; this holds what the shapes buy
/// an author, which is the thing a reader of a bug report has in hand.
#[test]
fn the_published_schema_interpolates_a_closed_set_the_table_does_not_pin() {
    let validator = compile_schema();
    let with = |tool: Value| {
        json!({
            "version": "0.1",
            "provider.p": {
                "kind": "openai",
                "api_key": "${OPENAI_API_KEY}",
                "server_tools": [tool],
            },
        })
    };

    for (key, reference) in [
        ("search_context_size", "${SEARCH_DEPTH}"),
        ("quality", "${IMAGE_QUALITY}"),
        // Embedded rather than whole, which is the shape class 2 states.
        ("output_format", "${IMAGE_FORMAT}"),
    ] {
        let tool = if key == "search_context_size" {
            json!({ "type": "web_search", key: reference })
        } else {
            json!({ "type": "image_generation", key: reference })
        };
        let errors = validation_errors(&validator, &with(tool));
        assert!(
            errors.is_empty(),
            "`{key}: {reference}` is a class 2 provider value `agent-compose validate` \
             accepts, so the editor must not squiggle it:\n{}",
            errors.join("\n")
        );
    }

    // …and the set is still closed for everything that is not a reference: a
    // typo does not buy itself an exemption by looking like one.
    for spelling in ["medum", "$${SEARCH_DEPTH}", "${lowercase}", "${UNCLOSED"] {
        assert!(
            !validation_errors(
                &validator,
                &with(json!({ "type": "web_search", "search_context_size": spelling })),
            )
            .is_empty(),
            "`search_context_size: {spelling}` reaches the wire as written and the service \
             refuses it, so both authorities must"
        );
    }

    // The pin, on the wire that has one. Both spellings the compiler refuses.
    let anthropic = |name: &str| {
        json!({
            "version": "0.1",
            "provider.p": {
                "kind": "anthropic",
                "api_key": "${ANTHROPIC_API_KEY}",
                "server_tools": [{ "type": "web_search_20250305", "name": name }],
            },
        })
    };
    assert!(
        !validation_errors(&validator, &anthropic("${WS_NAME}")).is_empty(),
        "a pinned `name:` is decided by the `type:` rather than by the environment"
    );
    let nested = json!({
        "version": "0.1",
        "provider.p": {
            "kind": "anthropic",
            "api_key": "${ANTHROPIC_API_KEY}",
            "server_tools": [{
                "type": "web_search_20250305",
                "name": "web_search",
                "user_location": { "type": "${LOCATION_KIND}" },
            }],
        },
    });
    assert!(
        !validation_errors(&validator, &nested).is_empty(),
        "…and so is a nested object's pinned `type:`"
    );
}
