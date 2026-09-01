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
///
/// The check pass reports nothing at all, and the assertion is exact: a
/// diagnostic arriving here would mean a legal trigger had become unwritable —
/// or that the surface had grown a report of its own — and no negative fixture
/// would notice either.
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
        "the auth surface should check clean, got:\n{}",
        render(&diagnostics)
    );
    // A composition `validate` accepts is one `build` owes a project to, and
    // every key of this surface is one the emitter reads — an emitter that
    // indexed one wrongly would arrive here as a panic rather than as a
    // diagnostic.
    assert!(
        !compose_core::emit(&ir, &compose_core::Authored::none())
            .files()
            .is_empty(),
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

/// The `header:` and `prefix:` shape rules bind on **every** block that takes
/// one, not on the block a fixture happens to name.
///
/// Three blocks carry the pair — the inbound `bearer:`, the inbound `hmac:`,
/// and the outbound `bearer:` — and three call sites read them. A rule applied
/// at two of the three ships the injection it refuses on whichever block the
/// corpus left out, and nothing downstream recovers it: the resolved value is
/// what a delivery writes onto its own request, verbatim.
#[test]
fn a_forged_header_or_prefix_is_refused_on_every_block_that_takes_one() {
    let sites = [
        (
            "inbound bearer",
            "    auth:\n      bearer:\n        token: ${WEBHOOK_TOKEN}\n",
        ),
        (
            "inbound hmac",
            "    auth:\n      hmac:\n        secret: ${WEBHOOK_SECRET}\n",
        ),
        (
            "outbound bearer",
            "    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n    callback_auth:\n      bearer:\n        token: ${CALLBACK_TOKEN}\n",
        ),
    ];
    let forgeries = [
        (
            "header",
            "\"X-Token: forged\"",
            "`header` must be an HTTP header name",
        ),
        (
            "prefix",
            "\"Bearer \\r\\nX-Injected: 1\"",
            "`prefix` must not contain control characters",
        ),
    ];
    for (site, block) in sites {
        for (key, value, expected) in forgeries {
            let source = format!(
                "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n{block}        {key}: {value}\n"
            );
            let parsed = parse_str(&source, "main.yml");
            assert!(
                parsed
                    .diagnostics
                    .iter()
                    .any(|diagnostic| diagnostic.message.starts_with(expected)),
                "the {site} `{key}` should be refused, got:\n{}",
                render(&parsed.diagnostics)
            );
        }
    }
}

/// A trigger's `header:` takes exactly the form a provider's `headers:` keys
/// take — the claim two shipped documents make, held to the surfaces that make
/// it (grammar 13.3, 12.1).
///
/// The diagnostic tells an author their header must be "the form a provider's
/// `headers:` keys take", and §13.3 says it again in the key table. Neither is
/// a comparison a reader can run: they are one sentence about two call sites,
/// and the sentence is what an author navigates by when the form is wider than
/// they expected. Widen `NameForm::HeaderLike` so a provider may declare
/// `headers: { X.Sig: … }` — `.` is an RFC 7230 token character, so this is a
/// change someone will want — and the trigger surface either follows or ships
/// both sentences false, refusing under a help text that names a surface which
/// accepts.
///
/// So the two are compared on the spellings that straddle the form's edges,
/// verdict against verdict rather than message against message: the messages
/// differ by design (a key names a header, a value is one), and what has to
/// agree is which spellings are headers at all.
#[test]
fn a_triggers_header_takes_the_form_a_providers_headers_keys_take() {
    for spelling in [
        "X-Hub-Signature-256",
        "Authorization",
        "x_signature",
        // The token characters HTTP allows and this form does not — the
        // widening this test exists to catch, whichever surface it reaches.
        "X.Sig",
        "X+Sig",
        // And the spellings that forge a second header rather than name one.
        "X Sig",
        "X-Sig:",
        "X-Sig\r\nInjected: 1",
        "",
    ] {
        let written = format!("{spelling:?}");
        let provider = format!(
            "version: \"0.1\"\nprovider.vendor:\n  kind: openai\n  api_key: ${{OPENAI_API_KEY}}\n  headers:\n    {written}: \"v\"\n"
        );
        let trigger = format!(
            "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    auth:\n      hmac:\n        secret: ${{WEBHOOK_SECRET}}\n        header: {written}\n"
        );
        let as_a_key = parse_str(&provider, "main.yml");
        let as_a_value = parse_str(&trigger, "main.yml");
        assert_eq!(
            as_a_key.diagnostics.is_empty(),
            as_a_value.diagnostics.is_empty(),
            "`{spelling}` is a header name on one surface and not on the other:\n  \
             as a provider `headers:` key:\n{}\n  as a trigger `header:`:\n{}",
            render(&as_a_key.diagnostics),
            render(&as_a_value.diagnostics)
        );
    }
}

/// The delivery's own header namespace is reserved against an outbound
/// `bearer:` — and against nothing else (grammar 13.3, Decision D127).
///
/// Five `X-AgentCompose-*` headers are normative for a receiver, so a delivery
/// asked to carry its static token under one of them writes two values to one
/// name: the receiver reads whichever its HTTP stack kept, and its signature
/// check then fails on every legitimate delivery — or passes on one whose
/// signature it never read. The rule is the whole prefix rather than the five
/// spellings, because the wire may grow a sixth and a receiver would meet the
/// same collision.
///
/// The inbound direction is deliberately untouched, and this test pins that
/// half too: a trigger *receiving* another deployment's callbacks verifies them
/// by naming `X-AgentCompose-Signature` in its own `auth:`, exactly as it would
/// name a vendor's. A rule applied to both directions would make the documented
/// recipe unwritable.
#[test]
fn a_delivery_header_is_refused_outbound_and_stays_legal_inbound() {
    for name in [
        "X-AgentCompose-Signature",
        "x-agentcompose-delivery",
        "X-AgentCompose-Anything",
    ] {
        let source = format!(
            "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n    callback_auth:\n      bearer:\n        token: ${{CALLBACK_TOKEN}}\n        header: \"{name}\"\n"
        );
        let parsed = parse_str(&source, "main.yml");
        assert!(
            parsed.diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .starts_with("`header` must not name an `X-AgentCompose-` delivery header")),
            "a delivery may not carry its token under `{name}`, got:\n{}",
            render(&parsed.diagnostics)
        );
    }

    let source = format!(
        "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    auth:\n      hmac:\n        secret: ${{WEBHOOK_SECRET}}\n        header: X-AgentCompose-Signature\n        prefix: \"sha256=\"\n"
    );
    let ir = resolve_clean("delivery-header-inbound", &source);
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
    assert_eq!(
        hmac.header, "X-AgentCompose-Signature",
        "verifying another deployment's deliveries is what naming this header inbound is for"
    );
}

/// The namespace is not the whole of what a delivery writes, and the rule is
/// about the collision rather than the spelling (grammar 13.3, Decision D127).
///
/// A delivery is a POST of the status route's report, as JSON, to the host the
/// allowlist admitted — so it writes `Content-Type`, `Content-Length` and `Host`
/// on its own request whatever the wire table says. A token asked for under one
/// of those is the same two-values-one-name failure the namespace rule refuses,
/// and a worse one to debug: the receiver answers 415 on a content type that is
/// a credential, or reads a body framed by a token's length, or is never reached
/// because `Host` named somewhere else. Every `parked` and `settled` delivery
/// for that trigger is lost, and `validate` had said the spec was good.
///
/// The three are matched **whole**, unlike the namespace's prefix: a receiver's
/// own `X-Content-Type` or `Content-Type-Signature` collides with nothing, and a
/// rule that swallowed them would refuse a working configuration. And **outbound
/// only**, like the namespace rule and through the same shared reader: inbound,
/// `header:` is the name a caller's header is looked up by, and this deployment
/// writes nothing on a request it received.
#[test]
fn a_transport_header_is_refused_outbound_and_stays_legal_inbound() {
    let outbound = |header: &str| {
        format!(
            "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n    callback_auth:\n      bearer:\n        token: ${{CALLBACK_TOKEN}}\n        header: \"{header}\"\n"
        )
    };
    for (header, named) in [
        ("Content-Type", "Content-Type"),
        ("content-type", "Content-Type"),
        ("Content-Length", "Content-Length"),
        ("host", "Host"),
    ] {
        let parsed = parse_str(&outbound(header), "main.yml");
        let expected =
            format!("`header` must not name `{named}`, which every delivery writes itself");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message.starts_with(&expected)),
            "a delivery may not carry its token under `{header}`, got:\n{}",
            render(&parsed.diagnostics)
        );
    }

    for header in ["X-Content-Type", "Content-Type-Signature", "Hosting"] {
        let parsed = parse_str(&outbound(header), "main.yml");
        assert!(
            parsed.diagnostics.is_empty(),
            "`{header}` is a receiver's own header and collides with nothing, got:\n{}",
            render(&parsed.diagnostics)
        );
    }

    let inbound = format!(
        "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    auth:\n      bearer:\n        token: ${{WEBHOOK_TOKEN}}\n        header: Content-Type\n"
    );
    let parsed = parse_str(&inbound, "main.yml");
    assert!(
        parsed.diagnostics.is_empty(),
        "inbound the name is looked up in what a caller sent, and this deployment writes \
         nothing on that request, got:\n{}",
        render(&parsed.diagnostics)
    );
}

/// Both inbound schemes state the **constant-time** requirement, in both
/// documents an implementer reads (grammar 13.3, PRD resolved q32).
///
/// §13.3 opens by saying it is "what the M3 runtime is written against", and the
/// runtime half of this surface is written from these two bullets alone. State
/// the requirement under `bearer` only — which is what both documents did — and
/// the implementer writes `timingSafeEqual` for the token and `computed ===
/// provided` for the signature, because only one bullet asked. A caller who can
/// time the response then recovers the expected digest for a body of their
/// choosing byte by byte and forges a validly signed request without ever
/// holding `${WEBHOOK_SECRET}`: the whole scheme defeated, by an asymmetry
/// nothing else in this repository can see.
///
/// The bullet is located rather than the document searched, because the failure
/// is exactly a claim living under the *other* scheme.
///
/// What the runtime *does* about the requirement is the sibling below's, and the
/// two are worth having separately: this one fails when a document stops asking,
/// which is how the next implementer comes to write the wrong comparison.
#[test]
fn both_inbound_schemes_require_a_constant_time_comparison() {
    let grammar = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("the manifest directory has a grandparent")
            .join("docs/grammar.md"),
    )
    .expect("the grammar is readable");
    let topic = compose_core::docs::topic("triggers").expect("the `triggers` topic ships");

    /// The list item opening `- **\`<scheme>\`**`, up to the next one.
    fn bullet<'a>(document: &'a str, scheme: &str) -> &'a str {
        let opener = format!("- **`{scheme}`**");
        let start = document
            .find(&opener)
            .unwrap_or_else(|| panic!("a document states what `{scheme}` does"));
        let rest = &document[start + opener.len()..];
        rest.find("\n- ").map_or(rest, |end| &rest[..end])
    }

    for (document, source) in [
        (grammar.as_str(), "grammar 13.3"),
        (topic.body, "the topic"),
    ] {
        for scheme in ["bearer", "hmac"] {
            assert!(
                bullet(document, scheme).contains("constant"),
                "{source}'s `{scheme}` bullet no longer requires a constant-time comparison, \
                 and it is where the next implementer of this surface reads about one"
            );
        }
    }
}

/// …and the app **makes** one, on both schemes (grammar 13.3, Decision D125).
///
/// The requirement above is the only security property this surface has, and
/// until the runtime landed prose was the whole of what held it. It is not any
/// more, and a document nobody disagrees with is the weakest possible binding
/// for a property whose loss is invisible: a later cleanup writes `return
/// offered === expected` — the `TextEncoder` round trip reads as ceremony — and
/// every gate in this repository stays green, because every other assertion
/// about `auth:` is about *which* requests are refused and none is about how
/// long a refusal took. What ships then leaks, byte by byte to a caller who can
/// time it, the expected HMAC digest for a body of that caller's choosing, and
/// a request forged with a digest recovered that way is accepted without the
/// caller ever holding `${WEBHOOK_SECRET}`.
///
/// So it is read off the emitted app, at the seam rather than over the file:
/// **both** of `verified`'s schemes have to reach the comparison, which is the
/// asymmetry D125 is written about, and the comparison has to be the one
/// `node:crypto` provides. A file-wide search for `timingSafeEqual` would be
/// satisfied by the bearer half alone.
#[test]
fn both_inbound_schemes_compare_in_constant_time_in_the_generated_app() {
    let ir = resolve_clean("constant-time", &source());
    let generated = compose_core::emit(&ir, &compose_core::Authored::none());
    let app = generated
        .files()
        .iter()
        .find(|file| file.path == "src/serve.ts")
        .expect("every project emits the app over its triggers")
        .contents
        .clone();

    // The three routes' one guard, split at the digest: what is before
    // `createHmac` is the `bearer` half and what is after it is the `hmac` half.
    let verified = function_body(&app, "function verified(");
    let digest = verified
        .find("createHmac(")
        .expect("`verified` computes the signature it verifies");
    for (half, scheme) in [
        (&verified[..digest], "bearer"),
        (&verified[digest..], "hmac"),
    ] {
        assert!(
            half.contains("equal("),
            "`verified`'s `{scheme}` half compares the credential itself rather than through \
             the constant-time comparison, so a caller who can time a refusal recovers what \
             this trigger expects (grammar 13.3, D125)"
        );
    }

    let equal = function_body(&app, "function equal(");
    assert!(
        equal.contains("timingSafeEqual("),
        "the comparison both schemes reach no longer uses `node:crypto`'s constant-time one: \
         a byte-by-byte early return leaks the token to a caller who can time it, and the \
         expected digest for a body of that caller's choosing (grammar 13.3, D125)"
    );
    assert!(
        !equal.contains("offered ===") && !equal.contains("=== offered"),
        "the credential a caller offered is compared directly somewhere in `equal`, which is \
         the leak `timingSafeEqual` is there to prevent however the call beside it reads"
    );
    assert!(
        app.contains("import { createHmac, timingSafeEqual } from \"node:crypto\";"),
        "`src/serve.ts` no longer takes its comparison from the runtime's own crypto module"
    );
}

/// The body of one top-level declaration of an emitted module.
///
/// `compose-core`'s own `codegen::journal` and `tests/trace_format_inventory.rs`
/// read the emitted modules the same way, and for the same reason: a rule about
/// what one function does is only a rule if it is read off that function rather
/// than off the file around it. The module is formatted, so a top-level
/// declaration opens at column zero and closes on a line that is exactly `}`.
fn function_body(source: &str, header: &str) -> String {
    let mut lines = source.lines().skip_while(|line| !line.starts_with(header));
    let opened = lines
        .next()
        .unwrap_or_else(|| panic!("`src/serve.ts` declares `{header}…`"));
    let mut held = String::from(opened);
    for line in lines {
        held.push('\n');
        held.push_str(line);
        if line == "}" {
            return held;
        }
    }
    panic!("`{header}…` has no closing brace in the first column")
}

/// The enforced claim, bound to the thing that enforces it — and the list of
/// documents that must not go back on it.
///
/// `auth:`, `callback_auth:` and `callback_allow:` were **reserved** grammar
/// until the http-native events pass: parsed, checked, carried into the IR, and
/// read by nothing that was generated. Four shipped documents said so in as
/// many words, and this test was their opposite — it asserted that none of an
/// authenticated trigger's material reached a built project, and enumerated the
/// five places to unwind when the runtime landed.
///
/// It has been turned around, because the asymmetry that made it worth writing
/// runs in both directions. A no-op `schedule` runs nothing and is visibly
/// inert, while a no-op `auth:` **serves every caller** and is
/// indistinguishable, from outside, from a guarded route — so a reader told the
/// wrong thing about this surface is told something false about a security
/// control whichever way the sentence points. What is asserted now is that the
/// material really does reach the generated project, that the credentials reach
/// the **environment manifest** so a deployment missing one is refused at
/// launch, and that the documents which used to call the surface inert say it
/// is enforced. A change that made the keys inert again — an emitter that
/// stopped writing them, a walk that stopped visiting `ir.triggers` — fails
/// here rather than shipping a binary whose `agent-compose docs triggers`
/// promises a guarantee the app does not keep.
#[test]
fn the_auth_surface_reaches_the_generated_app_and_every_document_says_so() {
    let ir = resolve_clean("enforced", &source());
    let generated = compose_core::emit(&ir, &compose_core::Authored::none());
    let triggers_module = generated
        .files()
        .iter()
        .find(|file| file.path == "src/triggers.ts")
        .expect("a composition with triggers emits the table")
        .contents
        .clone();
    let environment = generated
        .files()
        .iter()
        .find(|file| file.path == "src/env.ts")
        .expect("every project emits its environment manifest")
        .contents
        .clone();

    // Everything that has to reach the runtime for these keys to be enforced:
    // the header names and the allowlist, which are values the app compares
    // against, and the credentials, which reach it as the **names** of the
    // variables holding them (grammar 4.3 — `build` resolves nothing).
    for material in [
        "X-Hub-Signature-256",
        "X-Delivery-Token",
        "hooks.example.com",
        "sha512",
        "base64",
        "\"WEBHOOK_SECRET\"",
        "\"WEBHOOK_TOKEN\"",
        "\"CALLBACK_SECRET\"",
        "\"CALLBACK_TOKEN\"",
    ] {
        assert!(
            triggers_module.contains(material),
            "`src/triggers.ts` carries no `{material}`, so the app cannot enforce what the \
             trigger declared"
        );
    }
    for reference in ["${WEBHOOK_SECRET}", "${CALLBACK_TOKEN}"] {
        assert!(
            !triggers_module.contains(reference),
            "a credential reaches generated code as a variable *name* and never as a resolved \
             value or an uninterpreted `{reference}` (grammar 4.3)"
        );
    }

    // The launch check, which is the half with no sentence to bind: a runtime
    // reading `process.env.WEBHOOK_TOKEN` that `References::of` did not know
    // about would let a deployment missing the variable start clean and then
    // refuse every real call. `readEnvironment()` reports only what this list
    // holds.
    for (variable, site) in [
        ("WEBHOOK_SECRET", "triggers.intake.auth.hmac.secret"),
        ("WEBHOOK_TOKEN", "triggers.minimal.auth.bearer.token"),
        (
            "CALLBACK_TOKEN",
            "triggers.intake.callback_auth.bearer.token",
        ),
        (
            "CALLBACK_SECRET",
            "triggers.intake.callback_auth.hmac.secret",
        ),
    ] {
        assert!(
            environment.contains(&format!("name: \"{variable}\"")),
            "`src/env.ts` does not name `{variable}`, so a deployment missing it starts clean \
             and fails on every real call (grammar 4.3, 15)"
        );
        assert!(
            environment.contains(&format!("\"{site}\"")),
            "`src/env.ts` does not say where `{variable}` is referenced (`{site}`)"
        );
    }

    // …and the documents. Each is checked for the sentence that would become
    // false if the surface went inert again, so a reserved posture reintroduced
    // in one document fails here rather than silently.
    let triggers = compose_core::docs::topic("triggers").expect("the `triggers` topic ships");
    let targets = compose_core::docs::topic("targets").expect("the `targets` topic ships");
    let allowlist = compose_core::docs::explanation(
        compose_core::docs::codes::named("missing-callback-allowlist")
            .expect("the code is registered"),
    );
    let grammar = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(2)
            .expect("the manifest directory has a grandparent")
            .join("docs/grammar.md"),
    )
    .expect("the grammar is readable");
    for (document, claim) in [
        (grammar.as_str(), "are enforced by the generated\napp"),
        (
            grammar.as_str(),
            "**The three authentication keys were here until the http-native events pass**",
        ),
        (
            triggers.body,
            "**All three keys below are enforced by the generated app.**",
        ),
        (
            triggers.body,
            "**The three authentication keys were on this list and have left it.**",
        ),
        (
            targets.body,
            "An `http` trigger's `auth:`, `callback_auth:` and `callback_allow:` have left it",
        ),
        (allowlist, "**The check and the matching are both live.**"),
    ] {
        assert!(
            document.contains(claim),
            "a document that states the enforced posture no longer contains {claim:?} — \
             the sentence has to come back, or the runtime has to go"
        );
    }

    // The reserved posture must survive **nowhere**: no shipped document may
    // still file one of these three keys among the constructs a deployment must
    // not rely on.
    for (name, document) in [
        ("docs/grammar.md", grammar.as_str()),
        ("the `triggers` topic", triggers.body),
        ("the `targets` topic", targets.body),
        ("the `missing-callback-allowlist` explanation", allowlist),
    ] {
        for row in [
            "| `triggers.<t>.auth` |",
            "| `triggers.<t>.callback_auth` |",
            "| `triggers.<t>.callback_allow` |",
        ] {
            assert!(
                !document.contains(row),
                "{name} still files `{row}` among the reserved constructs, and the app enforces it"
            );
        }
    }
}

/// A project in the **committed golden corpus** populates all three keys, so
/// the generated-code gates type-check the values the emitter writes.
///
/// `tests/generated_code_gates.rs` runs `tsc` over every golden, and what that
/// buys for this surface depends entirely on a golden carrying a *value*: a
/// corpus in which no trigger declares `auth:` type-checks the `InboundAuth` and
/// `CallbackAuth` declarations against nothing. An emitter that wrote `secret:`
/// where the interface says `secretEnv:` — a plausible symmetry edit, since the
/// inbound block spells the same credential differently — would then be caught
/// by no gate at all: the string-matching tests in `codegen::trigger` move with
/// it, and the acceptance suite that would meet the failure at runtime returns
/// early on a machine with no JavaScript toolchain. A contributor without Bun
/// would see a fully green `cargo test` for a build whose served app throws on
/// its first signed delivery.
///
/// So the corpus property is asserted here rather than left to whoever next
/// edits `tests/projects/every-schema-form/main.yml`.
#[test]
fn a_golden_project_carries_a_populated_auth_table() {
    let goldens = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/goldens");
    let tables: Vec<String> = fs::read_dir(&goldens)
        .expect("the golden corpus exists")
        .filter_map(|entry| {
            let path = entry
                .expect("a readable entry")
                .path()
                .join("src/triggers.ts");
            fs::read_to_string(path).ok()
        })
        .collect();
    assert!(
        !tables.is_empty(),
        "no golden emits `src/triggers.ts`, so nothing type-checks the trigger table at all"
    );
    for populated in ["    auth: {", "    callbackAuth: {", "    callbackAllow: ["] {
        assert!(
            tables.iter().any(|table| table.contains(populated)),
            "no golden's `src/triggers.ts` contains `{populated}`, so `tsc` checks the \
             declaration of this key against no value the emitter ever wrote: give a golden \
             project's `http` trigger the key back (grammar 13.3)"
        );
    }
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

/// An allowlist entry is an absolute URL that names a host, and shape is the
/// whole of what this pass reads (grammar 13.3, Decision D127).
///
/// The refusals are the entries that name no receiver at all: no scheme, a
/// scheme this delivery cannot speak, and — the shape the schema's own pattern
/// has always refused — a delimiter sitting where the host should be, which the
/// three-delimiter scan would otherwise walk straight past with an empty host in
/// hand. A fixture pins each exact message.
///
/// What the legal list is for is the other half, and it is where a rule written
/// one character too tight does its damage: a port, a query string, an exact URL
/// carrying no wildcard, and a wildcard **inside the host** are all entries an
/// author writes, and refusing one of them makes a correct allowlist unwritable
/// while every negative fixture still passes.
///
/// `https://*.hooks.example.com/*` is legal, and §13.3 is where what it *means*
/// is said: `*` is any run of characters and crosses `/` and `?`, so that entry
/// admits `https://attacker.test/collect?x=.hooks.example.com/y` too. Narrowing
/// a wildcard inside the authority is a second wildcard kind — a language
/// decision the PRD owns rather than this pass, and one whose only compiling
/// repair on a signed trigger would be dropping `callback_auth:`, the posture
/// D126 exists to prevent. Matching is runtime's; entry shape is all `validate`
/// owns (D127).
#[test]
fn an_allowlist_entry_is_an_absolute_url_that_names_a_host() {
    let allowlist = |pattern: &str| {
        format!(
            "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"{pattern}\"\n"
        )
    };
    for pattern in [
        // No scheme, and a callback URL is absolute.
        "hooks.example.com/*",
        // A scheme no callback is delivered over.
        "ftp://hooks.example.com/*",
        // …and one that *is* one of the two, spelled in a case the match will
        // never see: an entry is compared to the URL as written.
        "HTTPS://hooks.example.com/*",
        // A scheme and nothing at all after it…
        "https://",
        // …and the three ways a host can be missing from something that has an
        // authority-shaped delimiter after the scheme.
        "https:///deliveries",
        "https://?tenant=acme",
        "https://#fragment",
    ] {
        let parsed = parse_str(&allowlist(pattern), "main.yml");
        assert!(
            parsed.diagnostics.iter().any(|diagnostic| diagnostic
                .message
                .contains("is not a `callback_allow` pattern of trigger `intake`")),
            "`{pattern}` names no receiver and must be refused, got:\n{}",
            render(&parsed.diagnostics)
        );
    }
    for pattern in [
        "https://hooks.example.com/*",
        "https://hooks.example.com:9000/*",
        "http://localhost:9000/*",
        "https://hooks.example.com/webhooks/intake",
        "https://hooks.example.com?tenant=*",
        "https://hooks.example.com",
        // A wildcard in the host is legal grammar, and §13.3 says what it
        // admits rather than the parser refusing the shape.
        "https://*.hooks.example.com/*",
        "https://hooks.example.com*",
        "http://*.localhost:9000/*",
        "https://*",
    ] {
        let parsed = parse_str(&allowlist(pattern), "main.yml");
        assert!(
            parsed.diagnostics.is_empty(),
            "`{pattern}` names a host and is legal, got:\n{}",
            render(&parsed.diagnostics)
        );
    }

    // A case-varied scheme gets a message of its own, and this is what it is
    // for: `HTTPS` *is* one of the two schemes a callback is delivered over, so
    // the general refusal would tell its author their scheme is not one of two
    // schemes, one of which is theirs — a sentence with no repair in it. The
    // spelling is the repair, and the message has to carry it.
    let parsed = parse_str(&allowlist("HTTPS://hooks.example.com/*"), "main.yml");
    assert!(
        parsed
            .diagnostics
            .iter()
            .any(|diagnostic| diagnostic.message.ends_with(
                "it names the scheme `HTTPS`, and an entry is matched against the callback URL \
                 as written — write `https://`"
            )),
        "a scheme written in another case is refused by its spelling, got:\n{}",
        render(&parsed.diagnostics)
    );
}

/// The outbound `hmac:` takes `secret:` and nothing else (grammar 13.3,
/// Decision D127).
///
/// The failure this guards is a symmetry edit. The inbound `hmac:` takes
/// `header:`, `algorithm:`, `encoding:` and `prefix:` because there a vendor
/// made those choices; outbound they are the deployment's own, and fixing them
/// at HMAC-SHA256/hex under `X-AgentCompose-Signature` is what lets one
/// verification recipe serve every agent-compose deployment. Adding them here
/// "for symmetry" would ship a spec that validates and then signs its
/// deliveries with a digest no receiver written against the published header
/// table can verify — so each of the four is refused by name, and
/// `trigger-callback-auth-hmac-choosing-an-algorithm.yml` pins the message.
#[test]
fn an_outbound_hmac_takes_a_secret_and_nothing_else() {
    for (key, value) in [
        ("header", "X-Delivery-Signature"),
        ("algorithm", "sha512"),
        ("encoding", "base64"),
        ("prefix", "\"sha512=\""),
    ] {
        let source = format!(
            "version: \"0.1\"\n{FLOW}\ntriggers:\n  intake:\n    type: http\n    flow: flow.support\n    callback: \"payload.body.callback_url\"\n    callback_allow:\n      - \"https://hooks.example.com/*\"\n    callback_auth:\n      hmac:\n        secret: ${{CALLBACK_SECRET}}\n        {key}: {value}\n"
        );
        let parsed = parse_str(&source, "main.yml");
        let expected =
            format!("unknown key `{key}` in the `hmac` of the `callback_auth` of trigger `intake`");
        assert!(
            parsed
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.message == expected),
            "an outbound `hmac:` takes no `{key}:`, got:\n{}",
            render(&parsed.diagnostics)
        );
    }
}
