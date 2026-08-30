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
//!
//! # Where the unwind has got to
//!
//! **The hub half has landed and the worker half has not.** `src/mesh.ts`,
//! `src/deployment.ts` and `src/artifact.ts` are emitted, a placed node is
//! dispatch-and-await, and the environment manifest is partitioned per
//! `docs/distributed.md` §9.1 — so items 5 and the *emission* half of this
//! file's second claim are done, and the two tests that asserted them have been
//! turned around below rather than deleted. What has **not** landed is the
//! `worker` verb, its node-runner, and the multi-process acceptance suite; the
//! documents therefore still say the protocol is not built, item 4's §12 is
//! still true of the half a reader can deploy, and the doc-pinning tests below
//! are untouched. The pass that lands the worker turns *those* around and
//! finishes this file the way `trigger_auth_surface.rs` was finished.

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

/// **The mesh reaches the generated project, and reaches the right files.**
///
/// This is the assertion the hub pass turned around, and the half of the
/// turnaround it could do: the material only a mesh contributes — the placement
/// names, the token's variable, the public base, the four routes of
/// `docs/distributed.md` §3 the hub mounts and the session header they carry —
/// is no longer absent, and what replaces "it is absent" is **where each piece
/// belongs**.
///
/// Half is still the dangerous amount, which is why the check is a partition
/// rather than a presence: the *composition's* mesh facts belong to
/// `src/deployment.ts`, which is the one module the hub and a worker both read
/// (§9.1), and the *protocol's* belong to `src/mesh.ts`, which is a constant of
/// the compiler release. A route that appeared in the emitted graph, or a
/// placement name compiled into the hub, would be a wire whose shape depended on
/// the composition — and §10.1's "the routes of §3, their paths and their
/// methods" is a promise about the release rather than about the deployment.
#[test]
fn the_mesh_reaches_the_generated_project_in_the_modules_that_hold_it() {
    let ir = resolve_clean("emitted");
    let generated = compose_core::emit(&ir);
    let file = |path: &str| {
        generated
            .file(path)
            .unwrap_or_else(|| panic!("a mesh project emits `{path}`"))
            .contents
            .as_str()
    };

    // The deploy layer's own facts, in the module that carries the partition.
    let deployment = file("src/deployment.ts");
    for material in [
        "mac_signing_pool",
        "gpu_pool",
        "MESH_INERTNESS_TOKEN",
        "hub.inertness.example",
    ] {
        assert!(
            deployment.contains(material),
            "`src/deployment.ts` does not carry `{material}`, so what the hub and a worker read \
             one answer out of has stopped describing this target (docs/distributed.md §9.1)"
        );
    }

    // …and the protocol's, in the constant that speaks it.
    let mesh = file("src/mesh.ts");
    for route in [
        "/workers/join",
        "/workers/poll",
        "/workers/effects",
        "/workers/result",
        "/workers/artifact",
    ] {
        assert!(
            mesh.contains(route),
            "`src/mesh.ts` mounts no `{route}`, and §3's five routes are what a worker speaks to"
        );
    }
    assert!(
        mesh.contains("x-worker-session"),
        "`src/mesh.ts` reads no session header, and every route after the join carries one (§3)"
    );

    // The composition's own modules stay clear of both: a placement decides
    // which process runs a node, and the node is compiled once.
    for path in [
        "src/graph.ts",
        "src/state.ts",
        "src/schemas.ts",
        "src/env.ts",
    ] {
        for route in ["/workers/join", "/workers/poll", "/workers/result"] {
            assert!(
                !file(path).contains(route),
                "`{path}` carries `{route}`, so the wire has leaked into the composition's own \
                 lowering: §10.1 promises the routes to a compiler release, not to a deployment"
            );
        }
    }
    assert!(
        !file("src/mesh.ts").contains("mac_signing_pool"),
        "`src/mesh.ts` names a placement of this composition, and it is a constant of the \
         compiler release: what differs between two deployments is `src/deployment.ts`"
    );

    // And the seam itself: a placed component's node is a dispatch rather than a
    // call, which is the whole of what `docs/distributed.md` §7 asks of the hub.
    let graph = file("src/graph.ts");
    assert!(
        graph.contains("mesh.dispatchPlaced({"),
        "no node of this composition dispatches, and `agent.signer` is placed on \
         `mac_signing_pool`: the generated node function for a placed component is \
         dispatch-and-await (docs/distributed.md §7)"
    );
    assert!(
        graph.contains("placement: \"mac_signing_pool\","),
        "the dispatch names no placement, so the hub would have nothing to queue the work to \
         (docs/distributed.md §2: work queues to a placement, never to a worker)"
    );
    // The seam is per component: the fixture's second placement gets a dispatch
    // of its own, and the `function:` node over the placed `tool.notarize` gets
    // one too — grammar §14.1 leaves a `function:` node's tool alone precisely
    // because its own placement is the whole answer there.
    assert!(
        graph.contains("placement: \"gpu_pool\","),
        "`agent.summarizer` is placed on `gpu_pool` and its node does not dispatch: a placement \
         decides which process runs a node, one node at a time"
    );
    assert_eq!(
        graph.matches("mesh.dispatchPlaced({").count(),
        3,
        "this fixture has three placed nodes — the two agents and the `function:` node over \
         `tool.notarize` — and each is one dispatch"
    );
}

/// The material a mesh contributes reaches the project, and nothing else does.
///
/// The counterpart of the test above, kept as a scan because it is the one that
/// catches a *leak*: a composition with no `placements:` must emit a project in
/// which nothing about a mesh appears except the constant hub module, which
/// mounts nothing for it.
#[test]
fn a_composition_with_no_placements_carries_no_mesh_of_its_own() {
    let directory = std::env::temp_dir()
        .join("agent-compose-placement-inertness")
        .join(format!("{}-unplaced", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(&directory).expect("can create the project");
    fs::write(directory.join("main.yml"), SPEC).expect("can write the spec");
    let resolution = compose_core::resolve(directory.join("main.yml"));
    let ir = resolution.ir.expect("the composition alone resolves");
    assert!(compose_core::check(&ir).is_empty());

    let generated = compose_core::emit(&ir);
    // The composition's own lowering, which is where a leak would show: the
    // constants (`src/mesh.ts`) and the identity module describe the wire in
    // their prose, and describing it is what they are for.
    for path in [
        "src/graph.ts",
        "src/env.ts",
        "src/state.ts",
        "src/schemas.ts",
        "src/triggers.ts",
    ] {
        let contents = &generated
            .file(path)
            .unwrap_or_else(|| panic!("every project emits `{path}`"))
            .contents;
        for material in MESH_ONLY {
            assert!(
                !contents.contains(material),
                "`{path}` carries `{material}` for a composition that places nothing"
            );
        }
        assert!(
            !contents.contains("dispatchPlaced"),
            "`{path}` dispatches a node for a composition that places none"
        );
    }
    let deployment = &generated
        .file("src/deployment.ts")
        .expect("every project emits its deployment module")
        .contents;
    assert!(
        deployment.contains("export const placements: readonly PlacementManifest[] = [];"),
        "a composition with no placements declares none: {deployment}"
    );
    assert!(
        deployment.contains("export const joinTokenEnv: string | undefined = undefined;"),
        "a composition with no `hub:` names no join token: {deployment}"
    );
}

/// The environment manifest is partitioned per process (PRD resolved q41).
///
/// The assertion this file used to make — that `src/env.ts` did not know about
/// `hub:` — was correct exactly while nothing read the token. The hub reads it
/// now, so the assertion **moved** rather than went: `readEnvironment()` checks
/// the *hub's* list, which is what `docs/distributed.md` §9.1 partitions, and
/// `src/deployment.ts` carries each placement's beside it so both ends read one
/// answer under one artifact hash.
///
/// Both directions §9.1 closes with are here, over this fixture's own shape.
/// Every agent it declares is placed, so `${OPENAI_API_KEY}` — the credential
/// their model's provider carries — is spent on the two workers and **not** on
/// the hub, which is the least-privilege line PRD 5.10 draws: the hub cannot
/// leak what it never held. The join token is the hub's alone, by the same rule
/// read the other way.
#[test]
fn the_environment_manifest_is_partitioned_per_process() {
    let ir = resolve_clean("environment");
    let generated = compose_core::emit(&ir);
    let contents = |path: &str| {
        generated
            .file(path)
            .unwrap_or_else(|| panic!("every project emits `{path}`"))
            .contents
            .as_str()
    };
    let environment = contents("src/env.ts");
    let deployment = contents("src/deployment.ts");

    assert!(
        !environment.contains("OPENAI_API_KEY"),
        "`src/env.ts` demands the provider credential of two placed agents, and the hub runs \
         neither: a launch check over a value nothing in this process reads is the false \
         requirement §9.1 is written against — {environment}"
    );
    assert_eq!(
        deployment.matches("\"OPENAI_API_KEY\",").count(),
        2,
        "the credential belongs to both placements' manifests, because both run an agent that \
         spends it (docs/distributed.md §9.1): {deployment}"
    );
    assert!(
        environment.contains("MESH_INERTNESS_TOKEN"),
        "`src/env.ts` does not name the hub's join token, and the hub verifies every join against \
         it: a deployment missing it would start clean and refuse every worker (grammar §14.2, \
         `docs/distributed.md` §3)"
    );
    assert!(
        environment.contains("deploy.hub.join_token"),
        "the token reaches the manifest without the surface that wrote it: {environment}"
    );
    for placement in ["mac_signing_pool", "gpu_pool"] {
        assert!(
            deployment.contains(&format!("name: \"{placement}\",")),
            "`src/deployment.ts` carries no manifest for `{placement}`, so a worker claiming it \
             has no list to report `env_ok` against (docs/distributed.md §3.1, §9.1)"
        );
    }
    // Named once in this module, as `joinTokenEnv` — the variable the hub reads
    // its own credential from — and in **no placement's manifest**: §9.1 puts a
    // deploy-layer variable on the hub's list alone, and a worker reporting the
    // mesh's own token would be reporting a credential it has no business
    // holding (§9.3).
    let placements = deployment
        .split_once("export const hubEnvironment")
        .expect("the module declares the hub's list after the placements")
        .0;
    assert!(
        !placements.contains("MESH_INERTNESS_TOKEN"),
        "a placement's manifest names the hub's join token: {placements}"
    );
    assert!(
        deployment
            .contains("export const joinTokenEnv: string | undefined = \"MESH_INERTNESS_TOKEN\";"),
        "the module does not name the variable the hub reads its join token from: {deployment}"
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
