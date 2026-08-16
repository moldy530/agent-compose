//! Shapes that are big rather than merely legal.
//!
//! `check_accepts.rs` proves the validator accepts what the grammar admits;
//! this file proves it still *answers* when what the grammar admits is large.
//! Neither a flow's node count, nor a node's out-degree, nor a composition's
//! nesting depth is bounded by anything the compiler controls, and each of the
//! three has its own failure mode:
//!
//! * **depth** — a traversal written with the call stack aborts the process on a
//!   stack overflow, which is not a diagnostic, not an exit code the CLI
//!   documents, and not something `--format json` can report;
//! * **out-degree** — a per-*pair* analysis of a fork is quadratic in the number
//!   of edges before it computes anything, so anything it recomputes per pair
//!   multiplies out. Two shapes are needed, because a fork has two costs: one
//!   fan of disjoint branches, where the pairs share no answers and what matters
//!   is that nothing is *walked* per pair, and one of overlapping branches, where
//!   the pairs share almost every answer and what matters is that nothing is
//!   *compared* or *crossed* per pair;
//! * **size** — the usual one, and the one the two worked projects already
//!   guard through `crates/agent-compose/tests/cli.rs`.
//!
//! Every case here is written so that a regression shows up as a failure rather
//! than as a slow test: the depth case runs on a thread whose stack is far too
//! small to recurse through, and the fork cases time the check pass alone — the
//! project is resolved off the clock — against a budget several times what they
//! cost and a fraction of what the shape costs a per-pair analysis. Each fork
//! case names both numbers, so a budget can be re-fitted from what it is for.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use compose_core::resolve;

/// The provider and model every case needs, a lister to fan out over, and the
/// one state channel that makes every `agent.a` node a **writer**.
///
/// The channel is what puts the concurrent-write half of `check/convergence.rs`
/// on the clock at all: that rule is answered over the nodes that write a
/// channel, `verdict` is where `agent.a`'s output field lands name-based
/// (grammar 8.0), and a project declaring no `state:` at all leaves every node
/// writing nothing — so the rule's inner loop never runs and a fork of any width
/// costs nothing to check. `reduce: last_wins` is what keeps the shapes below
/// *legal* while every one of their branches writes it (grammar 10.2, D32).
const BACKEND: &str = r#"provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise] }
agent.lister:
  model: model.m
  prompt: List them.
  output:
    items:
      type: array
      max_items: 10
      items: { type: string }
state:
  verdict:
    description: the verdict the last node to write reached
    enum: [approve, revise]
    reduce: last_wins
    default: approve
"#;

/// A scratch directory of this test's own, cleaned out before use.
fn scratch(name: &str) -> PathBuf {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("scale-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

/// `flow.f0` instantiates `flow.f1` instantiates … — one chain, `depth` deep,
/// spread over ten imported files, reached two ways at once so that both of
/// `check/reach.rs`'s traversals descend it:
///
/// * `flow.top` **dispatches** `flow.f0` from a `map`, which is what enumerates
///   dispatch sites (`frames`, grammar 11.4) — reached through
///   `stores::check_map_writes`, with no trigger involved;
/// * a `respond: sync` HTTP trigger names `flow.top`, which is what walks the
///   component relation (`reached`, grammar 7.7) — for `human` nodes
///   (grammar 13.3) and for session-scoped stores (grammar 11.3).
fn nested_project(dir: &Path, depth: usize) {
    let files = 10;
    let per = depth.div_ceil(files);
    let mut names = Vec::new();
    for (at, start) in (0..depth).step_by(per).enumerate() {
        let name = format!("flows{at}.yml");
        let mut text = String::new();
        for index in start..(start + per).min(depth) {
            // The dispatched flow is the one the map binds an item into; the
            // rest carry derivation inward with nothing to bind.
            let inputs = if index == 0 {
                "  inputs:\n    value: { type: string }\n"
            } else {
                ""
            };
            if index + 1 < depth {
                text.push_str(&format!(
                    "flow.f{index}:\n{inputs}  outputs: {{}}\n  nodes:\n    step: {{ flow: flow.f{} }}\n  edges:\n    - {{ from: start, to: step }}\n    - {{ from: step, to: end }}\n",
                    index + 1
                ));
            } else {
                text.push_str(&format!(
                    "flow.f{index}:\n{inputs}  outputs: {{}}\n  nodes:\n    leaf: {{ agent: agent.a, input: \"'x'\" }}\n  edges:\n    - {{ from: start, to: leaf }}\n    - {{ from: leaf, to: end }}\n"
                ));
            }
        }
        fs::write(dir.join(&name), text).expect("can write a flow file");
        names.push(name);
    }
    let imports: String = names
        .iter()
        .map(|name| format!("  - {name}\n"))
        .collect::<Vec<_>>()
        .concat();
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\nimports:\n{imports}{BACKEND}flow.top:\n  outputs: {{}}\n  nodes:\n    seed: {{ agent: agent.lister, input: \"'x'\" }}\n    fan:\n      map:\n        over: seed.output.items\n        as: item\n        max_concurrency: 4\n        node: flow.f0\n        input:\n          value: \"item\"\n  edges:\n    - {{ from: start, to: seed }}\n    - {{ from: seed, to: fan }}\n    - {{ from: fan, to: end }}\ntriggers:\n  on_request:\n    type: http\n    flow: flow.top\n    respond: sync\n"
        ),
    )
    .expect("can write the entrypoint");
}

/// A composition nested deeper than a call stack.
///
/// The whole pass runs on a thread with a 512 KiB stack — a recursion of one
/// frame per flow would need tens of megabytes at this depth, so a traversal
/// that recurses aborts the process here instead of returning a verdict. That
/// abort is the failure this pins: `validate` reported nothing at all on the
/// shape, and an exit code of 134 is not one the CLI's own table defines
/// (`crates/agent-compose/src/main.rs`).
#[test]
fn a_deeply_nested_composition_is_checked_without_recursing() {
    let dir = scratch("deep");
    nested_project(&dir, 10_000);
    let checked = std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            let resolution = resolve(dir.join("main.yml"));
            assert!(
                resolution.diagnostics.is_empty(),
                "the generated project resolves cleanly, got {} diagnostic(s): {:?}",
                resolution.diagnostics.len(),
                resolution.diagnostics.first().map(|d| d.message.clone())
            );
            let ir = resolution
                .ir
                .expect("a clean resolution produces an artifact");
            compose_core::check(&ir).len()
        })
        .expect("can spawn the walking thread")
        .join()
        .expect("the pass returns rather than overflowing its stack");
    assert_eq!(checked, 0, "the generated project checks cleanly");
}

/// One `hub` node with `width` unconditional out-edges, each starting a chain of
/// six nodes to `end`. Every pair of those edges is co-takeable, so the fork
/// analysis sees `width * (width - 1) / 2` pairs (grammar 7.6.1).
fn wide_fork_project(dir: &Path, width: usize) {
    let chain = 6;
    let mut nodes = String::from("    hub: { agent: agent.a, input: \"'h'\" }\n");
    let mut edges = String::from("    - { from: start, to: hub }\n");
    for branch in 0..width {
        for step in 0..chain {
            nodes.push_str(&format!(
                "    n{branch}_{step}: {{ agent: agent.a, input: \"'x'\" }}\n"
            ));
        }
        edges.push_str(&format!("    - {{ from: hub, to: n{branch}_0 }}\n"));
        for step in 0..chain - 1 {
            edges.push_str(&format!(
                "    - {{ from: n{branch}_{step}, to: n{branch}_{} }}\n",
                step + 1
            ));
        }
        edges.push_str(&format!(
            "    - {{ from: n{branch}_{}, to: end }}\n",
            chain - 1
        ));
    }
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{BACKEND}flow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// `layers` ranks of `wide` nodes, every node of one rank edging to every node of
/// the next. Every source is a fork whose branches **overlap** — each of its
/// out-edges reaches almost the whole graph below it — which is the half a fan of
/// disjoint chains does not reach: convergences at every depth for grammar
/// 7.6.2's distance comparison, and a cross product of writers per pair for
/// grammar 10.2's.
fn layered_project(dir: &Path, layers: usize, wide: usize) {
    let mut nodes = String::new();
    let mut edges = String::new();
    for rank in 0..layers {
        for at in 0..wide {
            nodes.push_str(&format!(
                "    n{rank}_{at}: {{ agent: agent.a, input: \"'x'\" }}\n"
            ));
        }
    }
    for at in 0..wide {
        edges.push_str(&format!("    - {{ from: start, to: n0_{at} }}\n"));
        edges.push_str(&format!(
            "    - {{ from: n{}_{at}, to: end }}\n",
            layers - 1
        ));
    }
    for rank in 0..layers - 1 {
        for from in 0..wide {
            for to in 0..wide {
                edges.push_str(&format!(
                    "    - {{ from: n{rank}_{from}, to: n{}_{to} }}\n",
                    rank + 1
                ));
            }
        }
    }
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{BACKEND}flow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// The minimum of three checks of one artifact, so a scheduling hiccup cannot
/// fail a test about an algorithm. Resolution is off the clock deliberately:
/// these are bounds on the graph analyses, not on the parser.
fn fastest_check(ir: &compose_core::Ir, what: &str) -> Duration {
    let fastest = (0..3)
        .map(|_| {
            let started = Instant::now();
            let diagnostics = compose_core::check(ir);
            assert!(
                diagnostics.is_empty(),
                "{what} checks cleanly, got {} diagnostic(s): {:?}",
                diagnostics.len(),
                diagnostics.first().map(|d| d.message.clone())
            );
            started.elapsed()
        })
        .min()
        .expect("three runs");
    println!("{what}: {fastest:?}");
    fastest
}

/// The artifact of a project that must resolve cleanly first.
fn artifact(dir: &Path) -> compose_core::Ir {
    let resolution = resolve(dir.join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "the generated project resolves cleanly, got {} diagnostic(s): {:?}",
        resolution.diagnostics.len(),
        resolution.diagnostics.first().map(|d| d.message.clone())
    );
    resolution
        .ir
        .expect("a clean resolution produces an artifact")
}

/// A fork wide enough that anything **walked** per pair shows.
///
/// 240 branches are 28,680 co-takeable pairs over 1,441 nodes, every one of them
/// a writer of the `verdict` channel. What the two rules read of an edge — its
/// step distances (grammar 7.6.2) and what it reaches (grammar 7.6.1) — is a
/// property of the node it delivers to, and every branch here leaves the hub for
/// a chain of its own, so no two pairs share an answer and 240 walks are the
/// fewest that can serve 28,680 pairs. Walking per pair instead made this shape
/// cubic in the out-degree and cost tens of seconds, against a command whose
/// budget is milliseconds (PRD 5.12).
#[test]
fn a_wide_fork_is_checked_in_proportion_to_its_pairs() {
    let dir = scratch("wide");
    wide_fork_project(&dir, 240);
    let budget = Duration::from_secs(6);
    let fastest = fastest_check(&artifact(&dir), "240-branch fork");
    assert!(
        fastest < budget,
        "checking a 240-branch fork took {fastest:?}, and the budget is {budget:?}"
    );
}

/// Overlapping branches, which is where anything **compared** per pair shows.
///
/// 5 ranks of 50 are 250 nodes and 10,100 edges, and each of the 201 forks —
/// `start` included — carries 1,225 co-takeable pairs, for 246,225 in all. The
/// two branches of any of them hold *almost the same nodes*, so a pair costs the
/// whole graph below it rather than a chain of six: the previous fan guards the
/// walks, and this one guards what is done with them. Both rules answer from the
/// two nodes a pair delivers to, of which there are 6,125 distinct combinations
/// here rather than 246,225 — comparing the two distance maps per pair instead
/// takes this shape from 1.4 s to 12.6 s, and crossing the two branches' writers
/// per pair from 1.4 s to minutes, on a composition with nothing wrong with it.
///
/// The `state:` channel of `BACKEND` is load-bearing: without a channel for
/// `agent.a`'s output field to land in, no node writes anything and grammar
/// 10.2's rule short-circuits before it looks at a single pair.
#[test]
fn overlapping_branches_are_checked_in_proportion_to_their_writers() {
    let dir = scratch("layered");
    layered_project(&dir, 5, 50);
    let budget = Duration::from_secs(6);
    let fastest = fastest_check(&artifact(&dir), "5x50 layered fan-out");
    assert!(
        fastest < budget,
        "checking a 5-by-50 layered fan-out took {fastest:?}, and the budget is {budget:?}"
    );
}
