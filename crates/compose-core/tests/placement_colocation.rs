//! Which process runs a placed component, in both directions (grammar 14.1
//! rule 4, Decision D129, `docs/distributed.md` §9.1).
//!
//! The colocation rule has a truth table, and a negative corpus can only pin the
//! rows that refuse. The rows that *accept* are the ones a one-token slip turns
//! into an over-rejection nothing else notices — and the rows that refuse are
//! the ones a one-token slip turns into silence, which is worse: a placement
//! written, accepted, and ignored by the deployment. This file asserts the whole
//! table, both halves, plus the transitive case a `flow.*` in a `tools:` list
//! opens.
//!
//! `tests/fixtures/invalid-check/` pins the exact rendering of the two refusals
//! an author is likeliest to meet. What is here is the table itself: every
//! combination, each named for the sentence it holds true.

use std::fs;
use std::path::PathBuf;

use compose_core::{Diagnostic, resolve_with_target};

/// The provider and model every case needs, and nothing more.
const BACKEND: &str = r#"version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
tool.xcodebuild:
  description: Build and sign the macOS app.
  input:
    scheme: { type: string }
  output:
    log: { type: string }
  exec:
    command: xcodebuild
"#;

/// An agent attaching whatever a case wants attached, and none if it wants none.
fn agent(name: &str, attaches: &str) -> String {
    let tools = if attaches.is_empty() {
        String::new()
    } else {
        format!("  tools: [{attaches}]\n")
    };
    format!(
        r#"agent.{name}:
  model: model.m
  prompt: Do the work.
{tools}  input:
    scheme: {{ type: string }}
  output:
    verdict: {{ type: string }}
"#
    )
}

/// A flow that reaches `tool.xcodebuild` from a `function:` node, usable both as
/// an agent's tool and as a `flow:` node.
const SIGNING_FLOW: &str = r#"flow.sign:
  description: Build and sign the requested scheme.
  inputs:
    scheme: { type: string }
  outputs: {}
  nodes:
    build:
      function: tool.xcodebuild
      input:
        scheme: "input.scheme"
  edges:
    - { from: start, to: build }
    - { from: build, to: end }
"#;

/// A scratch project of this test's own, cleaned out before use.
fn project(name: &str, spec: &str, deploy: &str) -> PathBuf {
    let directory = std::env::temp_dir()
        .join("agent-compose-placement-colocation")
        .join(format!("{}-{name}", std::process::id()));
    let _ = fs::remove_dir_all(&directory);
    fs::create_dir_all(directory.join("deploy")).expect("can create the project");
    fs::write(directory.join("main.yml"), spec).expect("can write the spec");
    fs::write(directory.join("deploy/mesh.yml"), deploy).expect("can write the deploy file");
    directory
}

/// Resolve and check one case under `--target mesh`.
///
/// A resolver diagnostic is this harness failing rather than the rule firing:
/// every case is a composition that resolves, so an undefined member or a
/// malformed deploy file would make the assertion below measure the wrong pass.
#[track_caller]
fn diagnose(name: &str, spec: &str, deploy: &str) -> Vec<Diagnostic> {
    let directory = project(name, spec, deploy);
    let resolution = resolve_with_target(directory.join("main.yml"), "mesh");
    assert!(
        resolution.diagnostics.is_empty(),
        "`{name}` is meant to resolve; the harness is measuring the wrong pass:\n{}",
        render(&resolution.diagnostics)
    );
    let ir = resolution
        .ir
        .expect("a clean resolution produces an artifact");
    compose_core::check(&ir)
}

#[track_caller]
fn accepts(name: &str, spec: &str, deploy: &str) {
    let diagnostics = diagnose(name, spec, deploy);
    assert!(
        diagnostics.is_empty(),
        "`{name}` is a legal placement and the checks refused it:\n{}",
        render(&diagnostics)
    );
}

#[track_caller]
fn refuses(name: &str, spec: &str, deploy: &str, message: &str) {
    let diagnostics = diagnose(name, spec, deploy);
    let messages: Vec<&str> = diagnostics
        .iter()
        .map(|diagnostic| diagnostic.message.as_str())
        .collect();
    assert_eq!(
        messages,
        [message],
        "`{name}` did not produce exactly the refusal it is written for"
    );
    assert_eq!(
        diagnostics[0].code.to_string(),
        "conflicting-placement",
        "`{name}` was refused under the wrong code"
    );
}

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

/// A deploy file with a hub and the placements a case declares.
fn mesh(placements: &str) -> String {
    format!(
        r#"version: "0.1"

hub:
  join_token: ${{MESH_JOIN_TOKEN}}

placements:
{placements}"#
    )
}

// ---------------------------------------------------------------------------
// The direct table: an agent and the `tool.*` it attaches.
// ---------------------------------------------------------------------------

/// Row 1: a placed agent attaching a tool that claims nothing.
///
/// The row the negative corpus can say nothing about, and the one the whole
/// design rests on: the artifact is everywhere (PRD resolved q40), so a tool
/// with no placement runs wherever the agent that called it runs. A rule written
/// one token too tight here — `is_none_or` where `is_some_and` belongs, say —
/// makes the ordinary mesh unwritable and no other test in the repository
/// notices.
#[test]
fn a_placed_agent_may_attach_a_tool_that_claims_nothing() {
    accepts(
        "row-1",
        &format!("{BACKEND}{}", agent("builder", "tool.xcodebuild")),
        &mesh("  mac:\n    members: [agent.builder]\n"),
    );
}

/// Row 2: the same answer written twice.
#[test]
fn an_agent_and_its_attached_tool_may_share_one_placement() {
    accepts(
        "row-2",
        &format!("{BACKEND}{}", agent("builder", "tool.xcodebuild")),
        &mesh("  mac:\n    members: [agent.builder, tool.xcodebuild]\n"),
    );
}

/// Row 3: two placements, two answers.
#[test]
fn an_attached_tool_placed_elsewhere_is_refused() {
    refuses(
        "row-3",
        &format!("{BACKEND}{}", agent("builder", "tool.xcodebuild")),
        &mesh("  mac:\n    members: [tool.xcodebuild]\n  gpu:\n    members: [agent.builder]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.builder` that attaches it \
         has placement `gpu`",
    );
}

/// Row 4: the placement an author is likeliest to write and a deployment would
/// silently ignore.
#[test]
fn a_placed_tool_attached_to_an_unplaced_agent_is_refused() {
    refuses(
        "row-4",
        &format!("{BACKEND}{}", agent("builder", "tool.xcodebuild")),
        &mesh("  mac:\n    members: [tool.xcodebuild]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.builder` that attaches it \
         has no placement, so it runs on the hub",
    );
}

/// The case a placed tool exists for: reached by the graph, not by a model.
#[test]
fn a_placed_tool_reached_from_a_function_node_is_untouched() {
    let spec = format!(
        r#"{BACKEND}flow.release:
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    build:
      function: tool.xcodebuild
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: build }}
    - {{ from: build, to: end }}
"#
    );
    accepts(
        "function-node",
        &spec,
        &mesh("  mac:\n    members: [tool.xcodebuild]\n"),
    );
}

// ---------------------------------------------------------------------------
// The transitive table: an agent and the `flow.*` it attaches.
// ---------------------------------------------------------------------------

/// A flow attached as a tool runs its instance in the agent's own process
/// (grammar 5.4), so a placement it reaches is a placement the agent's worker
/// would have to honour and cannot.
#[test]
fn a_placement_reached_only_through_an_attached_flow_is_refused() {
    refuses(
        "transitive-hub",
        &format!("{BACKEND}{SIGNING_FLOW}{}", agent("reviewer", "flow.sign")),
        &mesh("  mac:\n    members: [tool.xcodebuild]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.reviewer` that reaches it \
         through the attached `flow.sign` has no placement, so it runs on the hub",
    );
}

/// …and the disagreement is a disagreement whether or not the agent is placed.
#[test]
fn an_attached_flow_reaching_another_placement_is_refused() {
    refuses(
        "transitive-mismatch",
        &format!("{BACKEND}{SIGNING_FLOW}{}", agent("reviewer", "flow.sign")),
        &mesh("  mac:\n    members: [tool.xcodebuild]\n  gpu:\n    members: [agent.reviewer]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.reviewer` that reaches it \
         through the attached `flow.sign` has placement `gpu`",
    );
}

/// The agreement is legal, exactly as it is one indirection in.
#[test]
fn an_attached_flow_may_reach_the_agents_own_placement() {
    accepts(
        "transitive-agreeing",
        &format!("{BACKEND}{SIGNING_FLOW}{}", agent("reviewer", "flow.sign")),
        &mesh("  mac:\n    members: [agent.reviewer, tool.xcodebuild]\n"),
    );
}

/// …and so is an attached flow that reaches nothing placed at all.
#[test]
fn an_attached_flow_reaching_nothing_placed_is_untouched() {
    accepts(
        "transitive-unplaced",
        &format!("{BACKEND}{SIGNING_FLOW}{}", agent("reviewer", "flow.sign")),
        &mesh("  gpu:\n    members: [agent.reviewer]\n"),
    );
}

/// The nesting is followed: a flow reached from an attached flow runs in the
/// same process, and the walk does not stop at the first hop.
#[test]
fn a_placement_reached_through_a_nested_flow_is_refused() {
    let spec = format!(
        r#"{BACKEND}{SIGNING_FLOW}flow.release:
  description: Release the requested scheme.
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    signing:
      flow: flow.sign
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: signing }}
    - {{ from: signing, to: end }}
{}"#,
        agent("reviewer", "flow.release")
    );
    refuses(
        "transitive-nested",
        &spec,
        &mesh("  mac:\n    members: [tool.xcodebuild]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.reviewer` that reaches it \
         through the attached `flow.release` has no placement, so it runs on the hub",
    );
}

/// A placed **agent** inside an attached flow is refused for the same reason a
/// placed tool is: the instance runs in the attaching agent's process, and an
/// agent node inside it never reaches the hub's scheduler.
#[test]
fn a_placed_agent_reached_through_an_attached_flow_is_refused() {
    let spec = format!(
        r#"{BACKEND}{}flow.review:
  description: Review the requested scheme.
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    sign:
      agent: agent.signer
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: sign }}
    - {{ from: sign, to: end }}
{}"#,
        agent("signer", "tool.xcodebuild"),
        agent("reviewer", "flow.review")
    );
    refuses(
        "transitive-agent",
        &spec,
        &mesh("  mac:\n    members: [agent.signer, tool.xcodebuild]\n"),
        "`agent.signer` is a member of placement `mac`, and `agent.reviewer` that reaches it \
         through the attached `flow.review` has no placement, so it runs on the hub",
    );
}

/// An agent reached **only** through an attached flow is still hub-resident.
///
/// This is the shape that reads as a counterexample to the direct table's last
/// row and is not one. `agent.outer` is placed; it attaches `flow.review`, whose
/// `agent:` node names `agent.inner`; `agent.inner` attaches the placed
/// `tool.xcodebuild` and is named nowhere else. Every call *through
/// `agent.outer`* does run all three on a `mac` worker, which is what makes the
/// composition look coherent.
///
/// It is refused because manual invocation is universal: `agent-compose run
/// <spec> flow.review` starts that flow directly — the emitted registry is every
/// flow a composition declares, not every triggered one (PRD 5.11, Decision
/// D64) — and the hub then dispatches `agent.inner` itself, calling
/// `tool.xcodebuild` on the hub, where the signing keys are not. Reading the
/// attaching agent's *reachable* hosts instead of its own placement would accept
/// this and leave that execution silently misplaced.
///
/// The row-4 message is quoted here as it renders, because the claim it makes —
/// "`agent.inner` … has no placement, so it runs on the hub" — is the sentence
/// this case exists to hold true.
#[test]
fn an_agent_reached_only_through_an_attached_flow_is_still_hub_resident() {
    let spec = format!(
        r#"{BACKEND}{}flow.review:
  description: Review the requested scheme.
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    sign:
      agent: agent.inner
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: sign }}
    - {{ from: sign, to: end }}
{}"#,
        agent("inner", "tool.xcodebuild"),
        agent("outer", "flow.review")
    );
    refuses(
        "attached-flow-inner-agent",
        &spec,
        &mesh("  mac:\n    members: [agent.outer, tool.xcodebuild]\n"),
        "`tool.xcodebuild` is a member of placement `mac`, and `agent.inner` that attaches it \
         has no placement, so it runs on the hub",
    );
}

/// …and naming it in the placement is the repair, which has to keep working.
///
/// The refusal above is only worth having if the composition an author writes
/// next validates: three components in one placement, the whole chain on one
/// worker however it is entered — through `agent.outer`'s tool loop, or by the
/// hub running `flow.review` on its own.
#[test]
fn placing_that_inner_agent_too_is_the_repair() {
    let spec = format!(
        r#"{BACKEND}{}flow.review:
  description: Review the requested scheme.
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    sign:
      agent: agent.inner
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: sign }}
    - {{ from: sign, to: end }}
{}"#,
        agent("inner", "tool.xcodebuild"),
        agent("outer", "flow.review")
    );
    accepts(
        "attached-flow-inner-agent-placed",
        &spec,
        &mesh("  mac:\n    members: [agent.outer, agent.inner, tool.xcodebuild]\n"),
    );
}

/// The boundary: a `flow:` **node** is not an attachment.
///
/// The hub schedules the nodes of a flow instantiated by a `flow:` node, so
/// every placement inside it is honoured and there is nothing to refuse. Reading
/// the transitive rule as "a flow drags its contents into whatever calls it"
/// would make the ordinary distributed graph unwritable, which is the failure
/// this case exists to catch.
#[test]
fn a_flow_reached_from_a_flow_node_keeps_its_own_placements() {
    let spec = format!(
        r#"{BACKEND}{SIGNING_FLOW}flow.release:
  inputs:
    scheme: {{ type: string }}
  outputs: {{}}
  nodes:
    signing:
      flow: flow.sign
      input:
        scheme: "input.scheme"
    review:
      agent: agent.reviewer
      input:
        scheme: "input.scheme"
  edges:
    - {{ from: start, to: signing }}
    - {{ from: signing, to: review }}
    - {{ from: review, to: end }}
{}"#,
        agent("reviewer", "")
    );
    accepts(
        "flow-node",
        &spec,
        &mesh("  mac:\n    members: [tool.xcodebuild]\n"),
    );
}
