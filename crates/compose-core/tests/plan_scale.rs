//! Two artifacts that are big rather than merely different.
//!
//! `tests/check_scale.rs` proves the validator still answers when a composition
//! is large; this file proves the same of the pass that reads **two** of them.
//! `agent-compose plan` is in `validate`'s class by construction — it resolves
//! both sides and runs the whole check phase over each — so what has to stay
//! inside the budget is the part that is new: the structural diff.
//!
//! One shape, three ways of being wrong, and each has a name:
//!
//! * **matching by scan** — a flow's edges have no id, so they are paired by
//!   identity (`docs/plan.md` §5). Three passes, each looking for its partner by
//!   walking the other side, is quadratic in the edge count before it compares
//!   anything: a flow with a few thousand edges is tens of millions of JSON
//!   comparisons, on a pair with one edge between them. The passes are indexes
//!   instead;
//! * **serializing per comparison** — every subject is compared as JSON with its
//!   source coordinates removed (`crates/compose-core/src/plan/diff.rs`), and
//!   the artifact is a tree. Serializing a definition once per candidate partner
//!   rather than once is the same quadratic wearing different clothes;
//! * **paying for what did not change** — the common call is two artifacts that
//!   agree almost everywhere, which is what a plan run in review looks like. A
//!   pair that is *identical* must cost the walk and nothing else, so it is
//!   timed here beside the edited pair rather than assumed.
//!
//! The clock is on [`plan`](compose_core::plan) alone: both artifacts are
//! resolved off it, because this is a bound on the diff and not on the parser.
//! The check phase is inside the measurement because it is inside the command,
//! and the shape is chosen so that its cost is small and flat — a straight chain
//! has no fork, so none of the pair-wise graph analyses `check_scale.rs` bounds
//! is entered at width, and what is left on the clock is the diff.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use compose_core::{Composition, Ir, resolve};

/// The provider, model and agent every generated flow node runs.
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
    note: { type: string }
"#;

/// A scratch directory of this test's own, cleaned out before use.
fn scratch(name: &str) -> PathBuf {
    let dir =
        Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("plan-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).expect("can create a scratch directory");
    dir
}

/// A composition with `length` chained nodes, `spare` unused agent definitions,
/// and `channels` state channels — optionally with the one edit this file
/// measures a plan of.
///
/// A **chain** rather than a fan: every graph analysis stated over pairs of
/// co-takeable edges (`check_scale.rs`) sees nothing here, so the check phase
/// stays flat and the clock is on the diff. What the chain *is* large in is
/// exactly what the diff walks — one node and one edge per step, every one of
/// them a subject to be matched and compared.
///
/// The edit is deliberately small and of three kinds at once: one node retimed,
/// one edge retargeted (which is the pass a plan cannot answer by `(from, to)`
/// alone), and one definition changed. A plan run in review is a handful of
/// changes over an artifact that is otherwise the same, and a bound taken over
/// a pair that differs everywhere would not be a bound on that.
fn chain_project(dir: &Path, length: usize, spare: usize, channels: usize, edited: bool) {
    let mut definitions = String::from(BACKEND);
    for at in 0..spare {
        definitions.push_str(&format!(
            "agent.spare{at}:\n  model: model.m\n  prompt: Spare {at}.\n  output:\n    note: {{ type: string }}\n"
        ));
    }
    if channels > 0 {
        definitions.push_str("state:\n");
        for at in 0..channels {
            definitions.push_str(&format!("  c{at}: {{ type: string, default: \"\" }}\n"));
        }
    }

    let mut nodes = String::new();
    let mut edges = String::from("    - { from: start, to: n0 }\n");
    for at in 0..length {
        // The retimed node: one `timeout:` the other side does not declare.
        let policy = if edited && at == length / 2 {
            ", timeout: 45s"
        } else {
            ""
        };
        nodes.push_str(&format!(
            "    n{at}: {{ agent: agent.a, input: \"'x'\"{policy} }}\n"
        ));
        let next = if at + 1 < length {
            format!("n{}", at + 1)
        } else {
            "end".to_string()
        };
        // The retargeted edge: the step that skips its successor, on one side
        // only.
        let to = if edited && at == 1 && length > 3 {
            "n3".to_string()
        } else {
            next
        };
        edges.push_str(&format!("    - {{ from: n{at}, to: {to} }}\n"));
    }
    // The changed definition, which is the components section's subject.
    let prompt = if edited { "Do it twice." } else { "Do it." };
    let definitions = definitions.replace("prompt: Do it.\n", &format!("prompt: {prompt}\n"));

    fs::write(
        dir.join("main.yml"),
        format!(
            "version: \"0.1\"\n{definitions}flow.f:\n  outputs: {{}}\n  nodes:\n{nodes}  edges:\n{edges}"
        ),
    )
    .expect("can write the entrypoint");
}

/// The artifact of a project that must resolve cleanly first.
fn artifact(dir: &Path) -> Ir {
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

/// The minimum of three plans of one pair, so a scheduling hiccup cannot fail a
/// test about an algorithm.
///
/// How many changes come back is part of the measurement rather than a separate
/// assertion: a diff that reported one line per shifted index would be inside
/// any time budget and still useless, and one that reported nothing would be
/// the fastest of all.
fn fastest_plan(before: &Ir, after: &Ir, what: &str, reported: usize) -> Duration {
    let fastest = (0..3)
        .map(|_| {
            let started = Instant::now();
            let plan = compose_core::plan(
                Composition {
                    entrypoint: "before/main.yml",
                    ir: before,
                    resolution: &[],
                },
                Composition {
                    entrypoint: "after/main.yml",
                    ir: after,
                    resolution: &[],
                },
            );
            assert_eq!(
                plan.len(),
                reported,
                "{what} reports {reported} change(s), got {:?} / {:?} / {} introduced",
                plan.components
                    .iter()
                    .map(|change| change.address.clone())
                    .collect::<Vec<_>>(),
                plan.topology
                    .iter()
                    .map(|change| change.address.clone())
                    .collect::<Vec<_>>(),
                plan.validation.introduced.len(),
            );
            started.elapsed()
        })
        .min()
        .expect("three runs");
    let checked = (0..3)
        .map(|_| {
            let started = Instant::now();
            let _ = compose_core::check(after);
            started.elapsed()
        })
        .min()
        .expect("three runs");
    println!("{what}: {fastest:?} (one check phase of the after side: {checked:?})");
    fastest
}

/// A pair whose flows are large and whose difference is small.
///
/// 2,000 chained nodes are 2,001 edges, beside 200 spare definitions and 200
/// channels — and three edits between the two sides: a node that gained a
/// `timeout:`, an edge that now skips a step, and an agent whose prompt was
/// rewritten. Matching those 2,001 edges by scanning the other side is four
/// million JSON comparisons per pass and three passes of it; indexing them is a
/// sort. The command's budget is `validate`'s (PRD 5.12), and what is measured
/// here sits inside it with room for the two check phases it also contains — both
/// numbers are printed, so the budget can be re-fitted from what it is for.
#[test]
fn a_large_pair_is_planned_in_proportion_to_what_it_declares() {
    let before = scratch("chain-before");
    let after = scratch("chain-after");
    chain_project(&before, 2_000, 200, 200, false);
    chain_project(&after, 2_000, 200, 200, true);

    let budget = Duration::from_secs(6);
    // Four: the agent whose prompt was rewritten, the node that gained a
    // `timeout:`, the edge that now skips a step — and the one diagnostic that
    // skipping it introduces, because `n2` is no longer reachable. Three edits
    // and their consequence, over 4,000 nodes that did not move.
    let fastest = fastest_plan(
        &artifact(&before),
        &artifact(&after),
        "2,000-node chain, three edits",
        4,
    );
    assert!(
        fastest < budget,
        "planning a 2,000-node chain took {fastest:?}, and the budget is {budget:?}"
    );
}

/// …and the common case, which is a pair that agrees.
///
/// The same composition on both sides: every definition compared and equal,
/// every node matched by id, every edge matched on the first pass. It is the
/// call a plan run in review makes most often, and the one where a diff that
/// pays per candidate rather than per subject shows up first — there is nothing
/// to report, so nothing else is on the clock.
#[test]
fn a_pair_that_agrees_costs_the_walk_and_nothing_more() {
    let dir = scratch("chain-same");
    chain_project(&dir, 2_000, 200, 200, false);
    let ir = artifact(&dir);

    let budget = Duration::from_secs(6);
    let fastest = fastest_plan(&ir, &ir, "2,000-node chain against itself", 0);
    assert!(
        fastest < budget,
        "planning a 2,000-node chain against itself took {fastest:?}, and the budget is \
         {budget:?}"
    );
}
