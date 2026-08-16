//! Shapes that are big rather than merely legal.
//!
//! `check_accepts.rs` proves the validator accepts what the grammar admits;
//! this file proves it still *answers* when what the grammar admits is large.
//! Neither a flow's node count, nor a node's out-degree, nor the paths through
//! it, nor a composition's nesting depth is bounded by anything the compiler
//! controls, and each of the four has its own failure mode:
//!
//! * **depth** — a traversal written with the call stack aborts the process on a
//!   stack overflow, which is not a diagnostic, not an exit code the CLI
//!   documents, and not something `--format json` can report;
//! * **out-degree** — a per-*pair* analysis of a fork is quadratic in the number
//!   of edges before it computes anything, so anything it recomputes per pair
//!   multiplies out. Four shapes are needed, because a fork has four costs: one
//!   fan of disjoint branches, where the pairs share no answers and what matters
//!   is that nothing is *walked* per pair; one of overlapping branches, where
//!   the pairs share almost every answer and what matters is that nothing is
//!   *compared* or *crossed* per pair; one whose branches are **illegal** —
//!   every one of them writing the same unreduced channel — where what matters is
//!   that the report is one diagnostic and not one per pair; and one whose edges
//!   are **guarded**, which is the only shape that enters grammar 7.6.1's rule 2
//!   at all and where what matters is that no guard is *read* per pair. The
//!   clean ones among them time the analysis with its diagnostic path switched
//!   off, and a report quadratic in the out-degree is invisible to them;
//! * **path count** — a branch's step distances (grammar 7.6.2) are a property
//!   of its paths, of which a graph of *n* nodes has exponentially many and a
//!   node may be reached at *n* distinct depths. Neither fork shape above shows
//!   it, because both give every node exactly one distance from any entry, so a
//!   third is needed where a branch's distances are genuinely many;
//! * **size** — the usual one, and the one the two worked projects already
//!   guard through `crates/agent-compose/tests/cli.rs`.
//!
//! Every case here is written so that a regression shows up as a failure rather
//! than as a slow test: the depth case runs on a thread whose stack is far too
//! small to recurse through, and the graph cases time the check pass alone — the
//! project is resolved off the clock — against a budget several times what they
//! cost and a fraction of what the shape costs the analysis they guard. Each
//! names both numbers, so a budget can be re-fitted from what it is for.

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

/// A chain of `length` nodes in which every node skips one ahead as well as
/// stepping to the next, and forks aside to a leaf of its own.
///
/// The two forward edges are guarded on the source node's own enum field and are
/// provably exclusive (grammar 7.6.1 rule 2), so no convergence on the chain is
/// ever unbalanced and the shape is **legal**. The third edge is guarded on
/// state, which rule 2 cannot read for disjointness, so it is co-takeable with
/// both: every node is a fork and every branch below one gets walked.
///
/// What that buys is the one thing the two shapes above cannot have: a branch
/// whose distances are *multi-valued*. Stepping and skipping in any mix puts the
/// node `k` ahead at every depth between `k/2` and `k`, so a walk that enumerates
/// `(node, distance)` pairs is quadratic in the chain before the walk per branch
/// is counted at all — a legal flow answering "clean" in tens of seconds, against
/// a command whose budget is milliseconds (PRD 5.12). Reading a branch as the
/// nearest and farthest arrival at each node is what makes it one relaxation in
/// topological order (grammar 7.6.2, `check/graph.rs`).
///
/// `agent.b` is the reason this file's one shared channel is *not* wired here:
/// its output field is named after no channel, so no node of the chain writes
/// one, grammar 10.2's rule short-circuits over an empty writer set, and what is
/// left on the clock is the distance walk alone. The cross of two branches'
/// writers is the shape above's subject, and mixing the two would leave a
/// regression in either one hiding inside the other's cost.
fn skipping_chain_project(dir: &Path, length: usize) {
    let mut nodes = String::new();
    let mut edges = String::from("    - { from: start, to: n0 }\n");
    for at in 0..length {
        nodes.push_str(&format!(
            "    n{at}: {{ agent: agent.b, input: \"'x'\" }}\n    t{at}: {{ agent: agent.b, input: \"'x'\" }}\n"
        ));
        let step = if at + 1 < length {
            format!("n{}", at + 1)
        } else {
            "end".to_string()
        };
        let skip = if at + 2 < length {
            format!("n{}", at + 2)
        } else {
            "end".to_string()
        };
        edges.push_str(&format!("    - {{ from: t{at}, to: end }}\n"));
        edges.push_str(&format!(
            "    - {{ from: n{at}, to: {step}, when: \"n{at}.output.outcome == 'approve'\" }}\n"
        ));
        edges.push_str(&format!(
            "    - {{ from: n{at}, to: {skip}, when: \"n{at}.output.outcome == 'revise'\" }}\n"
        ));
        edges.push_str(&format!(
            "    - {{ from: n{at}, to: t{at}, when: \"state.verdict == 'approve'\" }}\n"
        ));
    }
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{BACKEND}agent.b:\n  model: model.m\n  prompt: Do it.\n  output:\n    outcome: {{ enum: [approve, revise] }}\nflow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// The minimum of three checks of one artifact, so a scheduling hiccup cannot
/// fail a test about an algorithm. Resolution is off the clock deliberately:
/// these are bounds on the graph analyses, not on the parser.
///
/// How many diagnostics come back is part of the measurement rather than a
/// separate assertion. Most shapes here are legal and report nothing, so the
/// clock is on the analysis alone; the one that is *not* legal is timed with the
/// report it is supposed to produce, because a rule whose cost is what it prints
/// is only bounded if the print is (PRD G3).
fn fastest_check(ir: &compose_core::Ir, what: &str, reported: usize) -> Duration {
    let fastest = (0..3)
        .map(|_| {
            let started = Instant::now();
            let diagnostics = compose_core::check(ir);
            assert_eq!(
                diagnostics.len(),
                reported,
                "{what} reports {reported} diagnostic(s), first was {:?}",
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
    let fastest = fastest_check(&artifact(&dir), "240-branch fork", 0);
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
    let fastest = fastest_check(&artifact(&dir), "5x50 layered fan-out", 0);
    assert!(
        fastest < budget,
        "checking a 5-by-50 layered fan-out took {fastest:?}, and the budget is {budget:?}"
    );
}

/// Branches whose **distances** are many, which is where anything counted per
/// path rather than per node shows.
///
/// 600 chain nodes and their 600 leaves are 1,200 nodes and 2,400 edges, and the
/// branch below any of the 600 forks reaches the far end of the chain at some
/// three hundred distinct depths. Neither shape above can see that: a fan of
/// disjoint chains and a layered rank both give every node exactly one distance
/// from any entry, so a walk over `(node, distance)` pairs behaves there like a
/// walk over nodes. Here it does not — enumerating them cost this shape 7 s at
/// half this size and a minute at twice it, on a flow with nothing wrong with it,
/// while the two extremes settle it in one pass (grammar 7.6.2, Decision D112).
#[test]
fn multi_valued_distances_are_checked_in_proportion_to_the_nodes() {
    let dir = scratch("skipping");
    skipping_chain_project(&dir, 600);
    let budget = Duration::from_secs(6);
    let fastest = fastest_check(&artifact(&dir), "600-node skipping chain", 0);
    assert!(
        fastest < budget,
        "checking a 600-node skipping chain took {fastest:?}, and the budget is {budget:?}"
    );
}

/// The one backend here whose channel carries **no** `reduce:`, so that every
/// node writing it by name (grammar 8.0) races every other one.
///
/// `BACKEND` declares `last_wins` deliberately — it keeps the shapes above legal
/// while their branches all write — and that is exactly what switches grammar
/// 10.2's diagnostic off. This one switches it on.
const RACED: &str = r#"provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.n:
  model: model.m
  prompt: Note.
  output:
    note: { type: string }
state:
  note: { type: string, default: "" }
"#;

/// `width` branches straight off `start`, each one node writing the unreduced
/// channel `note`, each straight to `end`.
///
/// `start` is a fork like any other vertex (grammar 7.3 rule 7, 7.6 step 0), so
/// its unguarded edges are co-takeable in pairs: `width * (width - 1) / 2` pairs,
/// every one of them a racing pair of writers, and one missing keyword.
fn racing_fork_project(dir: &Path, width: usize) {
    let mut nodes = String::new();
    let mut edges = String::new();
    for at in 0..width {
        nodes.push_str(&format!(
            "    n{at}: {{ agent: agent.n, input: \"'x'\" }}\n"
        ));
        edges.push_str(&format!("    - {{ from: start, to: n{at} }}\n"));
        edges.push_str(&format!("    - {{ from: n{at}, to: end }}\n"));
    }
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{RACED}flow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// The one backend here whose fork node carries **enum** output fields, so that
/// grammar 7.6.1's rule 2 has something to read, and whose one channel no node
/// writes.
///
/// Five enum fields, not one: rule 2 proves a pair exclusive when *some* field
/// of the source's output separates the two guards, so it is asked of every one
/// of them, and an agent that returns a verdict alongside a few other closed
/// choices is an ordinary shape. `agent.g`'s fields are named after no channel,
/// so the concurrent-write rule short-circuits over an empty writer set and what
/// is left on the clock is the guard reading alone — the same isolation
/// `agent.b` gives the distance walk above, and for the same reason.
const GUARDED: &str = r#"provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.g:
  model: model.m
  prompt: Do it.
  output:
    outcome: { enum: [approve, revise] }
    tone: { enum: [warm, plain, terse] }
    risk: { enum: [low, medium, high] }
    stage: { enum: [draft, review, final] }
    mood: { enum: [calm, urgent] }
state:
  seen:
    type: array
    max_items: 10
    items: { type: string }
    default: []
"#;

/// One `r` node with `width` **guarded** out-edges, each to a leaf of its own,
/// plus the `else: true` that routes the variant they all leave out.
///
/// Every guard is a conjunction of `conjuncts` terms of which only the first is
/// a shape the coverage table reads; the rest are over `state` and widen nothing
/// (grammar 7.3.1, Decision D82). All `width` guards leave the same one variant
/// of `outcome` possible and every variant of the other four, so no pair of them
/// is disjoint on any field: the fork is co-takeable at full width, and every
/// pair reaches the end of rule 2 rather than short-circuiting inside it.
fn guarded_fork_project(dir: &Path, width: usize, conjuncts: usize) {
    let mut nodes = String::from("    r: { agent: agent.g, input: \"'r'\" }\n");
    let mut edges = String::from("    - { from: start, to: r }\n");
    for at in 0..width {
        nodes.push_str(&format!(
            "    t{at}: {{ agent: agent.g, input: \"'t'\" }}\n"
        ));
        let rest: String = (0..conjuncts - 1)
            .map(|term| format!(" && size(state.seen) > {}", at + term))
            .collect();
        edges.push_str(&format!(
            "    - {{ from: r, to: t{at}, when: \"r.output.outcome == 'approve'{rest}\" }}\n"
        ));
        edges.push_str(&format!("    - {{ from: t{at}, to: end }}\n"));
    }
    nodes.push_str("    z: { agent: agent.g, input: \"'z'\" }\n");
    edges.push_str("    - { from: r, to: z, else: true }\n");
    edges.push_str("    - { from: z, to: end }\n");
    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{GUARDED}flow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// A fork whose out-edges are **guarded**, which is where anything read off a
/// guard per pair shows.
///
/// The three fan shapes above all carry unconditional edges, so grammar 7.6.1's
/// rule 1 settles every pair before a guard is looked at, and rule 2 — the one
/// that walks a CEL AST and builds a variant set — is never entered at width.
/// Here it is: 240 guarded edges are 28,680 co-takeable pairs, and what a pair
/// needs of an edge is what that edge's guard leaves *possible* for each of the
/// five enum fields, which is a property of the guard and not of the pair.
/// Reading it per pair walked every nineteen-conjunct AST 240 times over for
/// each field and cost this shape 10.2 s of check against 1.06 s once it is read
/// per edge — a composition with nothing wrong with it, against a command whose
/// budget is milliseconds (PRD 5.12). What is left inside the budget is the pair
/// enumeration itself, which the rule is stated over and no cache removes.
#[test]
fn a_guarded_fork_reads_each_guard_once_rather_than_once_per_pair() {
    let dir = scratch("guarded");
    guarded_fork_project(&dir, 240, 19);
    let budget = Duration::from_secs(6);
    let fastest = fastest_check(&artifact(&dir), "240-branch guarded fork", 0);
    assert!(
        fastest < budget,
        "checking a 240-branch guarded fork took {fastest:?}, and the budget is {budget:?}"
    );
}

/// A fan of branches that all race one channel, which is where anything
/// **reported** per pair shows.
///
/// 120 branches are 7,140 co-takeable pairs, 7,140 racing pairs of writers — and
/// one mistake: the missing `reduce:` on the one channel they all write. A
/// diagnostic per pair made that a 7,140-error report taking 1.4 s, against 190
/// errors at twenty branches and 45 at ten — an ordinary fan of parallel steps
/// turning one missing keyword into a wall of errors nobody can read (PRD G3),
/// on a rule whose analysis is fine. Neither shape above can see it: both declare
/// `reduce: last_wins`, so both cross the same quadratic with nothing to say
/// about it. The count is the assertion here and the clock is the corroboration —
/// a report that grows with the pairs cannot stay inside a budget this shape
/// answers in milliseconds.
#[test]
fn a_fan_racing_one_channel_is_one_diagnostic() {
    let dir = scratch("racing");
    racing_fork_project(&dir, 120);
    let budget = Duration::from_secs(6);
    let fastest = fastest_check(&artifact(&dir), "120-branch fan racing one channel", 1);
    assert!(
        fastest < budget,
        "checking a 120-branch racing fan took {fastest:?}, and the budget is {budget:?}"
    );
}
