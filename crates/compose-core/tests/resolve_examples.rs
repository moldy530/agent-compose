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
    assert!(!ir.deploy.placements.is_empty());
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

/// A resolution that reports nothing produces an artifact, and one that reports
/// an error produces none: half an artifact would send the next pass chasing
/// failures that belong to this one.
#[test]
fn an_unresolvable_composition_produces_no_artifact() {
    let missing = repo_root().join("examples/review-loop/does-not-exist.yml");
    let resolution = resolve(&missing);
    assert!(resolution.ir.is_none());
    assert!(resolution.has_errors());
    assert_eq!(resolution.diagnostics.len(), 1);
    assert_eq!(resolution.diagnostics[0].code.as_str(), "io-error");
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
    assert!(ir.state.is_empty() && ir.triggers.is_empty());
    assert!(ir.deploy.source.is_none(), "`local` needs no deploy file");
    let _ = fs::remove_dir_all(&dir);
}

/// A scratch directory of this test's own, cleaned out before use.
fn scratch(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-compose-resolve-{name}"));
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
