//! Cycle termination: every strongly connected component is bounded, and every
//! bounded edge has an escape (grammar 7.4, Decisions D19, D57, D98).
//!
//! # Bounding
//!
//! PRD 5.4 accepts two proofs and this is the decidable form of both. An SCC
//! with at least one edge — two or more nodes, or one node with a self-edge —
//! is **bounded** when either:
//!
//! 1. some edge whose `from` *and* `to` are both in it carries
//!    `max_iterations:` (and therefore a `when:`, Decision D90); or
//! 2. some node `n` in it has **a way out** — an outgoing edge leaving the SCC —
//!    and **a pass that takes it**: no outgoing edge of `n` that stays inside
//!    the SCC is unconditional, and if one of them carries `else: true`, then
//!    `n` also has a `when:`-guarded outgoing edge that leaves the SCC.
//!
//! Clause 2 is about the *in-SCC* edges going false, not about the shape of the
//! exit (Decision D98). Under multicast routing a loop continues iff some in-SCC
//! edge fires, so an unconditional in-SCC edge re-enters every pass whatever the
//! exit says, and an in-SCC `else: true` edge fires unless a guarded sibling was
//! **taken** — which only a guarded sibling that *leaves* can do without
//! re-entering. Both spellings of the review loop are therefore bounded: a
//! guarded back-edge beside a guarded exit, and a guarded back-edge beside an
//! `else: true` escape.
//!
//! Only clause 1 is a static termination proof; clause 2's guard is a runtime
//! value. PRD 5.4 accepts both, so this does too (grammar 7.4).
//!
//! # The escape
//!
//! Separately, and whatever else bounds the SCC, the **source node of each
//! `max_iterations` edge** must have an outgoing edge that leaves the SCC *and*
//! is unconditional or carries `else: true`. An exhausted edge is not taken
//! whatever its guard says (grammar 7.3 rule 5), so a guarded exit leaves the
//! pass that spends the budget with no viable route — an infinite loop converted
//! into a runtime dead end, which is the failure Decision D19's rule removes.
//!
//! Two sources are exempt, and both for the same reason: a budget that cannot
//! run out withdraws no guarantee.
//!
//! * An edge leaving **`start`**. `start` belongs to no component, so every edge
//!   leaving it leaves "its" SCC, and grammar 7.6.3 rule 2 — which the parser
//!   owns — already requires one of them to be unconditional or `else: true`.
//! * A node **no cycle reaches**. `max_iterations` counts traversals of one edge
//!   within one flow instance (grammar 7.4), and a node no cycle reaches
//!   executes once per instance, so a budget of at least one — which the range
//!   1..=1000 guarantees — is never spent and the edge is never untakeable.
//!   Grammar 7.4's escape bullet reads over "each `max_iterations`-carrying
//!   edge" while grammar 7.2's own gloss of the key says the source "then also
//!   needs an unconditional or `else:` edge **leaving the cycle**", which
//!   presupposes one; refusing the shape no loop can reach would reject a
//!   composition that is runtime-safe, and that is the one direction Decisions
//!   D99 and D112 say a conservative static check must not go. The budget there
//!   is inert rather than dangerous, and no rule in this grammar refuses it.
//!
//!   It is *reaches* and not *is a member of*, because what spends a budget is a
//!   node running twice rather than a node looping: a node the loop routes to
//!   runs once per pass (grammar 7.6: the union of the targets of the edges taken
//!   at step k is step k+1), so its budget can be spent while the flow is still
//!   running and the pass that spends it dead-ends on grammar 7.3 rule 7 — the
//!   very failure this rule exists to remove. Only a node standing outside every
//!   loop's reach ([`Graph::reached_by_cycle`]) is exempt.
//!
//!   Reachability is also where the exemption stops. A finer gate is arguable —
//!   a node whose every incoming edge is *exclusive* with every in-SCC edge
//!   (grammar 7.6.1) still runs once, however deep in the loop's reach it sits —
//!   but grammar 7.4 states its escape bullet over "each `max_iterations`-carrying
//!   edge" with no exemption at all. Every line of this one is therefore a
//!   relaxation of what the grammar says, and a relaxation is kept no wider than
//!   the argument that carries it.

use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::flow::Edge;

use super::graph::{Graph, Vertex};
use super::{Ctx, FlowCx};

/// Check every cycle of one flow.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    for members in graph.components() {
        let Some(first) = members.first().copied() else {
            continue;
        };
        if !graph.cyclic(first) || bounded(graph, members) {
            continue;
        }
        let ids: Vec<&str> = members.iter().map(|member| graph.id(*member)).collect();
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnboundedCycle,
                graph.node(first).id.span.clone(),
                format!(
                    "the cycle through {} in `{}` carries no bounded edge",
                    crate::parse::reader::list(&ids),
                    cx.address
                ),
            )
            .with_help(
                "every cycle must terminate: put `max_iterations:` on a `when:`-guarded edge inside it, or give one of its nodes a CEL exit condition — an edge that leaves the cycle, plus in-cycle edges that can all go untaken on one pass (grammar 7.4, Decisions D57, D98)",
            ),
        );
    }

    // A node no cycle reaches runs once, so its budget is never spent (above).
    // The mask costs one walk, so it is taken only where a budget exists to ask
    // about — which is no flow at all in most compositions.
    let mut looping: Option<Vec<bool>> = None;
    for at in 0..graph.nodes().len() {
        for edge in graph.outgoing(Vertex::Node(at)) {
            if graph.edge(*edge).max_iterations.is_none() {
                continue;
            }
            let looping = looping.get_or_insert_with(|| graph.reached_by_cycle());
            if !looping[at] || escapes(graph, at) {
                continue;
            }
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::DeadEnd,
                    graph.edge(*edge).span.clone(),
                    format!(
                        "the `max_iterations` edge from `{}` to {} in `{}` has no escape",
                        graph.id(at),
                        describe(graph, *edge),
                        cx.address
                    ),
                )
                .with_label(graph.node(at).id.span.clone(), "the source node is here")
                .with_help(help(graph, at)),
            );
        }
    }
}

/// Whether an SCC satisfies either of grammar 7.4's two clauses.
fn bounded(graph: &Graph<'_>, members: &[usize]) -> bool {
    for member in members {
        for edge in graph.outgoing(Vertex::Node(*member)) {
            if graph.edge(*edge).max_iterations.is_some() && inside(graph, *member, *edge) {
                return true;
            }
        }
    }
    members.iter().any(|member| exits(graph, *member))
}

/// Grammar 7.4 clause 2, of one node of the SCC.
fn exits(graph: &Graph<'_>, member: usize) -> bool {
    let outgoing = graph.outgoing(Vertex::Node(member));
    let (staying, leaving): (Vec<usize>, Vec<usize>) = outgoing
        .iter()
        .partition(|edge| inside(graph, member, **edge));
    // 2(i) — there is a way out.
    if leaving.is_empty() {
        return false;
    }
    // 2(ii) — there is a pass that takes it.
    if staying.iter().any(|edge| unconditional(graph.edge(*edge))) {
        return false;
    }
    if staying
        .iter()
        .any(|edge| graph.edge(*edge).else_edge.is_some())
        && !leaving.iter().any(|edge| graph.edge(*edge).when.is_some())
    {
        return false;
    }
    true
}

/// Whether an edge stays inside the component of the node it leaves. An edge to
/// `end` leaves it: `end` is not a node and belongs to no component.
fn inside(graph: &Graph<'_>, node: usize, edge: usize) -> bool {
    graph
        .target(edge)
        .is_some_and(|target| graph.inside_component_of(node, target))
}

/// Whether a node has an escape: an outgoing edge that leaves its component and
/// is guaranteed to fire (grammar 7.4, Decision D19).
fn escapes(graph: &Graph<'_>, node: usize) -> bool {
    graph
        .outgoing(Vertex::Node(node))
        .iter()
        .any(|edge| !inside(graph, node, *edge) && graph.edge(*edge).when.is_none())
}

/// The edit that discharges the escape rule, which reads differently either side
/// of the cycle: a looping node has to *leave* its SCC, while a node the loop
/// merely routes to belongs to no SCC with an edge, so every outgoing edge of it
/// already leaves one and only the guarantee is missing.
fn help(graph: &Graph<'_>, node: usize) -> String {
    let id = graph.id(node);
    if graph.cyclic(node) {
        return format!(
            "an exhausted edge is not taken whatever its guard says, so the pass that spends the budget needs somewhere to go: give `{id}` an outgoing edge that leaves the cycle *and* is unconditional or carries `else: true` — a guarded escape is not enough (grammar 7.4, Decision D19)"
        );
    }
    format!(
        "an exhausted edge is not taken whatever its guard says, and a cycle upstream runs `{id}` once per pass, so the budget is spent with the flow still going: give `{id}` an outgoing edge that is unconditional or carries `else: true` — a guarded escape is not enough (grammar 7.4, Decision D19)"
    )
}

/// An edge carrying neither `when:` nor `else:` (grammar 7.3 rule 2).
fn unconditional(edge: &Edge) -> bool {
    edge.when.is_none() && edge.else_edge.is_none()
}

/// How a diagnostic names an edge's destination.
fn describe(graph: &Graph<'_>, edge: usize) -> String {
    match graph.target(edge) {
        Some(Vertex::Node(at)) => format!("`{}`", graph.id(at)),
        _ => "`end`".to_string(),
    }
}
