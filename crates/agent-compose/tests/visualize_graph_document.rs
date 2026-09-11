//! What the graph document has to *say* about a real composition.
//!
//! `tests/visualize_cli.rs` owns the command and
//! `crates/compose-core/tests/graph_artifact_goldens.rs` owns the bytes. A
//! golden is a fence and not a claim: it would stay green over a document that
//! quietly stopped carrying the fan-out's variant narrowing, because the golden
//! would have been regenerated without it and the diff read as noise.
//!
//! So this file is the **semantic inventory**, asserted by name over
//! `examples/triage-fanout`. PRD resolved q56 states the content requirement —
//! "every 5.5 node kind renders distinguishably, and every routing decision an
//! execution could take … is a labeled edge, because the artifact's promise is
//! that what you see is every path the validator proved" — and every assertion
//! below is one clause of it, over a composition written to exercise the hard
//! cases: a discriminator-routed `map` with a narrowed `default:`, a node
//! reachable only through an `on_error:` fallback, a `human` node whose wait
//! transfers control, a store attached to an agent, and a subgraph.
//!
//! It runs the binary rather than the library, because what a consumer pins is
//! what `--format json` printed.

use std::path::{Path, PathBuf};

use assert_cmd::Command;
use serde_json::Value;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// The graph document of `examples/triage-fanout`, as the binary prints it.
fn document() -> Value {
    let output = Command::cargo_bin("agent-compose")
        .expect("the binary under test is built")
        .current_dir(repo_root())
        .env("NO_COLOR", "1")
        .args([
            "visualize",
            "examples/triage-fanout/main.yml",
            "--format",
            "json",
        ])
        .output()
        .expect("the command runs");
    assert_eq!(
        output.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("stdout is one JSON document")
}

/// One flow of that document, by address.
fn flow(document: &Value, address: &str) -> Value {
    document["flows"]
        .as_array()
        .expect("a flows array")
        .iter()
        .find(|flow| flow["address"] == address)
        .unwrap_or_else(|| panic!("the document carries `{address}`"))
        .clone()
}

/// One node of a flow, by canvas id.
fn node(flow: &Value, id: &str) -> Value {
    flow["nodes"]
        .as_array()
        .expect("a nodes array")
        .iter()
        .find(|node| node["id"] == id)
        .unwrap_or_else(|| panic!("the flow carries a node `{id}`"))
        .clone()
}

/// Every edge between two ids, whatever its class.
fn edges(flow: &Value, from: &str, to: &str) -> Vec<Value> {
    flow["edges"]
        .as_array()
        .expect("an edges array")
        .iter()
        .filter(|edge| edge["from"] == from && edge["to"] == to)
        .cloned()
        .collect()
}

/// Every §5.5 kind this composition reaches is on the canvas, told apart.
///
/// The grammar has eight and this project writes seven of them; a picture that
/// collapsed two — a tool node drawn like an agent, a store op drawn like an
/// `http` — would be a picture a reader cannot act on.
#[test]
fn every_node_kind_the_composition_uses_is_distinguishable() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    for (id, kind) in [
        ("start", "start"),
        ("end", "end"),
        ("enrich", "flow"),
        ("scan", "function"),
        ("classify", "agent"),
        ("dispatch", "map"),
        ("announce", "http"),
        ("announce_failed", "exec"),
        ("remember", "store"),
        ("approve", "human"),
    ] {
        assert_eq!(node(&triage, id)["kind"], kind, "the kind of `{id}`");
    }
}

/// The two `when:` edges leaving the `human` node carry their guards.
///
/// A conditional edge with no expression on it is a picture that says "control
/// may go either way" and stops — which is the picture PRD §2 says people are
/// stuck reading out of router functions today.
#[test]
fn every_conditional_edge_carries_the_cel_that_decides_it() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let approved = edges(&triage, "approve", "end");
    assert_eq!(approved.len(), 1);
    assert_eq!(approved[0]["class"], "conditional");
    assert_eq!(
        approved[0]["when"], "approve.output.decision == 'approve'",
        "the guard is carried as written"
    );
    assert_eq!(approved[0]["label"], approved[0]["when"]);

    let guarded: Vec<Value> = edges(&triage, "approve", "escalate")
        .into_iter()
        .filter(|edge| edge["class"] == "conditional")
        .collect();
    assert_eq!(guarded.len(), 1);
    assert_eq!(guarded[0]["when"], "approve.output.decision == 'reject'");
}

/// `announce`'s `on_error: { fallback: announce_failed }` is a labeled edge.
///
/// Grammar §7.8 counts that position alongside edges when it decides
/// reachability, which is what makes `announce_failed` — a node no `edges:`
/// entry targets — legal rather than an unreachable-node error. A canvas that
/// drew only the `edges:` list would show it floating, and the reader would
/// conclude the validator had let something through.
#[test]
fn the_on_error_fallback_is_an_edge_to_the_node_nothing_else_reaches() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let fallback = edges(&triage, "announce", "announce_failed");
    assert_eq!(fallback.len(), 1, "exactly one transfer, drawn once");
    assert_eq!(fallback[0]["class"], "error_fallback");
    assert_eq!(fallback[0]["label"], "on_error: fallback");

    assert!(
        triage["edges"]
            .as_array()
            .expect("an edges array")
            .iter()
            .filter(|edge| edge["to"] == "announce_failed")
            .all(|edge| edge["class"] == "error_fallback"),
        "nothing but the fallback reaches it"
    );
    assert_eq!(
        node(&triage, "announce")["policy"]["on_error"],
        serde_json::json!({
            "strategy": "fallback",
            "target": "announce_failed",
            "level": "node",
        }),
        "and the node's resolved policy says the same thing"
    );
}

/// `approve`'s `on_timeout:` is the other control-transfer position, also an
/// edge, and told apart from the `when:` edge that shares its endpoints.
#[test]
fn the_human_timeout_is_an_edge_beside_the_guarded_one_it_shares_a_target_with() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let both = edges(&triage, "approve", "escalate");
    assert_eq!(both.len(), 2, "a guarded route and a timeout transfer");
    let classes: Vec<&str> = both
        .iter()
        .map(|edge| edge["class"].as_str().expect("a class"))
        .collect();
    assert!(classes.contains(&"conditional") && classes.contains(&"timeout"));

    let timeout = both
        .iter()
        .find(|edge| edge["class"] == "timeout")
        .expect("the timeout transfer");
    assert_eq!(timeout["label"], "on_timeout (24h)");
    assert_eq!(
        node(&triage, "approve")["human"],
        serde_json::json!({ "timeout": "24h", "on_timeout": "escalate" })
    );
}

/// The fan-out: three routes, each an edge to its own satellite, and the
/// `default:` narrowed to the variants no named route claims.
///
/// The narrowing is the assertion that matters most. Grammar §8.6 rule 4 makes
/// `default:` see exactly the unrouted variants — which is why `finding.of`
/// type-checks on that route and nowhere else — and a picture that said only
/// "default" would be hiding the one thing a reader cannot derive from the
/// spec by looking at it.
#[test]
fn the_map_draws_every_route_and_narrows_the_default() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let map = node(&triage, "dispatch")["map"].clone();

    assert_eq!(map["over"], "classify.output.findings");
    assert_eq!(map["max_items"], 50, "the cardinality bound (Decision D10)");
    assert_eq!(map["max_concurrency"], 8, "the execution bound");
    assert_eq!(map["item_binding"], "finding");
    assert_eq!(map["dispatch"], "routed");
    assert_eq!(map["route_by"], "kind");
    assert_eq!(map["on_item_error"]["strategy"], "retry");
    assert_eq!(map["on_item_error"]["retry"]["max"], 2);

    let routes = map["routes"].as_array().expect("a routes array");
    assert_eq!(routes.len(), 3, "two named routes and the catch-all");
    assert_eq!(
        routes
            .iter()
            .map(|route| (
                route["node"].as_str().expect("a satellite id"),
                route["target"].as_str().expect("a target"),
                route["covers"]
                    .as_array()
                    .map(|held| held
                        .iter()
                        .map(|tag| tag.as_str().expect("a tag"))
                        .collect::<Vec<_>>())
                    .unwrap_or_default(),
            ))
            .collect::<Vec<_>>(),
        [
            ("dispatch/auto_fixable", "agent.fixer", vec!["auto_fixable"]),
            (
                "dispatch/needs_human",
                "tool.review_queue",
                vec!["needs_human"]
            ),
            ("dispatch/default", "tool.dead_letter", vec!["duplicate"]),
        ]
    );
    assert_eq!(
        routes[2]["default"], true,
        "the third route is the catch-all"
    );
    assert_eq!(routes[0]["max_concurrency"], 4, "a route's own bound");
    assert_eq!(
        routes[0]["writes"],
        serde_json::json!([{ "field": "patch", "channel": "patches", "reduce": "append" }]),
        "a write from inside a map names the reduce policy that makes it legal"
    );

    // One labeled edge per route, and each satellite is a node on the canvas.
    for route in routes {
        let id = route["node"].as_str().expect("a satellite id");
        let drawn = edges(&triage, "dispatch", id);
        assert_eq!(drawn.len(), 1, "`{id}` is dispatched by one edge");
        assert_eq!(drawn[0]["class"], "map_route");
        let expected = route["variant"].as_str().unwrap_or("default");
        assert_eq!(drawn[0]["label"], expected);
        assert_eq!(node(&triage, id)["dispatch"]["map"], "dispatch");
    }
}

/// The map's downstream edge is the join barrier, marked as one.
#[test]
fn the_maps_downstream_edge_is_the_join_barrier() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let barrier = edges(&triage, "dispatch", "verify");
    assert_eq!(barrier.len(), 1);
    assert_eq!(barrier[0]["join_barrier"], true);
    assert_eq!(
        barrier[0]["class"], "unconditional",
        "the barrier is a property of the edge, not a class of its own"
    );

    // And no other edge of this flow claims to be one.
    assert_eq!(
        triage["edges"]
            .as_array()
            .expect("an edges array")
            .iter()
            .filter(|edge| edge["join_barrier"] == true)
            .count(),
        1
    );
}

/// `agent.triage`'s wire carries the tool its attached store synthesized.
///
/// Nothing in the composition writes `docs_search` down: grammar §11.5
/// synthesizes it from `stores: [store.docs]`, and `agent_access: read`
/// withholds the write tool. It is exactly the kind of thing a reader cannot
/// see by reading the YAML, which is what a picture is for (PRD 5.8).
#[test]
fn an_attached_store_shows_up_as_the_tool_it_synthesizes() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let agent = node(&triage, "classify")["agent"].clone();

    assert_eq!(agent["address"], "agent.triage");
    assert_eq!(agent["stores"], serde_json::json!(["store.docs"]));
    let tools = agent["tools"].as_array().expect("a tools array");
    assert_eq!(
        tools.len(),
        1,
        "the store's read tool, and no write tool: {tools:?}"
    );
    assert_eq!(tools[0]["name"], "docs_search");
    assert_eq!(tools[0]["source"], "store");
    assert_eq!(tools[0]["address"], "store.docs");
    assert_eq!(tools[0]["op"], "search");
    assert!(
        tools[0]["detail"]
            .as_str()
            .expect("a detail line")
            .contains("agent_access: read"),
        "and it says why there is only one: {:?}",
        tools[0]["detail"]
    );
    assert!(
        agent["prompt"]
            .as_str()
            .expect("a prompt")
            .starts_with("You triage incoming bug reports."),
        "the prompt is carried verbatim"
    );
}

/// `classify`'s model is resolved through to the provider and its settings.
#[test]
fn an_agents_model_resolves_through_to_its_provider() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let model = node(&triage, "classify")["agent"]["model"].clone();
    assert_eq!(model["address"], "model.smart");
    assert_eq!(model["form"], "direct");
    assert_eq!(model["id"], "claude-sonnet-4-6");
    assert_eq!(model["provider"]["address"], "provider.anthropic");
    assert_eq!(model["provider"]["kind"], "anthropic");
    assert_eq!(
        model["settings"],
        serde_json::json!([{ "name": "max_tokens", "value": 8000 }])
    );
    // The credential survives unresolved, exactly as the IR carries it: a
    // picture is a file people share (PRD 5.9, resolved q15).
    assert_eq!(
        model["provider"]["config"],
        serde_json::json!([{ "name": "api_key", "value": "${ANTHROPIC_API_KEY}" }])
    );
}

/// A node's policy is resolved through grammar §9.3 with the level named.
#[test]
fn every_policy_says_which_level_of_the_chain_it_came_from() {
    let document = document();
    let triage = flow(&document, "flow.triage");

    // Level 3: the composition's `defaults:`, which this node declares nothing
    // against.
    assert_eq!(
        node(&triage, "classify")["policy"],
        serde_json::json!({
            "retry": { "max": 1, "backoff": "2s", "level": "defaults" },
            "timeout": { "value": "90s", "level": "defaults" },
            "on_error": { "strategy": "fail", "level": "defaults" },
        })
    );
    // Level 2 for two of the three, level 3 for the other.
    let verify = node(&triage, "verify")["policy"].clone();
    assert_eq!(
        verify["timeout"],
        serde_json::json!({ "value": "5m", "level": "node" })
    );
    assert_eq!(verify["retry"]["level"], "defaults");
    assert_eq!(
        verify["on_error"],
        serde_json::json!({ "strategy": "skip", "level": "node" })
    );
    // A `human` node resolves no timeout and no retry at any level (D102).
    let approve = node(&triage, "approve")["policy"].clone();
    assert_eq!(approve.get("timeout"), None, "withheld, not defaulted");
    assert_eq!(approve.get("retry"), None);
    assert_eq!(approve["on_error"]["strategy"], "fail");
}

/// A subgraph is a jump target with its bindings at the boundary, never an
/// inline expansion (PRD 5.7, resolved q56).
#[test]
fn a_subgraph_node_links_to_its_own_canvas() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let enrich = node(&triage, "enrich");

    assert_eq!(enrich["kind"], "flow");
    assert_eq!(enrich["subflow"]["address"], "flow.enrich");
    assert_eq!(enrich["subflow"]["context"], "isolated");
    // Level 1 of grammar §9.3, reported where it is written rather than folded
    // into the nodes it governs.
    assert_eq!(
        enrich["subflow"]["policy"],
        serde_json::json!({ "timeout": "30s", "on_error": "fail" })
    );
    assert_eq!(
        enrich["input"],
        serde_json::json!({
            "form": "fields",
            "bindings": [{ "name": "report", "expression": "input.report" }],
        }),
        "what crosses the boundary is drawn at the boundary"
    );

    // The canvas it links to is in the same document, and holds its own nodes
    // rather than appearing inside this flow's.
    let inner = flow(&document, "flow.enrich");
    assert_eq!(
        inner["nodes"]
            .as_array()
            .expect("a nodes array")
            .iter()
            .map(|node| node["id"].as_str().expect("an id"))
            .collect::<Vec<_>>(),
        ["start", "end", "fetch"]
    );
    assert!(
        !triage["nodes"]
            .as_array()
            .expect("a nodes array")
            .iter()
            .any(|node| node["id"] == "fetch"),
        "nothing of the subflow leaks onto the caller's canvas"
    );
}

/// Every trigger targeting a flow is listed on it, and only those.
#[test]
fn a_flow_lists_the_triggers_that_target_it() {
    let document = document();
    let triage = flow(&document, "flow.triage");
    let triggers = triage["triggers"].as_array().expect("a triggers array");
    assert_eq!(
        triggers
            .iter()
            .map(|trigger| (
                trigger["name"].as_str().expect("a name"),
                trigger["type"].as_str().expect("a type")
            ))
            .collect::<Vec<_>>(),
        [
            ("cli", "manual"),
            ("ingest", "event"),
            ("nightly", "schedule"),
            ("on_report", "http"),
        ],
        "in trigger-name order"
    );
    assert_eq!(
        triggers[3]["summary"], "POST /reports · respond async",
        "an http trigger says how an execution arrives"
    );
    assert_eq!(triggers[0]["session_key"], "payload.session");
    assert!(
        flow(&document, "flow.enrich")["triggers"]
            .as_array()
            .expect("a triggers array")
            .is_empty(),
        "a flow nothing targets lists nothing"
    );
}

/// Every edge names two nodes the same flow declares.
///
/// The invariant §9.1 of `docs/graph.md` gives a reader: an edge whose endpoint
/// is not on the canvas is an edge a renderer has to drop, and a dropped edge
/// is a path the picture stopped promising.
#[test]
fn every_edge_endpoint_is_a_node_of_its_own_flow() {
    let document = document();
    for flow in document["flows"].as_array().expect("a flows array") {
        let ids: Vec<&str> = flow["nodes"]
            .as_array()
            .expect("a nodes array")
            .iter()
            .map(|node| node["id"].as_str().expect("an id"))
            .collect();
        for edge in flow["edges"].as_array().expect("an edges array") {
            for end in ["from", "to"] {
                let held = edge[end].as_str().expect("an endpoint");
                assert!(
                    ids.contains(&held),
                    "`{}` names `{held}`, which is not a node of `{}`",
                    edge,
                    flow["address"]
                );
            }
        }
    }
}
