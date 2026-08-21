//! The three `map` rules that are not decided from schemas: `over` reads a
//! dominating node, and `detach:` is legal under the active target and over what
//! it dispatches (grammar 8.6 rules 11 and 7).
//!
//! # `over` reads a dominating node (rule 11, Decision D76)
//!
//! In `over: <node>.output.…`, `<node>` MUST **dominate** the map node: every
//! path from the flow's `start` to the map node passes through it. Mere
//! path-existence is not enough — a producer sitting on a guarded sibling branch
//! may not have run when the map dispatches, leaving `over` with no value at
//! all — and dominance is the reading under which the value's existence is a
//! guarantee rather than a hope. It is computed on the same graph grammar 7.4's
//! SCC analysis builds, and stays well defined with cycles present because a
//! back-edge adds no new path from `start`.
//!
//! Reading the **edge** graph rather than grammar 7.8's control-transfer
//! relation is what grammar 8.6 rule 11 says, and it is the only place in this
//! module where the two could differ: a map node addressed by an
//! `on_error: { fallback: … }` is scheduled without its producer having run, and
//! this check does not see that path. The half rule 1 shares with the schemas —
//! that `over` resolves to a bounded array at all — is [`maps`](super::maps)'.
//!
//! # `detach:` under a checkpointed target (rule 7, Decision D59)
//!
//! `detach: true` is a v0 validation error under any target whose execution
//! state is durably checkpointed, which grammar 14 fixes as every target except
//! `local`. Checkpointing is a property of the target rather than a spec
//! construct, so this is a **target-dependent** rule like backend alias
//! resolution: the same composition is legal under `--target local` and rejected
//! under `--target staging`. What a detached dispatch may *write* is decided
//! from the composition's channels alone and is [`maps`](super::maps)'.
//!
//! # A detached dispatch reaches no `human` node (rule 7, Decision D118)
//!
//! The other half of the same rule, and it holds under every target. A detached
//! dispatch is resolved the moment it is issued: the join never observes its
//! instance, and the execution that issued it can finish while the instance is
//! still in flight. A `human` node inside one is therefore a question with
//! nothing left to receive its answer — the wait belongs to an execution that is
//! not waiting for it, and is dropped when that execution ends — so the
//! construct is refused rather than given a semantics that depends on which of
//! the two finishes first. It is decided over grammar 7.7's reachability, like
//! the `respond: sync` rule it mirrors: a `human` node inside a flow the target
//! instantiates, or inside a `flow.*` tool of an agent it dispatches, is as
//! unanswerable as one written in the target itself.

use crate::ast::common::Address;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::flow::{Map, MapDispatch, NodeKind};
use crate::resolve::DEFAULT_TARGET;

use super::graph::Graph;
use super::reach;
use super::{Ctx, FlowCx};

/// Check every `map` node of one flow.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    for at in 0..graph.nodes().len() {
        let NodeKind::Map { map } = &graph.node(at).kind else {
            continue;
        };
        dominance(ctx, cx, graph, at, map);
        detach(ctx, map);
        detached_pause(ctx, cx, graph, at, map);
    }
}

/// Grammar 8.6 rule 11.
fn dominance<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, at: usize, map: &'a Map) {
    let root = map.over.value.root.as_str();
    // `over` may also read `input` or `state`, which are values the instance
    // holds rather than a node's result and so have no producer to dominate
    // anything. Neither can name a node: both are reserved (grammar 2.5).
    let Some(producer) = graph
        .nodes()
        .iter()
        .position(|node| node.id.value.as_str() == root)
    else {
        return;
    };
    if graph.dominates(producer, at) {
        return;
    }
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::NonDominatingSource,
            map.over.span.clone(),
            format!(
                "`over` of the `map` of node `{}` in `{}` reads `{root}`, which does not dominate it",
                graph.id(at),
                cx.address
            ),
        )
        .with_label(
            graph.node(producer).id.span.clone(),
            "the producer is declared here",
        )
        .with_help(
            "every path from `start` to the map node must pass through the producer, or the array `over` reads may not exist when the map dispatches — a producer on a guarded sibling branch may not have run (grammar 8.6 rule 11, Decision D76)",
        ),
    );
}

/// Every dispatch of one `map` block that declares `detach: true`, with the
/// target it dispatches — the pair both halves of rule 7 are stated over.
fn detached(map: &Map) -> Vec<(&Spanned<bool>, &Spanned<Address>)> {
    match &map.dispatch {
        MapDispatch::Homogeneous { node, detach, .. } => detach
            .iter()
            .filter(|detach| detach.value)
            .map(|detach| (detach, node))
            .collect(),
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .filter_map(|route| {
                route
                    .detach
                    .as_ref()
                    .filter(|detach| detach.value)
                    .map(|detach| (detach, &route.node))
            })
            .collect(),
    }
}

/// Grammar 8.6 rule 7's target-dependent half.
fn detach(ctx: &mut Ctx, map: &Map) {
    let target = ctx.ir.target.clone();
    if target == DEFAULT_TARGET {
        return;
    }
    for (detach, _) in detached(map) {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnsupportedDetach,
                detach.span.clone(),
                format!("`detach: true` is not supported under the target `{target}`"),
            )
            .with_help(format!(
                "only `{DEFAULT_TARGET}` runs with an in-memory checkpointer; every other target is durably checkpointed, and v0 has no outbox-pattern delivery for a fire-and-forget dispatch under one — drop `detach:`, or validate against `{DEFAULT_TARGET}` (grammar 8.6 rule 7, 14, Decision D59)"
            )),
        );
    }
}

/// Grammar 8.6 rule 7's other half: a detached dispatch reaches no `human` node
/// (Decision D118).
///
/// Target-independent, and decided over grammar 7.7's relation — the same walk
/// the `respond: sync` rule reads, asked of one dispatch target instead of a
/// trigger's flow. The **first** `human` node the target reaches is reported:
/// one diagnostic per detached dispatch says the thing that is wrong with the
/// dispatch, where one per reachable pause would repeat it.
fn detached_pause<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, at: usize, map: &Map) {
    for (detach, target) in detached(map) {
        let reached = reach::reached_by(ctx.ir, &target.value);
        let Some(((holder, node), declared)) = reached.humans.iter().next() else {
            continue;
        };
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::DetachedInterrupt,
                detach.span.clone(),
                format!(
                    "the `map` of node `{}` in `{}` dispatches `{}` with `detach: true`, and `{}` reaches the `human` node `{node}` of `{holder}`",
                    graph.id(at),
                    cx.address,
                    target.value,
                    target.value,
                ),
            )
            .with_label(declared.clone(), "the `human` node is declared here")
            .with_label(target.span.clone(), "the detached dispatch targets it here")
            .with_help(
                "a detached dispatch is resolved the moment it is issued: the join never observes its instance, and the execution that issued it can finish while the instance is still in flight — so a pause inside one is a question nothing is left to answer, and the wait is dropped when that execution ends. Drop `detach:` so the dispatch is joined, or take the `human` node out of what it dispatches — its own nodes, the flows it instantiates, and the `flow.*` tools of any agent it reaches (grammar 8.6 rule 7, 8.7, 7.7, Decisions D94, D118)",
            ),
        );
    }
}
