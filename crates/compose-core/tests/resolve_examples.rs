//! The positive corpus for resolution: both example projects resolve clean,
//! and their whole IR is snapshotted.
//!
//! The IR is the deploy artifact (PRD 5.1), so what it contains is a contract
//! and not an implementation detail: a change to any of it — a key, an
//! ordering, a span, an unresolved env ref — has to show up as a reviewable
//! diff rather than as a silent change to what every downstream pass consumes.
//! That is what these snapshots are for, and they are deliberately the *whole*
//! document rather than a summary of it.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Resolution, resolve, resolve_with_target};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("manifest dir has a grandparent")
        .to_path_buf()
}

fn render(resolution: &Resolution) -> String {
    resolution
        .diagnostics
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

/// Resolve an example, asserting it is clean, and return its artifact as JSON.
fn resolve_clean(project: &str, target: &str) -> String {
    let entrypoint = repo_root().join(project).join("main.yml");
    let resolution = resolve_with_target(&entrypoint, target);
    assert!(
        resolution.diagnostics.is_empty(),
        "{project} --target {target} produced diagnostics:\n{}",
        render(&resolution)
    );
    resolution
        .ir
        .expect("a composition with no diagnostics produces an artifact")
        .to_json()
        .expect("the artifact serializes")
}

#[test]
fn snapshot_review_loop_ir() {
    insta::assert_snapshot!(resolve_clean("examples/review-loop", "local"));
}

#[test]
fn snapshot_triage_fanout_ir() {
    insta::assert_snapshot!(resolve_clean("examples/triage-fanout", "local"));
}

/// The same composition under a named target. Only the deploy layer differs —
/// that is PRD 5.8's per-target invariant, and the two snapshots are what make
/// a regression in it visible.
#[test]
fn snapshot_triage_fanout_ir_under_staging() {
    insta::assert_snapshot!(resolve_clean("examples/triage-fanout", "staging"));
}

/// Every example resolves, and every file it reads is recorded in the artifact.
#[test]
fn examples_resolve_with_every_file_they_read() {
    for (project, target, expected) in [
        ("examples/review-loop", "local", 8),
        ("examples/triage-fanout", "local", 15),
        ("examples/triage-fanout", "staging", 15),
    ] {
        let entrypoint = repo_root().join(project).join("main.yml");
        let resolution = resolve_with_target(&entrypoint, target);
        let ir = resolution
            .ir
            .as_ref()
            .unwrap_or_else(|| panic!("{project} resolves:\n{}", render(&resolution)));
        assert_eq!(
            ir.sources.len(),
            expected,
            "{project} --target {target} read {} file(s)",
            ir.sources.len()
        );
        assert_eq!(ir.target, target);
        assert_eq!(ir.entrypoint, "main.yml");
        assert_eq!(ir.spec_version, "0.1");
    }
}

/// `local` is the target when none is named (grammar 14, Decision D59).
#[test]
fn no_target_named_resolves_local() {
    let entrypoint = repo_root().join("examples/triage-fanout/main.yml");
    let ir = resolve(&entrypoint).ir.expect("the example resolves");
    assert_eq!(ir.target, "local");
    assert_eq!(ir.deploy.source.as_deref(), Some("deploy/local.yml"));
    // `local` substitutes local storage unconditionally and may not carry a
    // backend section at all, so the artifact carries none either.
    assert!(ir.deploy.storage_backends.is_none());
    // `hub:` and `placements:` are live static grammar `local` admits like any
    // other target: the artifact carries the sections it was declared with,
    // spans and all (D87, grammar 14.1, 14.2).
    let placements = ir
        .deploy
        .placements
        .as_ref()
        .expect("`deploy/local.yml` declares `placements:`");
    assert!(!placements.is_empty());
    assert_eq!(placements.span.source.as_str(), "deploy/local.yml");
    let hub = ir
        .deploy
        .hub
        .as_ref()
        .expect("`deploy/local.yml` declares `hub:`");
    assert_eq!(
        hub.join_token
            .as_ref()
            .expect("the hub declares a join token")
            .value
            .name,
        "MESH_JOIN_TOKEN"
    );
}

/// Under a named target the same composition carries that target's layer, and
/// nothing else about the artifact moves.
#[test]
fn only_the_deploy_layer_forks_per_target() {
    let entrypoint = repo_root().join("examples/triage-fanout/main.yml");
    let local = resolve_with_target(&entrypoint, "local")
        .ir
        .expect("the example resolves under local");
    let staging = resolve_with_target(&entrypoint, "staging")
        .ir
        .expect("the example resolves under staging");

    assert_eq!(local.definitions, staging.definitions);
    assert_eq!(local.state, staging.state);
    assert_eq!(local.triggers, staging.triggers);
    assert_eq!(local.defaults, staging.defaults);
    assert_ne!(local.deploy, staging.deploy);
    assert!(staging.deploy.storage_backends.is_some());
}

/// Every definition is inlined under the address it is reached by, whichever
/// file declares it (grammar 2.2, PRD 5.1).
#[test]
fn definitions_are_keyed_by_typed_address() {
    let entrypoint = repo_root().join("examples/review-loop/main.yml");
    let ir = resolve(&entrypoint).ir.expect("the example resolves");
    let addresses: Vec<&str> = ir.definitions.keys().map(String::as_str).collect();
    assert_eq!(
        addresses,
        [
            "agent.researcher",
            "agent.reviewer",
            "flow.review_loop",
            "model.default",
            "model.fast",
            "model.smart",
            "provider.anthropic",
            "tool.web_search",
        ],
        "definitions are emitted sorted by address (Decision D55)"
    );
    assert!(ir.definition("agent.reviewer").is_some());
    assert!(ir.definition("agent.nonexistent").is_none());
}

/// Spans in the artifact name files by their **project-relative** path, so the
/// same spelling appears in the IR, on a command line, and in a diagnostic on
/// every host (grammar 1.4) — and so two machines produce the same bytes
/// (PRD 5.12).
#[test]
fn spans_name_files_relative_to_the_project_root() {
    let entrypoint = repo_root().join("examples/review-loop/main.yml");
    let json = resolve(&entrypoint)
        .ir
        .expect("the example resolves")
        .to_json()
        .expect("the artifact serializes");
    assert!(
        json.contains("\"span\": \"agents/reviewer.yml:"),
        "an imported file's spans are named relative to the project root"
    );
    assert!(
        !json.contains(&repo_root().display().to_string()),
        "no absolute path reaches the artifact"
    );
}

/// A resolution that rejects nothing produces an artifact, and one that rejects
/// something produces none: half an artifact would send the next pass chasing
/// failures that belong to this one.
///
/// An entrypoint that could not be read is also the one diagnostic that names a
/// file by the path the *command line* typed rather than by its project-relative
/// name. Nothing else may: every span that reaches the artifact is relative to
/// the project root so that two machines produce the same bytes (grammar 1.4,
/// PRD 5.12) — but there is no artifact here, and a bare `does-not-exist.yml`
/// would never say which directory the compiler looked in.
#[test]
fn an_unresolvable_composition_produces_no_artifact() {
    let missing = repo_root().join("examples/review-loop/does-not-exist.yml");
    let resolution = resolve(&missing);
    assert!(resolution.ir.is_none());
    assert!(resolution.has_errors());
    assert_eq!(resolution.diagnostics.len(), 1);
    let reported = &resolution.diagnostics[0];
    assert_eq!(reported.code.as_str(), "io-error");
    let typed = missing.display().to_string();
    assert_eq!(
        reported.span.source.as_str(),
        typed,
        "the diagnostic is anchored at the path the command line named"
    );
    assert!(
        reported
            .message
            .starts_with(&format!("cannot read `{typed}`: ")),
        "an unreadable entrypoint is named by the path the command line typed, \
         directory and all — got: {}",
        reported.message
    );
}

/// The artifact is a build product: reading `examples/` twice must produce the
/// same bytes, or "same DSL in, byte-identical output" is not a property this
/// compiler has (PRD 5.12).
#[test]
fn resolving_twice_produces_identical_bytes() {
    for project in ["examples/review-loop", "examples/triage-fanout"] {
        let entrypoint = repo_root().join(project).join("main.yml");
        let first = resolve(&entrypoint).ir.expect("resolves").to_json();
        let second = resolve(&entrypoint).ir.expect("resolves").to_json();
        assert_eq!(first.expect("serializes"), second.expect("serializes"));
    }
}

/// Import order does not affect semantics (grammar 1.4), so it may not affect
/// the artifact either. This builds one synthetic project, writes its
/// `imports:` list in **every** order, and demands byte-identical output — the
/// property PRD 5.12 asks for, tested where it is most easily lost.
#[test]
fn import_order_does_not_change_the_artifact() {
    let dir = scratch("import-order");
    let files = [
        (
            "providers.yml",
            "provider.p:\n  kind: anthropic\n  api_key: ${KEY}\n",
        ),
        (
            "models.yml",
            "model.m:\n  provider: provider.p\n  id: some-model\n",
        ),
        (
            "agents/a.yml",
            "agent.a:\n  model: model.m\n  prompt: Do the thing.\n  output:\n    text: { type: string }\n",
        ),
        (
            "flows/f.yml",
            concat!(
                "flow.f:\n",
                "  outputs:\n",
                "    text: { type: string }\n",
                "  nodes:\n",
                "    run: { agent: agent.a, input: { } }\n",
                "  edges:\n",
                "    - { from: start, to: run }\n",
                "    - { from: run, to: end }\n",
            ),
        ),
    ];
    for (name, body) in files {
        write(&dir, name, body);
    }

    let names: Vec<&str> = files.iter().map(|(name, _)| *name).collect();
    let mut expected: Option<String> = None;
    for order in permutations(&names) {
        let imports: String = order
            .iter()
            .map(|name| format!("  - {name}\n"))
            .collect::<Vec<_>>()
            .concat();
        write(
            &dir,
            "main.yml",
            &format!(
                "version: \"0.1\"\nimports:\n{imports}state:\n  text: {{ type: string, default: \"\" }}\n"
            ),
        );
        let resolution = resolve(dir.join("main.yml"));
        let ir = resolution
            .ir
            .as_ref()
            .unwrap_or_else(|| panic!("the synthetic project resolves:\n{}", render(&resolution)));
        let json = ir.to_json().expect("the artifact serializes");
        match &expected {
            None => expected = Some(json),
            Some(first) => assert_eq!(
                *first, json,
                "importing in the order {order:?} produced a different artifact"
            ),
        }
    }
    assert!(expected.is_some(), "at least one order was resolved");
    let _ = fs::remove_dir_all(&dir);
}

/// No object in the artifact may repeat a key.
///
/// The IR writes several constructs by merging a tagged variant into its
/// container — a node's kind onto the node, a type node's form onto the type
/// node, a definition's body onto the definition — which is what keeps the
/// document reading like the DSL. The hazard that comes with it is a name
/// colliding across the merge, and JSON's own answer to a repeated key is to
/// keep one silently: the artifact would parse, and a field would be gone. A
/// `flow:` node is the case that has both halves — its own `timeout:` (level 2
/// of grammar 9.3) and a `policy:` override for the nodes inside it (level 1,
/// Decision D60) — so it is built here explicitly rather than left to whether
/// an example happens to declare one.
#[test]
fn no_object_in_the_artifact_repeats_a_key() {
    let dir = scratch("unique-keys");
    write(
        &dir,
        "main.yml",
        concat!(
            "version: \"0.1\"\n",
            "flow.inner:\n",
            "  outputs: {}\n",
            "  nodes:\n",
            "    step: { exec: { command: \"true\" } }\n",
            "  edges:\n",
            "    - { from: start, to: step }\n",
            "    - { from: step, to: end }\n",
            "flow.outer:\n",
            "  outputs: {}\n",
            "  nodes:\n",
            "    sub:\n",
            "      flow: flow.inner\n",
            "      policy: { timeout: 30s }\n",
            "      timeout: 10s\n",
            "      on_error: skip\n",
            "  edges:\n",
            "    - { from: start, to: sub }\n",
            "    - { from: sub, to: end }\n",
        ),
    );

    for json in [
        resolve(dir.join("main.yml"))
            .ir
            .expect("the nesting project resolves")
            .to_json()
            .expect("the artifact serializes"),
        resolve_clean("examples/review-loop", "local"),
        resolve_clean("examples/triage-fanout", "staging"),
    ] {
        serde_json::from_str::<unique::Object>(&json).unwrap_or_else(|error| {
            panic!("the artifact repeats a key: {error}");
        });
    }
    let _ = fs::remove_dir_all(&dir);
}

/// A JSON value that refuses to deserialize when any object in it repeats a
/// key. `serde_json`'s own `Value` keeps the last of a repeated pair, so the
/// check has to be made while the keys go past.
mod unique {
    use std::collections::BTreeSet;
    use std::fmt;

    use serde::Deserialize;
    use serde::de::{Deserializer, Error, MapAccess, SeqAccess, Visitor};

    pub struct Object;

    impl<'de> Deserialize<'de> for Object {
        fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            deserializer.deserialize_any(Any)
        }
    }

    struct Any;

    impl<'de> Visitor<'de> for Any {
        type Value = Object;

        fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            formatter.write_str("any JSON value whose objects have distinct keys")
        }

        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut seen: BTreeSet<String> = BTreeSet::new();
            while let Some(key) = map.next_key::<String>()? {
                map.next_value::<Object>()?;
                if !seen.insert(key.clone()) {
                    return Err(A::Error::custom(format!("`{key}` appears twice")));
                }
            }
            Ok(Object)
        }

        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Self::Value, A::Error> {
            while seq.next_element::<Object>()?.is_some() {}
            Ok(Object)
        }

        fn visit_str<E: Error>(self, _: &str) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_bool<E: Error>(self, _: bool) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_i64<E: Error>(self, _: i64) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_u64<E: Error>(self, _: u64) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_f64<E: Error>(self, _: f64) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_unit<E: Error>(self) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_none<E: Error>(self) -> Result<Self::Value, E> {
            Ok(Object)
        }
        fn visit_some<D: Deserializer<'de>>(
            self,
            deserializer: D,
        ) -> Result<Self::Value, D::Error> {
            deserializer.deserialize_any(Any)
        }
    }
}

/// A single-file project is a composition too: no `imports:`, no deploy file,
/// and the zero-config `local` target (grammar 1.4, 14).
#[test]
fn a_single_file_project_resolves() {
    let dir = scratch("single-file");
    write(
        &dir,
        "main.yml",
        "version: \"0.1\"\nprovider.p:\n  kind: anthropic\n  api_key: ${KEY}\n",
    );
    let resolution = resolve(dir.join("main.yml"));
    let ir = resolution
        .ir
        .as_ref()
        .unwrap_or_else(|| panic!("a one-file project resolves:\n{}", render(&resolution)));
    assert_eq!(ir.sources.len(), 1);
    assert_eq!(ir.definitions.len(), 1);
    // A section nobody declared is absent from the artifact, not empty in it.
    assert!(ir.state.is_none() && ir.triggers.is_none() && ir.defaults.is_none());
    assert!(ir.deploy.source.is_none(), "`local` needs no deploy file");
    let _ = fs::remove_dir_all(&dir);
}

/// Grammar 1.4 bars `deploy/*.yml` from `imports:` — one path segment under
/// `deploy/`, which is exactly the set `--target` selects from (grammar 14). A
/// file one level deeper is not one of them, so it imports like any other spec
/// file: refusing it would reject a composition for a rule it does not break,
/// and call the file something it is not while doing so.
///
/// The companion is the `import-names-a-deploy-file` fixture, which pins that
/// `deploy/staging.yml` — a real target file — is still refused.
#[test]
fn a_file_below_deploy_is_an_ordinary_import() {
    let dir = scratch("deploy-subdirectory");
    write(
        &dir,
        "main.yml",
        "version: \"0.1\"\nimports:\n  - deploy/shared/providers.yml\n",
    );
    write(
        &dir,
        "deploy/shared/providers.yml",
        "provider.p:\n  kind: anthropic\n  api_key: ${KEY}\n",
    );
    let resolution = resolve(dir.join("main.yml"));
    let ir = resolution.ir.as_ref().unwrap_or_else(|| {
        panic!(
            "a spec file below `deploy/` is not `deploy/<target>.yml`:\n{}",
            render(&resolution)
        )
    });
    assert_eq!(ir.sources.len(), 2);
    assert!(ir.definition("provider.p").is_some());
    let _ = fs::remove_dir_all(&dir);
}

/// Every section carries the region it was declared in, and an absent section
/// is not an empty one.
///
/// The distinction is only worth drawing if both sides are reachable, and they
/// are: `state: {}` is a composition that declares a channel set and leaves it
/// empty, while no `state:` at all is one that declares none. A rule whose
/// subject is a whole section reports against the first and has nothing to say
/// about the second — which is why the artifact keeps them apart rather than
/// writing both as an empty map.
#[test]
fn a_declared_section_carries_its_span_even_when_it_declares_nothing() {
    let dir = scratch("empty-sections");
    write(
        &dir,
        "main.yml",
        concat!(
            "version: \"0.1\"\n",
            "defaults: {}\n",
            "state: {}\n",
            "triggers: {}\n",
        ),
    );
    let resolution = resolve(dir.join("main.yml"));
    let ir = resolution.ir.as_ref().unwrap_or_else(|| {
        panic!(
            "a project whose sections declare nothing resolves:\n{}",
            render(&resolution)
        )
    });

    let state = ir.state.as_ref().expect("`state:` was declared");
    assert!(state.is_empty());
    assert_eq!(state.span.start.line, 3);
    let triggers = ir.triggers.as_ref().expect("`triggers:` was declared");
    assert!(triggers.is_empty());
    assert_eq!(triggers.span.start.line, 4);
    let defaults = ir.defaults.as_ref().expect("`defaults:` was declared");
    assert_eq!(defaults.span.start.line, 2);

    // What reaches the document is the span, not an empty map.
    let json = ir.to_json().expect("the artifact serializes");
    assert!(
        json.contains("\"state\": {\n    \"span\": \"main.yml:3:8..3:10\"\n  }"),
        "a section that declares nothing is written as its span alone:\n{json}"
    );
    let _ = fs::remove_dir_all(&dir);
}

/// Grammar 14.3's capability rule is checked on both halves of a backend
/// config — the per-kind `defaults:` in the parser, the alias here — and a rule
/// written a little too tight is exactly what a negative corpus cannot catch.
/// So: every store kind bound to an alias whose provider serves it, two stores
/// sharing one alias, and a target that resolves clean.
///
/// The companion is the `store-binds-an-alias-that-serves-another-kind`
/// fixture, which pins the rejection and its exact text.
#[test]
fn every_kind_bound_to_an_alias_that_serves_it_resolves() {
    let dir = scratch("backend-capability");
    write(
        &dir,
        "main.yml",
        concat!(
            "version: \"0.1\"\n",
            "provider.embeddings:\n",
            "  kind: openai\n",
            "  api_key: ${OPENAI_API_KEY}\n",
            "store.sessions_a:\n",
            "  kind: kv\n",
            "  scope: session\n",
            "  backend: sessions\n",
            "  value_schema:\n",
            "    seen: { type: integer }\n",
            "store.sessions_b:\n",
            "  kind: kv\n",
            "  scope: session\n",
            "  backend: sessions\n",
            "  value_schema:\n",
            "    seen: { type: integer }\n",
            "store.docs:\n",
            "  kind: vector\n",
            "  scope: global\n",
            "  backend: docs_db\n",
            "  embed:\n",
            "    model: text-embedding-3-small\n",
            "    provider: provider.embeddings\n",
            "store.artifacts:\n",
            "  kind: blob\n",
            "  scope: global\n",
            "  backend: artifacts\n",
        ),
    );
    write(
        &dir,
        "deploy/staging.yml",
        concat!(
            "version: \"0.1\"\n",
            "storage_backends:\n",
            "  aliases:\n",
            "    sessions: { provider: redis, url: \"${REDIS_URL}\" }\n",
            "    docs_db: { provider: chroma, url: \"${CHROMA_URL}\" }\n",
            "    artifacts: { provider: s3, bucket: acme-artifacts }\n",
        ),
    );

    let resolution = resolve_with_target(dir.join("main.yml"), "staging");
    let ir = resolution.ir.as_ref().unwrap_or_else(|| {
        panic!(
            "a store bound to an alias that serves its kind resolves:\n{}",
            render(&resolution)
        )
    });
    let aliases: Vec<&str> = ir
        .deploy
        .storage_backends
        .as_ref()
        .expect("the staging target declares `storage_backends:`")
        .aliases
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(aliases, ["artifacts", "docs_db", "sessions"]);
    let _ = fs::remove_dir_all(&dir);
}

/// A scratch directory of this test's own, cleaned out before use.
///
/// Named by the process as well as by the test, because it is *emptied* before
/// use: two `cargo test` runs at once — one per worktree, which is how this
/// repo is worked on, or two jobs on one CI runner — would otherwise delete
/// each other's fixtures mid-test and fail for a reason that is not in the
/// code. `CARGO_TARGET_TMPDIR` is the scratch root Cargo hands integration
/// tests; the pid is what separates two runs that share one target directory.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("resolve-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

fn write(dir: &Path, name: &str, body: &str) {
    let path = dir.join(name);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).expect("can create a scratch subdirectory");
    }
    fs::write(&path, body).unwrap_or_else(|e| panic!("cannot write {}: {e}", path.display()));
}

/// Every ordering of `items`, smallest first.
fn permutations<'a>(items: &[&'a str]) -> Vec<Vec<&'a str>> {
    if items.len() <= 1 {
        return vec![items.to_vec()];
    }
    let mut out = Vec::new();
    for (index, item) in items.iter().enumerate() {
        let mut rest = items.to_vec();
        rest.remove(index);
        for mut tail in permutations(&rest) {
            tail.insert(0, item);
            out.push(tail);
        }
    }
    out
}
