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
//!   multiplies out;
//! * **size** — the usual one, and the one the two worked projects already
//!   guard through `crates/agent-compose/tests/cli.rs`.
//!
//! Every case here is written so that a regression shows up as a failure rather
//! than as a slow test: the depth case runs on a thread whose stack is far too
//! small to recurse through, and the fork case times the check pass alone — the
//! project is resolved off the clock — against a budget orders of magnitude
//! above what it costs.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use compose_core::resolve;

/// The provider and model every case needs, and a lister to fan out over.
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

/// A fork wide enough that anything recomputed per *pair* shows.
///
/// 240 branches are 28,680 co-takeable pairs over 1,441 nodes. What the two
/// rules read of an edge — its step distances (grammar 7.6.2) and what it
/// reaches (grammar 7.6.1) — is a property of the edge, so 480 walks answer for
/// every pair; taking them per pair instead made this shape cubic in the
/// out-degree and cost seconds, against a command whose budget is milliseconds
/// (PRD 5.12). Resolution is off the clock deliberately: this is a bound on the
/// graph analyses, not on the parser.
#[test]
fn a_wide_fork_is_checked_in_proportion_to_its_pairs() {
    let dir = scratch("wide");
    wide_fork_project(&dir, 240);
    let resolution = resolve(dir.join("main.yml"));
    assert!(
        resolution.diagnostics.is_empty(),
        "the generated project resolves cleanly, got {} diagnostic(s)",
        resolution.diagnostics.len()
    );
    let ir = resolution
        .ir
        .expect("a clean resolution produces an artifact");

    // The minimum of three runs, so a scheduling hiccup cannot fail the test.
    let budget = Duration::from_secs(4);
    let fastest = (0..3)
        .map(|_| {
            let started = Instant::now();
            let diagnostics = compose_core::check(&ir);
            assert!(diagnostics.is_empty(), "the wide fork checks cleanly");
            started.elapsed()
        })
        .min()
        .expect("three runs");
    println!("240-branch fork: {fastest:?}");
    assert!(
        fastest < budget,
        "checking a 240-branch fork took {fastest:?}, and the budget is {budget:?}"
    );
}
