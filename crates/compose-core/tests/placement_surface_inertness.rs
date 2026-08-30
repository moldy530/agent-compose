//! The distributed surface, end to end through the static passes — and the bind
//! that says its runtime has not landed (grammar 14.1, 14.2,
//! `docs/distributed.md`, PRD resolved q37–q44).
//!
//! `hub:` and `placements:` are **live static grammar**: every rule about them
//! is enforced by `validate` today, and every one of them is carried into the
//! artifact. What does not exist yet is the protocol — the worker verb, the five
//! routes of `docs/distributed.md` §3, the artifact server among them, placement
//! waits on the board, and effect streaming. This file is that middle state written down
//! in executable form, and it is the sibling of
//! `tests/trigger_auth_surface.rs`, which holds the *opposite* state for a
//! surface whose runtime did land.
//!
//! Three things are pinned, and each fails a different way.
//!
//! * **The surface parses and resolves, and reaches the artifact.** Over-
//!   rejection is what a negative corpus cannot catch, and a key silently
//!   dropped on the way to the IR is what a positive one cannot: a placement
//!   that stopped being carried would leave `plan` blind to a deployment change
//!   and leave the runtime nothing to read.
//! * **None of it reaches the generated project.** Not the placement names, not
//!   the join token's variable, not the public base. A build that started
//!   emitting half a protocol is worse than one that emits none of it, because
//!   the half would look like a mesh that works.
//! * **Every shipped document says so.** A reader told "the rules are enforced"
//!   must also be told "nothing runs yet", or they will deploy a mesh and wait
//!   for a worker that has nowhere to join. And no shipped document may describe
//!   a shape `validate` refuses: the re-cut left the *whole* surface live, so a
//!   document still carrying the retired one is not a stale paragraph but an
//!   instruction to write a deploy file that does not compile.
//!
//! # The unwind list — what the runtime pass must flip
//!
//! When the worker protocol lands, this file is turned around the way
//! `trigger_auth_surface.rs` was. These are the exact points, and nothing else
//! in the repository enumerates them:
//!
//! 1. **`docs/grammar.md` §14.2's awaiting-runtime sentence** — "The keys are
//!    live static grammar today: every rule above is enforced by `validate`, and
//!    the `worker` verb that reads them lands with the runtime." It becomes a
//!    statement that the runtime enforces them.
//! 2. **`docs/grammar.md` §15's `placements` paragraph**, which names this file
//!    as what holds the middle state honest.
//! 3. **The `targets` topic's row and its status paragraph** — "The rules above
//!    are enforced today; the protocol is not built yet." and the reserved-summary
//!    paragraph that explains why `placements` left the reserved list by being
//!    re-cut rather than by a runtime landing.
//! 4. **`docs/distributed.md` §12**, "What is built today", which is the whole
//!    section that stops being true.
//! 5. **The environment-manifest partition in
//!    `crates/compose-core/src/codegen/env.rs`** — the half with no sentence to
//!    bind. `References::of` walks the definitions, the trigger table and the
//!    deploy layer's `storage_backends:`/`event_sources:`, and deliberately does
//!    **not** walk `hub:`. Per PRD resolved q41 the runtime pass must teach it
//!    the per-placement partition of `docs/distributed.md` §9.1 — which the
//!    protocol leans on twice over, since the partition ships *in the artifact*
//!    and is what makes a join's `env_ok` computable at all (§3.1) — and the
//!    shape of that partition is the part worth reading before writing it: a
//!    process's manifest is the variables of every component that can **execute
//!    in it**, which is not the same as the components `members:` lists. A tool
//!    an agent attaches runs in that agent's process whether or not it names a
//!    placement (grammar 14.1 rule 4), and so does everything an attached
//!    `flow.*` reaches — so those variables belong to the *agent's* placement,
//!    and to the hub's as well wherever the component reached has a hub
//!    dispatch of its own, which every unplaced `agent.*` has and an unplaced
//!    `tool.*` has exactly when a `function:` node names it. A tool nothing
//!    unplaced reaches is the case that leaves the hub's list entirely.
//!    `hub.join_token:` and the rest of the deploy layer belong to the hub's.
//!    Until then a hub credential in `src/env.ts` would be a launch check for a
//!    value nothing reads.
//! 6. **`docs/distributed.md` §13, "What this document does not settle"** — not
//!    sentences to flip but work to do before the code they govern is written.
//!    A session's dispatch capacity and containment beyond the process boundary
//!    are open design questions the runtime pass carries to the PRD's Open
//!    Questions and implements against the resolution, never against the
//!    placeholder §13 records.

use std::fs;
use std::path::{Path, PathBuf};

use compose_core::{Diagnostic, resolve_with_target};

/// A composition with something to place: two agents, a tool one of them
/// attaches, and a tool reached from a `function:` node.
///
/// The `function:` tool is the case a placed tool exists for — grammar 14.1's
/// rule 4 leaves it alone, because nothing there disagrees with its placement —
/// so putting it in the mesh keeps this fixture on the legal side of the one
/// rule that spans the two files.
const SPEC: &str = r#"version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

tool.xcodebuild:
  description: Build and sign the macOS app.
  input:
    scheme: { type: string }
  output:
    log: { type: string }
  exec:
    command: xcodebuild
    args: ["-json"]

tool.notarize:
  description: Submit the signed build for notarization.
  input:
    path: { type: string }
  output:
    ticket: { type: string }
  exec:
    command: notarytool

agent.signer:
  model: model.smart
  prompt: Build the requested scheme and report what the log says.
  tools: [tool.xcodebuild]
  input:
    scheme: { type: string }
  output:
    verdict: { type: string }

agent.summarizer:
  model: model.smart
  prompt: Summarize the release for the changelog.
  input:
    verdict: { type: string }
  output:
    summary: { type: string }

flow.release:
  inputs:
    scheme: { type: string }
  outputs: {}
  nodes:
    sign:
      agent: agent.signer
      input:
        scheme: "input.scheme"
    stamp:
      function: tool.notarize
      input:
        path: "input.scheme"
    describe:
      agent: agent.summarizer
      input:
        verdict: "input.scheme"
  edges:
    - { from: start, to: sign }
    - { from: sign, to: stamp }
    - { from: stamp, to: describe }
    - { from: describe, to: end }
"#;

/// The deploy layer: every key of both new sections, written out.
const DEPLOY: &str = r#"version: "0.1"

hub:
  join_token: ${MESH_INERTNESS_TOKEN}
  public_url: "https://hub.inertness.example"

placements:
  mac_signing_pool:
    members:
      - agent.signer
      - tool.xcodebuild
      - tool.notarize
    description: The machine with the signing keys.
  gpu_pool:
    members: [agent.summarizer]
"#;

/// The strings that exist **only** because this composition declares a mesh.
///
/// Deliberately not the member addresses: `agent.signer` and `tool.xcodebuild`
/// are components, and a generated project is full of them whether or not
/// anything is placed. What is listed here is the material a placement or a hub
/// block contributes and nothing else does, so a hit is a protocol half-landing
/// rather than a coincidence.
const MESH_ONLY: &[&str] = &[
    "mac_signing_pool",
    "gpu_pool",
    "MESH_INERTNESS_TOKEN",
    "hub.inertness.example",
    "join_token",
    "joinToken",
    "/workers/join",
    "/workers/poll",
    "/workers/effects",
    "/workers/result",
    "/workers/artifact",
    "X-Worker-Session",
];

fn render(diagnostics: &[Diagnostic]) -> String {
    diagnostics
        .iter()
        .map(|diagnostic| {
            format!(
                "  {} at {}: {}",
                diagnostic.code, diagnostic.span, diagnostic.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The repository root.
fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// A scratch project of this test's own, cleaned out before use.
fn project(name: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-placement-inertness")
        .join(format!("{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("deploy")).expect("can create the project");
    fs::write(directory.join("main.yml"), SPEC).expect("can write the spec");
    fs::write(directory.join("deploy/mesh.yml"), DEPLOY).expect("can write the deploy file");
    directory
}

/// Resolve the fixture under `--target mesh`, asserting nothing was reported.
fn resolve_clean(name: &str) -> compose_core::Ir {
    let directory = project(name);
    let resolution = resolve_with_target(directory.join("main.yml"), "mesh");
    let ir = resolution
        .ir
        .expect("a composition with a legal deploy layer resolves");
    let mut diagnostics = resolution.diagnostics;
    diagnostics.extend(compose_core::check(&ir));
    assert!(
        diagnostics.is_empty(),
        "the fixture no longer resolves clean, so this file is measuring the wrong thing:\n{}",
        render(&diagnostics)
    );
    ir
}

/// The whole surface parses, resolves, and checks clean.
///
/// The assertion is exact: a diagnostic arriving here would mean a legal mesh
/// had become unwritable — every rule of grammar 14.1 and 14.2 is satisfied by
/// this fixture, including the one that spans the two files, since
/// `tool.xcodebuild` sits in the same placement as the agent attaching it and
/// `tool.notarize` is reached from a `function:` node.
#[test]
fn the_whole_placement_surface_parses_and_resolves_clean() {
    let _ = resolve_clean("clean");
}

/// …and it reaches the artifact, key for key.
///
/// Inert means "no runtime reads it", never "the compiler forgot it". A
/// placement dropped on the way to the IR would leave `plan` unable to report a
/// deployment change and would leave the runtime pass with nothing to build on,
/// and neither failure produces a diagnostic anybody would see.
#[test]
fn the_deploy_layer_carries_every_key_into_the_artifact() {
    let ir = resolve_clean("artifact");
    let hub = ir.deploy.hub.as_ref().expect("the target declares a hub");
    assert_eq!(
        hub.join_token
            .as_ref()
            .expect("the hub declares a join token")
            .value
            .name,
        "MESH_INERTNESS_TOKEN",
        "the join token reaches the artifact as the variable's *name* (grammar 4.3)"
    );
    assert_eq!(
        hub.public_url
            .as_ref()
            .expect("the hub declares a public base")
            .value,
        "https://hub.inertness.example"
    );

    let placements = ir
        .deploy
        .placements
        .as_ref()
        .expect("the target declares placements");
    let mac = placements
        .get("mac_signing_pool")
        .expect("`mac_signing_pool` is declared");
    let members: Vec<String> = mac
        .members
        .iter()
        .map(|member| member.value.to_string())
        .collect();
    assert_eq!(
        members,
        ["agent.signer", "tool.xcodebuild", "tool.notarize"],
        "a placement's members reach the artifact in the order they were written"
    );
    assert!(
        placements.get("gpu_pool").is_some(),
        "every placement reaches the artifact, not just the first"
    );
}

/// **None of it reaches the generated project.**
///
/// This is the assertion the runtime pass turns around. Every file of the
/// emitted project is scanned for the material only a mesh contributes — the
/// placement names, the token's variable, the public base, the five routes
/// of `docs/distributed.md` §3 and the session header they carry — and a hit
/// means half a protocol shipped.
///
/// Half is the dangerous amount. A build that emits nothing is a deployment
/// where no worker ever joins, which is visible the first time somebody starts
/// one; a build that emits a route and no scheduler is a mesh that accepts a
/// join and never dispatches, which looks like a hung graph.
#[test]
fn no_placement_or_hub_material_reaches_the_generated_project() {
    let ir = resolve_clean("emitted");
    let generated = compose_core::emit(&ir);
    for file in generated.files() {
        for material in MESH_ONLY {
            assert!(
                !file.contents.contains(material),
                "`{}` carries `{material}`, so part of the worker protocol has shipped ahead of \
                 the rest: turn this file around the way `trigger_auth_surface.rs` was turned \
                 around, following the unwind list in its module docs",
                file.path
            );
        }
    }
}

/// The environment manifest is the half with no sentence to bind.
///
/// `crates/compose-core/src/codegen/env.rs` walks the definitions, the trigger
/// table and the deploy layer's `storage_backends:`/`event_sources:`. It does
/// **not** walk `hub:`, so `${MESH_INERTNESS_TOKEN}` is absent from
/// `src/env.ts` — and that is correct exactly while nothing reads it: a launch
/// check that refused to start a project over a variable no code touches would
/// be a false requirement, and one that let a real mesh start without its token
/// would be the opposite failure.
///
/// The runtime pass has to move this assertion, not delete it. PRD resolved q41
/// makes the manifest **per placement**: the hub's own list is what
/// `readEnvironment()` checks at start, and a worker's list is what it reports
/// at join. `docs/distributed.md` §9.1 is the partition rule to implement.
#[test]
fn the_environment_manifest_does_not_yet_know_about_the_hub() {
    let ir = resolve_clean("environment");
    let generated = compose_core::emit(&ir);
    let environment = &generated
        .files()
        .iter()
        .find(|file| file.path == "src/env.ts")
        .expect("every project emits its environment manifest")
        .contents;

    assert!(
        environment.contains("OPENAI_API_KEY"),
        "the manifest still lists what it always listed, so this test is measuring the walk \
         rather than an empty file"
    );
    assert!(
        !environment.contains("MESH_INERTNESS_TOKEN"),
        "`src/env.ts` names the hub's join token, so the environment walk has learned about \
         `hub:` — which is the runtime pass's job, together with the per-placement partition of \
         `docs/distributed.md` §9.1 (PRD resolved q41)"
    );
}

/// Every shipped document says the runtime has not landed.
///
/// The sentences are located rather than the documents searched, because the
/// failure this catches is a document that quietly stops saying it: a reader
/// told the rules are enforced and not told the protocol is missing deploys a
/// mesh and waits.
#[test]
fn every_document_says_the_protocol_is_not_built_yet() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let distributed = fs::read_to_string(repository().join("docs/distributed.md"))
        .expect("the protocol document is readable");
    let targets = compose_core::docs::topic("targets").expect("the `targets` topic ships");

    for (document, claim) in [
        (
            grammar.as_str(),
            "the `worker` verb that reads them lands\nwith the runtime",
        ),
        (
            grammar.as_str(),
            "**`placements:` left it by being re-cut**",
        ),
        (
            distributed.as_str(),
            "**The static surface is live. The protocol is not built yet.**",
        ),
        (
            distributed.as_str(),
            "**Nothing a\nplacement or a `hub:` block declares reaches the project a build emits.**",
        ),
        (
            targets.body,
            "**The rules above are enforced today; the protocol is not built yet.**",
        ),
        (
            targets.body,
            "**`placements` left it by being re-cut, which is the one departure that is not a\nruntime landing.**",
        ),
    ] {
        assert!(
            document.contains(claim),
            "a document that states the awaiting-runtime posture no longer contains {claim:?} — \
             the sentence has to come back, or the runtime has to land and this whole file has to \
             be turned around"
        );
    }
}

/// No shipped document files `placements` among the **reserved** constructs.
///
/// The re-cut moved it off that list into a third posture — live static grammar
/// with a runtime still to come — and the two must not be confused. A reserved
/// construct may be re-shaped freely because nothing has run; a live one may
/// not, because `validate` already refuses compositions on its rules and a
/// re-shape would break specs somebody wrote. A document that filed it back
/// under "reserved" would be inviting the next re-cut.
#[test]
fn no_document_files_placements_as_reserved_grammar() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let targets = compose_core::docs::topic("targets").expect("the `targets` topic ships");

    for (name, document) in [
        ("docs/grammar.md", grammar.as_str()),
        ("the `targets` topic", targets.body),
    ] {
        for row in [
            "| `placements` | parsed + validated, no-op |",
            "| `network:` on a placement |",
        ] {
            assert!(
                !document.contains(row),
                "{name} still files `{row}` among the reserved constructs, and `placements:` has \
                 live static rules `validate` enforces"
            );
        }
    }
}

/// The grammar's normative index of reference positions agrees with the member
/// rule `validate` enforces.
///
/// §2.3 is the single place a reader — or an agent, which the `targets` topic
/// points at the grammar as its normative source — looks up what a position
/// accepts, and §14.1 rule 2 is what refuses a member. They are two statements of
/// one fact and drifted apart once already: the re-cut left §2.3 carrying a row
/// for the retired address-keyed shape, which named `placements` *keys* as a
/// reference position and offered `flow.*` among the namespaces it took. Both
/// halves of that row are now compile errors — `invalid-identifier` on the key
/// and `unsupported-placement` on the member — so a reader following it writes a
/// deploy file the compiler refuses twice over.
///
/// The rule is not "no document mentions `flow.*` near a placement": §14.1, D129
/// and the topic all say a `flow.*` member is refused, and saying so is the
/// point — the row this test guards says it too, in its notes. What is checked is
/// narrower and is the shape a reader scans rather than reads: no table row that
/// names a `placements` position may **offer** `flow.*` in the cell that lists
/// what the position accepts.
#[test]
fn the_reference_position_index_agrees_with_the_member_rule() {
    let grammar =
        fs::read_to_string(repository().join("docs/grammar.md")).expect("the grammar is readable");
    let targets = compose_core::docs::topic("targets").expect("the `targets` topic ships");

    assert!(
        grammar.contains("| `placements.<name>.members[]` | `agent.*`, `tool.*` |"),
        "§2.3's reference-position table no longer names `placements.<name>.members[]` and what \
         it accepts, so the grammar's normative index of reference positions has stopped covering \
         the one position the deploy layer has"
    );

    for (name, document) in [
        ("docs/grammar.md", grammar.as_str()),
        ("the `targets` topic", targets.body),
    ] {
        for row in document.lines().filter(|line| line.starts_with('|')) {
            let mut cells = row.split('|').skip(1);
            let (Some(position), Some(accepts)) = (cells.next(), cells.next()) else {
                continue;
            };
            if !position.contains("placements") {
                continue;
            }
            assert!(
                !accepts.contains("`flow.*`"),
                "{name} has a table row offering `flow.*` where a placement member goes, and \
                 §14.1 rule 2 refuses one: {row}"
            );
        }
    }
}

/// The protocol document is normative, and is reachable the way
/// `docs/durability.md` is.
///
/// It gets no embedded topic of its own — a normative wire contract is the wrong
/// shape for the example-led curriculum, which is the same call `durability.md`
/// carries — so the one thing that has to hold is that the topic covering the
/// deploy layer names it as a normative source. Without that line the document
/// is reachable only by knowing it exists.
#[test]
fn the_targets_topic_names_the_protocol_document_as_normative() {
    let targets = compose_core::docs::topic("targets").expect("the `targets` topic ships");
    let normative = targets
        .body
        .lines()
        .rev()
        .find(|line| line.starts_with("Normative source:"))
        .expect("the topic closes with its normative sources");
    for source in ["docs/durability.md", "docs/distributed.md"] {
        assert!(
            normative.contains(source),
            "the `targets` topic's normative sources omit `{source}`: {normative}"
        );
    }
    assert!(
        compose_core::docs::topics::names()
            .iter()
            .all(|name| *name != "distributed"),
        "`docs/distributed.md` gained a topic of its own; it is normative like \
         `docs/durability.md`, which has none either, and the curriculum is example-led"
    );
}
