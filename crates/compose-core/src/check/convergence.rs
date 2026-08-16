//! Forks, co-takeable pairs, and the two rules stated over them: balanced
//! convergence and concurrent writes (grammar 7.6.1, 7.6.2, 10.2).
//!
//! # Co-takeability is a relation on a pair
//!
//! Two outgoing edges of one vertex are **exclusive** when the validator can
//! prove they are never taken together — one carries `else: true` and the other
//! `when:` (grammar 7.3 rule 4 makes those mutually exclusive by construction),
//! or both carry `when:` guards whose `possible` sets are disjoint for some
//! enum-typed field of the source node's output ([`guards`](super::guards), read
//! for disjointness instead of coverage). Every other pair is **co-takeable**,
//! and an edge is never co-takeable on its own: it is one or the other *with
//! respect to a named sibling* (Decision D99).
//!
//! A vertex with a co-takeable pair is a **fork**. `start` is one too — grammar
//! 7.6's step 0 runs the nodes its taken edges target, and grammar 7.3 rule 7
//! says outright that routing applies to `start` as well — but only rule 1 can
//! prove its edges exclusive, because `start` has no output for rule 2 to read
//! (grammar 2.4).
//!
//! # Balanced convergence
//!
//! `dist(f, e, d)` is the set of step distances from a fork `f` to a node `d`
//! over paths that leave `f` by the edge `e` and traverse no node belonging to a
//! cycle ([`Graph::distances`](super::graph::Graph::distances)). For one
//! co-takeable pair, if some `a ∈ dist(f, e₁, d)` differs from some
//! `b ∈ dist(f, e₂, d)`, the convergence at `d` runs once at step `a` and again
//! at step `b`, and that is a compile error naming both edges and both distances
//! (Decision D112).
//!
//! **One distance from each edge, never the union of one side.** Two paths of
//! different lengths on *one* side part at some node `g`, and `g` is where the
//! question belongs — asked of the two edges they leave it by, which this same
//! check compares if they are co-takeable and which deliver at most one arrival
//! if they are exclusive. Pooling the two sides would refuse a guarded shortcut
//! inside one branch, which is runtime-safe, while naming a pair that cannot
//! deliver the distances the diagnostic reports.
//!
//! `end` is exempt: it is not a node, it retires branches instead of running,
//! and branches legitimately reach it at different depths (grammar 7.6.3). One
//! diagnostic is reported per co-takeable pair, at the *nearest* convergence it
//! unbalances: every node downstream of that one inherits the same skew, and a
//! page of diagnostics for one mistake is not a diagnostic (PRD G3).
//!
//! # Concurrent writes
//!
//! Two nodes are **concurrent** when both are reachable from a common fork
//! through the two edges of one co-takeable pair — one through each — and
//! neither is reachable from the other (grammar 7.6.1). Writing one channel from
//! concurrent contexts requires that channel to declare a `reduce:` policy, on
//! exactly the rule the instances of a `map` obey (grammar 10.2, Decision D32);
//! the `map` half is [`maps`](super::maps)', and this is the concurrent-branch
//! half.
//!
//! The analysis is deliberately conservative: a guard pair it cannot prove
//! exclusive is co-takeable, so it may ask for a policy on a channel two
//! branches could not really both write. Declaring the policy is the cost, and
//! D32 already holds that a declared overwrite beats a silent race.

use std::collections::{BTreeMap, BTreeSet};

use ::cel::Program;

use crate::diag::{Diagnostic, DiagnosticCode, Span};
use crate::ir::flow::{MapDispatch, Node, NodeKind};
use crate::ir::schema::TypeForm;

use super::channels::Written;
use super::graph::{Graph, Vertex};
use super::{Ctx, FlowCx, channels, guards};

/// One pair of sibling out-edges that may both fire.
struct Pair {
    /// The fork they leave.
    source: Vertex,
    /// The two edge indexes, in declaration order.
    edges: (usize, usize),
}

/// Check one flow's forks.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    let pairs = pairs(ctx, graph);
    balanced(ctx, cx, graph, &pairs);
    concurrent(ctx, cx, graph, &pairs);
}

/// Every co-takeable pair of every fork (grammar 7.6.1, Decision D99).
fn pairs<'a>(ctx: &Ctx<'a>, graph: &Graph<'a>) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for source in graph.sources() {
        let outgoing = graph.outgoing(source);
        if outgoing.len() < 2 {
            continue;
        }
        let programs: Vec<Option<Program>> = outgoing
            .iter()
            .map(|edge| {
                let guard = graph.edge(*edge).when.as_ref()?;
                // A guard the CEL front-end rejected proves nothing about which
                // variants it admits, and reading it would turn one mistake into
                // a second, unrelated diagnostic downstream.
                ctx.guard_type_checked(&guard.span)
                    .then(|| guards::parse(guard.value.as_str()))
                    .flatten()
            })
            .collect();
        let enums = source_enums(ctx, graph, source);
        for left in 0..outgoing.len() {
            for right in (left + 1)..outgoing.len() {
                if exclusive(graph, source, outgoing, &programs, &enums, left, right) {
                    continue;
                }
                pairs.push(Pair {
                    source,
                    edges: (outgoing[left], outgoing[right]),
                });
            }
        }
    }
    pairs
}

/// The enum-typed fields of a fork's output, with their variant sets. `start`
/// has no output, so its edges are only ever proved exclusive by rule 1.
fn source_enums<'a>(
    ctx: &Ctx<'a>,
    graph: &Graph<'a>,
    source: Vertex,
) -> Vec<(String, BTreeSet<String>)> {
    let Vertex::Node(at) = source else {
        return Vec::new();
    };
    let Some(output) = ctx.node_output(graph.node(at)) else {
        return Vec::new();
    };
    output
        .fields
        .iter()
        .filter_map(|field| {
            let TypeForm::Enum(declared) = &field.ty.form else {
                return None;
            };
            Some((
                field.name.value.to_string(),
                declared
                    .variants
                    .iter()
                    .map(|variant| variant.value.clone())
                    .collect(),
            ))
        })
        .collect()
}

/// Grammar 7.6.1's two exclusivity rules.
fn exclusive(
    graph: &Graph<'_>,
    source: Vertex,
    outgoing: &[usize],
    programs: &[Option<Program>],
    enums: &[(String, BTreeSet<String>)],
    left: usize,
    right: usize,
) -> bool {
    let one = graph.edge(outgoing[left]);
    let other = graph.edge(outgoing[right]);
    // Rule 1: an `else:` edge fires only when no guarded sibling was taken.
    if (one.else_edge.is_some() && other.when.is_some())
        || (other.else_edge.is_some() && one.when.is_some())
    {
        return true;
    }
    // Rule 2: two guards no enum value satisfies at once.
    let Vertex::Node(at) = source else {
        return false;
    };
    let (Some(one), Some(other)) = (&programs[left], &programs[right]) else {
        return false;
    };
    let id = graph.id(at);
    enums.iter().any(|(field, variants)| {
        let one = guards::coverage(one.expression(), id, field, variants).possible;
        let other = guards::coverage(other.expression(), id, field, variants).possible;
        one.is_disjoint(&other)
    })
}

/// Grammar 7.6.2: the two edges of a pair may not deliver to one node at two
/// different depths.
fn balanced<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, pairs: &[Pair]) {
    for pair in pairs {
        let (left, right) = pair.edges;
        let near = graph.distances(left);
        let far = graph.distances(right);
        let mut nearest: Option<(usize, usize, usize, usize)> = None;
        for node in 0..graph.nodes().len() {
            let Some((one, other)) = differ(&near[node], &far[node]) else {
                continue;
            };
            let key = one.min(other);
            if nearest.as_ref().is_none_or(|(best, ..)| key < *best) {
                nearest = Some((key, node, one, other));
            }
        }
        let Some((_, node, one, other)) = nearest else {
            continue;
        };
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnbalancedConvergence,
                graph.node(node).id.span.clone(),
                format!(
                    "node `{}` of `{}` is reached from the fork {} at two different depths",
                    graph.id(node),
                    cx.address,
                    describe(graph, pair.source)
                ),
            )
            .with_label(
                graph.edge(left).span.clone(),
                format!("this edge reaches it in {}", steps(one)),
            )
            .with_label(
                graph.edge(right).span.clone(),
                format!("this one in {}", steps(other)),
            )
            .with_help(
                "a convergence reached in two different steps runs twice, once per arrival: route the short branch through the same depth, or make the two edges exclusive — `else: true` on one, or guards grammar 7.6.1 can prove disjoint (grammar 7.6.2, Decisions D69, D112)",
            ),
        );
    }
}

/// One distance from each side that differ, smallest first — never a comparison
/// inside one side (Decision D112).
fn differ(left: &BTreeSet<usize>, right: &BTreeSet<usize>) -> Option<(usize, usize)> {
    left.iter()
        .flat_map(|one| right.iter().map(move |other| (*one, *other)))
        .find(|(one, other)| one != other)
}

fn steps(count: usize) -> String {
    if count == 1 {
        "1 step".to_string()
    } else {
        format!("{count} steps")
    }
}

/// Grammar 10.2's concurrent-writer half: two nodes on the two sides of one
/// co-takeable pair may not both write an unreduced channel.
fn concurrent<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, pairs: &[Pair]) {
    let mut reported: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut writes: BTreeMap<usize, Vec<Written<'a>>> = BTreeMap::new();
    for pair in pairs {
        let (left, right) = pair.edges;
        let near = graph.reachable_through(left);
        let far = graph.reachable_through(right);
        for (one, reached) in near.iter().enumerate() {
            if !reached {
                continue;
            }
            for (other, reached) in far.iter().enumerate() {
                if one == other || !reached {
                    continue;
                }
                // Neither reachable from the other: a node downstream of both
                // branches runs after them, not beside them.
                if graph.reaches(one, other) || graph.reaches(other, one) {
                    continue;
                }
                let key = (one.min(other), one.max(other));
                if !reported.insert(key) {
                    continue;
                }
                for node in [key.0, key.1] {
                    writes
                        .entry(node)
                        .or_insert_with(|| written(ctx, graph.node(node)));
                }
                races(ctx, cx, graph, &writes, key);
            }
        }
    }
}

/// The channels two concurrent nodes both write with no `reduce:` policy.
fn races<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    graph: &Graph<'a>,
    writes: &BTreeMap<usize, Vec<Written<'a>>>,
    (one, other): (usize, usize),
) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for first in &writes[&one] {
        for second in &writes[&other] {
            let name = first.channel.name.value.as_str();
            if second.channel.name.value.as_str() != name || first.channel.reduce.is_some() {
                continue;
            }
            if !seen.insert(name) {
                continue;
            }
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::UnreducedWrite,
                    second.at.clone(),
                    format!(
                        "nodes `{}` and `{}` of `{}` run concurrently and both write the unreduced channel `{name}`",
                        graph.id(one),
                        graph.id(other),
                        cx.address
                    ),
                )
                .with_label(first.at.clone(), "the other write is here")
                .with_label(first.channel.span.clone(), "the channel is declared here")
                .with_help(
                    "two edges of one fork that are not provably exclusive can both fire, so the branches they start are concurrent: the channel they both write needs a declared `reduce:` policy — `append`, `merge`, or an explicit `last_wins` (grammar 7.6.1, 10.2, Decision D32)",
                ),
            );
        }
    }
}

/// Every channel one node writes: its effective write map, or — for a `map`
/// node, which has no output of its own — the effective write map of each
/// dispatch it issues (grammar 8.0, 8.6 rule 9).
fn written<'a>(ctx: &Ctx<'a>, node: &'a Node) -> Vec<Written<'a>> {
    let NodeKind::Map { map } = &node.kind else {
        return ctx.node_output(node).map_or_else(Vec::new, |output| {
            channels::effective(ctx, &output, node.writes.as_ref(), &node.span)
        });
    };
    let sites: Vec<(
        &crate::ast::common::Address,
        Option<&crate::ir::binding::Writes>,
        &Span,
    )> = match &map.dispatch {
        MapDispatch::Homogeneous {
            node: target,
            writes,
            ..
        } => vec![(&target.value, writes.as_ref(), &map.span)],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| (&route.node.value, route.writes.as_ref(), &route.span))
            .collect(),
    };
    sites
        .into_iter()
        .filter_map(|(target, writes, at)| {
            let output = ctx.target_output(target)?;
            Some(channels::effective(ctx, output, writes, at))
        })
        .flatten()
        .collect()
}

/// How a diagnostic names a fork.
fn describe(graph: &Graph<'_>, source: Vertex) -> String {
    match source {
        Vertex::Node(at) => format!("`{}`", graph.id(at)),
        _ => "`start`".to_string(),
    }
}
