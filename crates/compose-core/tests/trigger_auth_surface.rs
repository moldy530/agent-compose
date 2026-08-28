//! The `http` trigger's authentication surface, end to end through the static
//! passes: `auth:`, `callback_auth:`, and `callback_allow:` (grammar 13.3,
//! PRD resolved q32/q33).
//!
//! Three things are pinned here, and each fails a different way.
//!
//! * **The parser accepts the whole surface.** Over-rejection is what a negative
//!   corpus cannot catch: a rule written slightly too tight makes a legal
//!   trigger unwritable, and nothing else would notice.
//! * **The AST records what was written.** The snapshot is the shape the
//!   resolver consumes, so a change to it is a reviewable diff rather than a
//!   silent change to what every later pass reads.
//! * **The IR carries the scheme *resolved*.** Unlike `path:` and `method:`,
//!   which record the author's silence and leave the default to whoever reads
//!   them, a declared scheme lands in the artifact complete — every parameter
//!   that decides whether a credential verifies carries the value the trigger
//!   enforces. A verifier is the wrong place to re-derive a default: getting one
//!   wrong there does not fail a build, it accepts the wrong request. The
//!   defaulted trigger's snapshot is what holds that, beside the declared one's.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, parse_str, resolve};

/// A flow every case can point a trigger at, needing no provider and no model.
const FLOW: &str = r#"
flow.support:
  outputs: {}
  nodes:
    approve:
      human:
        input: {}
        output:
          decision: { enum: [approve, reject] }
  edges:
    - { from: start, to: approve }
    - { from: approve, to: end }
"#;

/// Two triggers: one writing every optional key of every block, one writing
/// none of them.
///
/// `intake` is also the shape resolved q33 calls "and/or": a `callback_auth:`
/// declaring **both** schemes, which the inbound `auth:` of `minimal` may not
/// do.
const TRIGGERS: &str = r#"
triggers:
  intake:
    type: http
    flow: flow.support
    path: /intake
    callback: "payload.body.callback_url"
    auth:
      hmac:
        secret: ${WEBHOOK_SECRET}
        header: X-Hub-Signature-256
        algorithm: sha512
        encoding: base64
        prefix: "sha512="
    callback_auth:
      bearer:
        token: ${CALLBACK_TOKEN}
        header: X-Delivery-Token
        prefix: "Token "
      hmac:
        secret: ${CALLBACK_SECRET}
    callback_allow:
      - "https://hooks.example.com/*"
      - "http://localhost:9000/*"

  minimal:
    type: http
    flow: flow.support
    callback: "payload.body.callback_url"
    auth:
      bearer:
        token: ${WEBHOOK_TOKEN}
    callback_auth:
      hmac:
        secret: ${CALLBACK_SECRET}
    callback_allow:
      - "https://hooks.example.com/*"
"#;

fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} [{}] {}",
                diagnostic.span, diagnostic.code, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// A scratch directory of this test's own, cleaned out before use — named by
/// the process as well as by the case, as the other resolving corpora are.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("trigger-auth-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

/// Resolve a one-file project, asserting nothing was reported.
fn resolve_clean(name: &str, source: &str) -> compose_core::Ir {
    let dir = scratch(name);
    fs::write(dir.join("main.yml"), source).expect("can write the project");
    let resolution = resolve(dir.join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "{name} should resolve cleanly, got:\n{}",
        render(&resolution.diagnostics)
    );
    resolution
        .ir
        .expect("a clean resolution produces an artifact")
}

fn source() -> String {
    format!("version: \"0.1\"\n{FLOW}{TRIGGERS}")
}

/// The parser accepts every key of the surface, written out and left out.
#[test]
fn the_whole_auth_surface_parses_and_resolves_clean() {
    let parsed = parse_str(&source(), "main.yml");
    assert!(
        parsed.diagnostics.is_empty(),
        "the auth surface should parse cleanly, got:\n{}",
        render(&parsed.diagnostics)
    );
    // …and checks clean, which is the pass that would refuse a trigger for what
    // its flow is rather than for what it declares.
    let ir = resolve_clean("surface", &source());
    let diagnostics = compose_core::check(&ir);
    assert!(
        diagnostics.is_empty(),
        "the auth surface should check cleanly, got:\n{}",
        render(&diagnostics)
    );
    // A composition `validate` accepts is one `build` owes a project to, and
    // codegen does not read these keys yet — an emitter that indexed one anyway
    // would arrive here as a panic rather than as a diagnostic.
    assert!(
        !compose_core::emit(&ir).files().is_empty(),
        "the auth surface validates, so `build` owes it a project"
    );
}

/// The AST: what the author wrote, and nothing filled in.
#[test]
fn snapshot_the_parsed_auth_surface() {
    let parsed = parse_str(&source(), "main.yml");
    let document = parsed.document.expect("the surface parses to a document");
    let compose_core::ast::Document::Spec(file) = document else {
        panic!("a file declaring `triggers:` is a spec file");
    };
    insta::assert_debug_snapshot!(file.triggers.expect("the `triggers:` section is read"));
}

/// The IR: the same two triggers with every scheme default applied, so a later
/// reader verifies against a value rather than against a table of its own.
#[test]
fn snapshot_the_resolved_auth_surface() {
    let ir = resolve_clean("resolved", &source());
    let artifact: serde_json::Value =
        serde_json::from_str(&ir.to_json().expect("the artifact serializes"))
            .expect("the artifact is JSON");
    insta::assert_snapshot!(
        serde_json::to_string_pretty(&artifact["triggers"]).expect("a printable trigger table")
    );
}

/// The defaults are grammar 13.3's own, read off the resolved artifact rather
/// than off the snapshot — a snapshot review can bless a wrong value, and these
/// four decide whether a signature verifies.
#[test]
fn an_undeclared_scheme_parameter_resolves_to_the_grammars_default() {
    let ir = resolve_clean("defaults", &source());
    let triggers = ir.triggers.as_ref().expect("the artifact holds a table");
    let trigger = triggers.get("minimal").expect("`minimal` is declared");
    let compose_core::ir::trigger::TriggerKind::Http(http) = &trigger.kind else {
        panic!("`minimal` is an `http` trigger");
    };

    let Some(compose_core::ir::trigger::InboundAuth::Bearer(bearer)) = &http.auth else {
        panic!("`minimal` declares an inbound `bearer` scheme");
    };
    assert_eq!(bearer.token.value.name, "WEBHOOK_TOKEN");
    assert_eq!(bearer.header, "Authorization");
    assert_eq!(bearer.prefix, "Bearer ");

    let hmac = http
        .callback_auth
        .as_ref()
        .and_then(|auth| auth.hmac.as_ref())
        .expect("`minimal` signs its callbacks");
    assert_eq!(hmac.secret.value.name, "CALLBACK_SECRET");

    // …and the inbound hmac defaults, which `intake` declares away from.
    let trigger = triggers.get("intake").expect("`intake` is declared");
    let compose_core::ir::trigger::TriggerKind::Http(http) = &trigger.kind else {
        panic!("`intake` is an `http` trigger");
    };
    let Some(compose_core::ir::trigger::InboundAuth::Hmac(hmac)) = &http.auth else {
        panic!("`intake` declares an inbound `hmac` scheme");
    };
    assert_eq!(hmac.header, "X-Hub-Signature-256");
    assert_eq!(
        hmac.algorithm,
        compose_core::ast::HmacAlgorithm::Sha512,
        "a declared `algorithm:` is the one that lands"
    );
    assert_eq!(hmac.encoding, compose_core::ast::SignatureEncoding::Base64);
    assert_eq!(hmac.prefix, "sha512=");

    // The undeclared half of the same block, on a trigger that declares none of
    // them: `X-Signature`, `sha256`, `hex`, and no prefix at all.
    let defaulted = format!(
        "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    auth:\n      hmac:\n        secret: ${{WEBHOOK_SECRET}}\n"
    );
    let ir = resolve_clean("hmac-defaults", &defaulted);
    let trigger = ir
        .triggers
        .as_ref()
        .expect("the artifact holds a table")
        .get("intake")
        .expect("`intake` is declared");
    let compose_core::ir::trigger::TriggerKind::Http(http) = &trigger.kind else {
        panic!("`intake` is an `http` trigger");
    };
    let Some(compose_core::ir::trigger::InboundAuth::Hmac(hmac)) = &http.auth else {
        panic!("`intake` declares an inbound `hmac` scheme");
    };
    assert_eq!(hmac.header, "X-Signature");
    assert_eq!(hmac.algorithm, compose_core::ast::HmacAlgorithm::Sha256);
    assert_eq!(hmac.encoding, compose_core::ast::SignatureEncoding::Hex);
    assert_eq!(hmac.prefix, "");
}

/// A trigger with a `callback:` and no outbound auth at all is the posture
/// resolved q33 documents rather than refuses: it may POST anywhere, and needs
/// no allowlist to do it.
#[test]
fn a_callback_with_no_outbound_auth_needs_no_allowlist() {
    let source = format!(
        "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n"
    );
    let parsed = parse_str(&source, "main.yml");
    assert!(
        parsed.diagnostics.is_empty(),
        "the test posture is legal, got:\n{}",
        render(&parsed.diagnostics)
    );
}

/// `callback_allow:` without `callback_auth:` is legal too: an allowlist bounds
/// where a webhook may go whether or not the delivery is signed, and the
/// mandatory direction is only the other one.
#[test]
fn an_allowlist_without_outbound_auth_is_legal() {
    let source = format!(
        "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n"
    );
    let parsed = parse_str(&source, "main.yml");
    assert!(
        parsed.diagnostics.is_empty(),
        "an allowlist stands on its own, got:\n{}",
        render(&parsed.diagnostics)
    );
}
