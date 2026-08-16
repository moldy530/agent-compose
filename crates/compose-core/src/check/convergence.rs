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
//!
//! # What a pair costs
//!
//! Both rules are stated over pairs, and a fork of *w* out-edges has w(w-1)/2 of
//! them — so a pair has to cost almost nothing. Three things keep it there, and
//! all three are the same observation: **a pair is read only through the two
//! nodes its edges deliver to** ([`Graph::entry`](super::graph::Graph::entry)).
//!
//! 1. *One walk per entry.* The step distances a branch delivers at
//!    (grammar 7.6.2) and the nodes it holds (grammar 7.6.1) are properties of
//!    where the edge lands, so both are taken once per entry node and paired out
//!    of a cache — not once per edge, and never once per pair. A layer of ten
//!    nodes each edging to the same ten is ten walks, not a hundred.
//! 2. *One comparison per entry pair.* The verdict of each rule is likewise a
//!    function of the two entries alone, so the same two branches are compared
//!    once however many forks put them side by side. The diagnostic is still the
//!    pair's own — it names that fork and those two edges — and only the answer
//!    is shared.
//! 3. *One decision per node pair.* Two branches can hold hundreds of nodes
//!    each, and the concurrent-write rule asks its question of every combination.
//!    It is asked over *writers* rather than nodes (a node that writes nothing
//!    cannot race), and a bitset of the pairs already decided is carried across
//!    the whole flow: a cross of two branches costs a machine word per writer to
//!    scan, and every pair it turns up is one being decided for the first and
//!    last time. The work is then bounded by the answer — pairs of writers — and
//!    not by how many forks separate them.
//!
//! A wide fan-out is a shape the grammar admits — nothing bounds a node's
//! out-degree, and `start` forks like any other vertex — while `validate` is a
//! millisecond-budget command (PRD 5.12); this is the one shape that could spend
//! that budget, and `tests/check_scale.rs` is where the bound is held.

use std::collections::{BTreeMap, BTreeSet};

use ::cel::Program;

use crate::diag::{Diagnostic, DiagnosticCode, Span};
use crate::ir::flow::{MapDispatch, Node, NodeKind};
use crate::ir::schema::TypeForm;

use super::channels::Written;
use super::graph::{Graph, Vertex};
use super::{Ctx, FlowCx, channels, guards};

/// What two branches deliver to at two different depths: the nearest such node,
/// and one distance from each side (grammar 7.6.2, Decision D112). `None` where
/// the two are balanced.
type Skew = Option<(usize, usize, usize)>;

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
    let mut distances: BTreeMap<usize, BTreeMap<usize, BTreeSet<usize>>> = BTreeMap::new();
    let mut skew: BTreeMap<(usize, usize), Skew> = BTreeMap::new();
    for pair in pairs {
        let (left, right) = pair.edges;
        // An edge that retires its branch at `end` delivers to no node, and
        // `end` is exempt anyway (grammar 7.6.2).
        let (Some(near), Some(far)) = (graph.entry(left), graph.entry(right)) else {
            continue;
        };
        for entry in [near, far] {
            distances
                .entry(entry)
                .or_insert_with(|| graph.distances(entry));
        }
        // Both the walk and the comparison read the pair only through the two
        // nodes it delivers to, so a fan-out whose branches repeat — every node
        // of one layer edging to every node of the next — asks each question
        // once instead of once per pair of sources.
        let found = match skew.get(&(near, far)) {
            Some(found) => *found,
            None => {
                let found = nearest(&distances[&near], &distances[&far]);
                skew.insert((near, far), found);
                found
            }
        };
        let Some((node, one, other)) = found else {
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

/// The nearest node the two branches deliver to at two different depths, as the
/// node and the two distances the diagnostic names.
fn nearest(
    near: &BTreeMap<usize, BTreeSet<usize>>,
    far: &BTreeMap<usize, BTreeSet<usize>>,
) -> Skew {
    let mut nearest: Option<(usize, usize, usize, usize)> = None;
    // Ascending node index, so a tie on the distance keeps the earlier node —
    // the order a scan of every node would have found them in.
    for (node, near) in near {
        let Some(far) = far.get(node) else {
            continue;
        };
        let Some((one, other)) = differ(near, far) else {
            continue;
        };
        let key = one.min(other);
        if nearest.as_ref().is_none_or(|(best, ..)| key < *best) {
            nearest = Some((key, *node, one, other));
        }
    }
    nearest.map(|(_, node, one, other)| (node, one, other))
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
    if pairs.is_empty() {
        return;
    }
    let Some(writers) = Writers::of(ctx, graph) else {
        return;
    };
    let mut branches: BTreeMap<usize, Branch> = BTreeMap::new();
    let mut crossed: BTreeSet<(usize, usize)> = BTreeSet::new();
    // One bit per ordered pair of *writers*, so a pair is decided once for the
    // whole flow however many forks separate it.
    let mut examined = vec![0u64; writers.count() * writers.words()];
    for pair in pairs {
        let (Some(near), Some(far)) = (graph.entry(pair.edges.0), graph.entry(pair.edges.1)) else {
            continue;
        };
        // What a branch holds depends on the pair only through the node it
        // delivers to, so repeated branches are crossed once.
        if !crossed.insert((near.min(far), near.max(far))) {
            continue;
        }
        for entry in [near, far] {
            branches
                .entry(entry)
                .or_insert_with(|| writers.branch(graph, entry));
        }
        for one in &branches[&near].members {
            let one = *one;
            let row = one * writers.words();
            for word in 0..writers.words() {
                // The writers of the far branch this one has never been paired
                // with. Every bit left standing is a pair decided for the first
                // and last time, so the scan costs a word per branch and the
                // decisions cost what the answer holds.
                let mut fresh = branches[&far].mask[word] & !examined[row + word];
                while fresh != 0 {
                    let other = word * 64 + fresh.trailing_zeros() as usize;
                    fresh &= fresh - 1;
                    examined[row + word] |= 1 << (other % 64);
                    examined[other * writers.words() + one / 64] |= 1 << (one % 64);
                    if one == other {
                        continue;
                    }
                    let (one, other) = writers.order(one, other);
                    // Neither reachable from the other: a node downstream of both
                    // branches runs after them, not beside them.
                    if graph.reaches(writers.node(one), writers.node(other))
                        || graph.reaches(writers.node(other), writers.node(one))
                    {
                        continue;
                    }
                    races(ctx, cx, graph, &writers, one, other);
                }
            }
        }
    }
}

/// The nodes of one flow that write a channel at all, and nothing else.
///
/// A node that writes none cannot race one for it, so the concurrent-write rule
/// is answered over *writers* rather than over nodes: they are what the branch
/// masks below index and what the examined-pair matrix is sized by. A flow that
/// declares no state has none, and the rule then costs one pass over its nodes
/// whatever its forks look like — which is also why the effective write map of
/// each node is taken once, here, instead of once per branch that holds it.
struct Writers<'a> {
    /// Per writer, the node it is and every channel that node writes.
    entries: Vec<(usize, Vec<Written<'a>>)>,
    /// Per node, its writer index — `None` for a node that writes nothing.
    slot: Vec<Option<usize>>,
}

impl<'a> Writers<'a> {
    /// The writers of one flow, or `None` where fewer than two can race.
    fn of(ctx: &Ctx<'a>, graph: &Graph<'a>) -> Option<Self> {
        let mut entries = Vec::new();
        let mut slot = Vec::with_capacity(graph.nodes().len());
        for at in 0..graph.nodes().len() {
            let written = written(ctx, graph.node(at));
            slot.push((!written.is_empty()).then(|| {
                entries.push((at, written));
                entries.len() - 1
            }));
        }
        (entries.len() > 1).then_some(Self { entries, slot })
    }

    fn count(&self) -> usize {
        self.entries.len()
    }

    /// The words one bitset over the writers occupies.
    fn words(&self) -> usize {
        self.entries.len().div_ceil(64)
    }

    /// The writers one branch holds, as the list its far side is scanned by and
    /// the mask its near side scans (grammar 7.6.1).
    fn branch(&self, graph: &Graph<'a>, entry: usize) -> Branch {
        let mut branch = Branch {
            members: Vec::new(),
            mask: vec![0u64; self.words()],
        };
        for node in graph.reachable_through(entry) {
            if let Some(at) = self.slot[node] {
                branch.members.push(at);
                branch.mask[at / 64] |= 1 << (at % 64);
            }
        }
        branch
    }

    /// The node one writer is.
    fn node(&self, at: usize) -> usize {
        self.entries[at].0
    }

    /// Every channel one writer writes.
    fn writes(&self, at: usize) -> &[Written<'a>] {
        &self.entries[at].1
    }

    /// Two writers in the order the diagnostic names them: ascending node index,
    /// so the message reads the same whichever branch each was found in.
    fn order(&self, one: usize, other: usize) -> (usize, usize) {
        if self.node(one) <= self.node(other) {
            (one, other)
        } else {
            (other, one)
        }
    }
}

/// The writers reachable through one edge (grammar 7.6.1), both ways round.
struct Branch {
    /// Their writer indexes, ascending.
    members: Vec<usize>,
    /// The same set as a bitset over writer indexes.
    mask: Vec<u64>,
}

/// The channels two concurrent nodes both write with no `reduce:` policy.
fn races<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    graph: &Graph<'a>,
    writers: &Writers<'a>,
    one: usize,
    other: usize,
) {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    for first in writers.writes(one) {
        for second in writers.writes(other) {
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
                        graph.id(writers.node(one)),
                        graph.id(writers.node(other)),
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
            // The node's id, not its block: the same anchor
            // [`channels::node_writes`] reports its own diagnostics at, for the
            // same reason — a block mapping ends where the next node's key
            // begins.
            channels::effective(ctx, &output, node.writes.as_ref(), &node.id.span)
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
