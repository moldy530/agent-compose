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
//! A pair whose two edges land on **one** node is that same reading seen from
//! the other end, and both rules skip it: grammar 7.6's P2 runs a node targeted
//! by several edges taken in one step exactly once, so such a pair starts a
//! single branch, and the two sides the rules would compare are the very same
//! set of paths ([`branches`]).
//!
//! `end` is exempt: it is not a node, it retires branches instead of running,
//! and branches legitimately reach it at different depths (grammar 7.6.3).
//!
//! **One diagnostic per convergence of one fork, not per pair that unbalances
//! it.** A pair is read at the *nearest* convergence it unbalances, because
//! every node downstream of that one inherits the same skew. That bounds the
//! report *within* a pair and not across the pairs of one fork, and a fork whose
//! arms reach one node at differing depths unbalances it once per pair: an
//! ordinary ten-way parallel fan is 33 error blocks, twenty arms 133, a hundred
//! and twenty 4,800 — every one of them the same sentence about the same node at
//! the same span, differing only in which two of the arms it underlines. What
//! the diagnostic is *about* is the convergence and the fork it names, so that
//! is what it is reported per, drawn through the two edges that come first in
//! declaration order — the same policy the concurrent-write rule below states
//! for its own quadratic, and the same reason (PRD G3). Two forks that unbalance
//! one node stay two diagnostics: they are two mistakes and two edits, and the
//! sentence names the fork.
//!
//! The rule itself is untouched by that. Co-takeability and `dist` are stated
//! over pairs and stay stated over pairs (Decision D99): every pair is still
//! compared, because a pair a later one repeats is still what proves the skew,
//! and a shape reported through one pair is reported at all only because some
//! pair unbalances it. What the count bounds is the report.
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
//! **What a branch holds is read over control transfers, not over edges.** The
//! two rules on this page read a fork's branches over two different relations,
//! because grammar 7.6 states them that way: `dist` above "counts **edges**
//! only" and says why, while 7.6.1's "reachable" carries no qualifier and the
//! two control-transfer positions "schedule a node exactly as an edge does"
//! (grammar 7.8, 9.2). A node reached only by `on_error: { fallback: … }` or
//! `human.on_timeout:` runs on the branch of the node that failed over to it,
//! while the sibling branch runs on — concurrent branches are never cancelled
//! (grammar 7.6.3) — so it races that sibling's writers exactly as an
//! edge-reached node would. Both halves of the definition are read over the one
//! relation ([`Graph::reachable_through`], [`Graph::reaches`]): the same
//! transfer that puts a fallback target on its predecessor's branch is ordinarily
//! what makes the two of them sequential rather than concurrent — with the two
//! exceptions the next paragraph is about.
//!
//! **"Reachable from the other" has to mean "runs after the other".** That is
//! what the clause is for, and reachability says it only where the reaching
//! relation is the whole story of when the second node runs. Two shapes where it
//! is not, and both are races the rule has to keep:
//!
//! * *The pair's own two entries* ([`entries_race`]). Grammar 7.6's step rule puts
//!   the targets of every edge taken in step *k* into step *k+1* together, so a
//!   co-takeable pair that fires both ways runs its two entries in one and the
//!   same step. Naming one of them as the other's `on_error: { fallback: … }` or
//!   `human.on_timeout:` target adds a *second* run of it and moves nothing, so
//!   that one pair is not asked the question.
//! * *Two nodes that reach each other* ([`sequential`]). That is a loop rather
//!   than an order: neither runs after the other, both are re-entered on every
//!   pass, and their arrivals interleave. Grammar 7.6.2 says outright that a
//!   distance across a cycle is not static and hands that case to the runtime
//!   rule — of which grammar 10.2 has none, an unreduced channel being a
//!   compile-time promise of one writer per step (7.6.4) — so mutual
//!   reachability reads as concurrent. Without it a fan inside a bounded review
//!   loop falls between the two rules: the back edge makes every node of one
//!   branch reach every node of the other, while balanced convergence is
//!   (correctly) silent for a cyclic node.
//!
//! The analysis is deliberately conservative: a guard pair it cannot prove
//! exclusive is co-takeable, so it may ask for a policy on a channel two
//! branches could not really both write. Declaring the policy is the cost, and
//! D32 already holds that a declared overwrite beats a silent race.
//!
//! **One diagnostic per channel, not per pair of writers that race it.** A fan
//! of *w* branches each writing one unreduced channel holds w(w-1)/2 racing
//! pairs and exactly one mistake — the missing `reduce:` on that one
//! declaration — so a report of the cross is a report of the same edit w(w-1)/2
//! times over: 190 errors at twenty branches, 7,140 at a hundred and twenty.
//! The channel is what the diagnostic is about and what the fix touches, so the
//! channel is what it is reported per, named through the first two nodes in
//! declaration order that race it — the same policy the balanced-convergence
//! rule states above for its own quadratic, and the same reason (PRD G3).
//! Choosing that pair by declaration order rather than by which fork the walk
//! reached first is what keeps the choice stable: adding an unrelated fork
//! elsewhere in the flow moves no diagnostic.
//!
//! # What a pair costs
//!
//! Both rules are stated over pairs, and a fork of *w* out-edges has w(w-1)/2 of
//! them — so a pair has to cost almost nothing. Four things keep it there, and
//! the first three are the same observation: **a pair is read only through the
//! two nodes its edges deliver to**
//! ([`Graph::entry`](super::graph::Graph::entry)). The fourth is its twin one
//! step earlier — what decides whether a pair *is* a pair is read only through
//! the two edges themselves.
//!
//! 1. *One walk per entry.* The step distances a branch delivers at
//!    (grammar 7.6.2) and the nodes it holds (grammar 7.6.1) are properties of
//!    where the edge lands, so both are taken once per entry node and paired out
//!    of a cache — not once per edge, and never once per pair. A layer of ten
//!    nodes each edging to the same ten is ten walks, not a hundred. Each walk
//!    is proportional to the branch as well: the rule reads a node's distances
//!    only for whether they are one value and which, so a branch is carried as
//!    the nearest and farthest arrival at each node it reaches
//!    ([`Delivery`](super::graph::Delivery)) rather than as the paths that get
//!    there, of which there may be one per node in the branch.
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
//! 4. *One guard read per edge* ([`possible`]). Grammar 7.6.1's rule 2 asks what
//!    each guard leaves possible for each enum-typed field of the fork's output,
//!    and that is a property of the guard rather than of the pair: read per pair
//!    instead, every guard's AST is walked and its variant sets rebuilt *w*
//!    times over. It is read once per edge, and a pair costs the disjointness
//!    test alone, over sets bounded by the field's declared variants.
//!
//! A wide fan-out is a shape the grammar admits — nothing bounds a node's
//! out-degree, and `start` forks like any other vertex — while `validate` is a
//! millisecond-budget command (PRD 5.12); this is the one shape that could spend
//! that budget, and `tests/check_scale.rs` is where the bound is held.

use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};

use crate::diag::{Diagnostic, DiagnosticCode, Span};
use crate::ir::Channel;
use crate::ir::flow::{MapDispatch, Node, NodeKind};
use crate::ir::schema::TypeForm;

use super::channels::Written;
use super::graph::{Delivery, Graph, Vertex};
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
        let possible = possible(ctx, graph, source, outgoing);
        for left in 0..outgoing.len() {
            for right in (left + 1)..outgoing.len() {
                if exclusive(graph, outgoing, &possible, left, right) {
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

/// What each out-edge of one vertex leaves **possible** for each enum-typed
/// field of that vertex's output (grammar 7.6.1 rule 2, 7.3.1), in one order —
/// `None` for an edge rule 2 cannot read at all, whose exclusivity is rule 1's
/// question alone.
///
/// Read once per **edge**, not once per pair. What a guard admits is a property
/// of that guard, while the rule that consumes it is stated over pairs and a
/// fork of `w` out-edges has w(w-1)/2 of them — so a guard read per pair is a
/// guard walked `w` times over, for every enum field, and the variant sets it
/// builds are rebuilt with it. A 240-branch guarded fork over an output of five
/// enum fields cost 10.2 s of `check` that way against 1.06 s this way, on a
/// composition with nothing wrong with it and against a command whose budget is
/// milliseconds (PRD 5.12). What a pair costs is then the disjointness test
/// alone, over sets bounded by the field's declared variants
/// (`tests/check_scale.rs`).
fn possible<'a>(
    ctx: &Ctx<'a>,
    graph: &Graph<'a>,
    source: Vertex,
    outgoing: &[usize],
) -> Vec<Option<Vec<BTreeSet<String>>>> {
    let enums = source_enums(ctx, graph, source);
    // `start` has no output for rule 2 to read (grammar 2.4), and neither has a
    // node whose output declares no enum-typed field: nothing to be disjoint
    // over, so no guard is worth parsing.
    let Vertex::Node(at) = source else {
        return vec![None; outgoing.len()];
    };
    if enums.is_empty() {
        return vec![None; outgoing.len()];
    }
    let id = graph.id(at);
    outgoing
        .iter()
        .map(|edge| {
            let guard = graph.edge(*edge).when.as_ref()?;
            // A guard the CEL front-end rejected proves nothing about which
            // variants it admits, and reading it would turn one mistake into a
            // second, unrelated diagnostic downstream.
            let program = ctx
                .guard_type_checked(&guard.span)
                .then(|| guards::parse(guard.value.as_str()))
                .flatten()?;
            Some(
                enums
                    .iter()
                    .map(|(field, variants)| {
                        guards::coverage(program.expression(), id, field, variants).possible
                    })
                    .collect(),
            )
        })
        .collect()
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
    outgoing: &[usize],
    possible: &[Option<Vec<BTreeSet<String>>>],
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
    // Rule 2: two guards no enum value satisfies at once. Both lists carry one
    // entry per enum-typed field of the source's output, in one order
    // ([`possible`]), so the fields are compared by walking them together.
    let (Some(one), Some(other)) = (&possible[left], &possible[right]) else {
        return false;
    };
    one.iter()
        .zip(other)
        .any(|(one, other)| one.is_disjoint(other))
}

/// The two nodes a co-takeable pair starts a branch at, or `None` where it
/// starts fewer than two.
///
/// Both rules read a pair only through this, and there are two ways a pair has
/// no second branch for them to compare. An edge that retires its branch at
/// `end` delivers to no node, and `end` is exempt from both rules anyway
/// (grammar 7.6.2, 7.6.3). And two edges that land on the **same** node deliver
/// one branch rather than two: grammar 7.6's P2 runs a node targeted by several
/// edges taken in one step exactly once, so the pair schedules a single
/// instance, which then leaves by one of *its* own out-edges.
///
/// Comparing that branch with itself is exactly the comparison inside one side
/// that Decision D112 refuses, arrived at by a different route — the two
/// distance sets are not merely overlapping but identical, so any node the
/// branch reaches at two depths is reported as an unbalanced convergence, with
/// a diagnostic naming two edges that share a target and claiming they arrive at
/// different depths. The same skew read at the node where the branch really
/// parts is the question grammar 7.6.2 puts there, and this check asks it of
/// that node's own pair. Grammar 10.2's half over-rejects the same way: two
/// nodes one branch reaches are concurrent only if something *inside* it forks,
/// which is that fork's pair to answer for, not this one's.
fn branches(graph: &Graph<'_>, pair: &Pair) -> Option<(usize, usize)> {
    let (near, far) = (graph.entry(pair.edges.0)?, graph.entry(pair.edges.1)?);
    (near != far).then_some((near, far))
}

/// Grammar 7.6.2: the two edges of a pair may not deliver to one node at two
/// different depths.
fn balanced<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, pairs: &[Pair]) {
    let mut distances: BTreeMap<usize, Vec<Delivery>> = BTreeMap::new();
    let mut skew: BTreeMap<(usize, usize), Skew> = BTreeMap::new();
    // One diagnostic per convergence of one fork rather than per pair that
    // unbalances it: a fork of `w` arms landing on one node at differing depths
    // holds up to w(w-1)/2 of those pairs and one mistake, and the sentence they
    // all carry is the same one (see this module's header).
    let mut reported: BTreeSet<(Vertex, usize)> = BTreeSet::new();
    for pair in pairs {
        let (left, right) = pair.edges;
        let Some((near, far)) = branches(graph, pair) else {
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
        // The pairs of one fork are enumerated in edge declaration order
        // ([`pairs`]), so the first to reach a convergence is the one the report
        // draws through — a choice that does not move when an unrelated arm is
        // added elsewhere on the fork.
        if !reported.insert((pair.source, node)) {
            continue;
        }
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
///
/// Both lists are ascending by node ([`Graph::distances`]), so the nodes the two
/// branches share are found by walking them side by side — what the pairing
/// costs is what the two branches hold, not what the flow holds.
fn nearest(near: &[Delivery], far: &[Delivery]) -> Skew {
    let (mut left, mut right) = (0, 0);
    let mut nearest: Option<(usize, usize, usize, usize)> = None;
    while left < near.len() && right < far.len() {
        let (one, other) = (near[left], far[right]);
        match one.node.cmp(&other.node) {
            Ordering::Less => left += 1,
            Ordering::Greater => right += 1,
            Ordering::Equal => {
                left += 1;
                right += 1;
                let Some((first, second)) = differ(one, other) else {
                    continue;
                };
                // Ascending node index, so a tie on the distance keeps the
                // earlier node — the order a scan of every node would have found
                // them in.
                let key = first.min(second);
                if nearest.as_ref().is_none_or(|(best, ..)| key < *best) {
                    nearest = Some((key, one.node, first, second));
                }
            }
        }
    }
    nearest.map(|(_, node, one, other)| (node, one, other))
}

/// One distance from each side that differ — never a comparison inside one side
/// (Decision D112) — or `None` where every path on either side arrives in the
/// same step.
///
/// Two non-empty sets of distances hold no differing pair only when both are the
/// same single value, so the nearest and farthest of each side decide it and
/// name it. The pair returned is left-side-first, matching the two edges the
/// diagnostic labels, and its smaller half is always the shallower of the two
/// arrivals — the step the convergence first runs at, which is what orders the
/// choice of *which* convergence to report.
fn differ(one: Delivery, other: Delivery) -> Option<(usize, usize)> {
    if one.nearest != other.nearest {
        return Some((one.nearest, other.nearest));
    }
    if one.farthest != one.nearest {
        return Some((one.farthest, other.nearest));
    }
    (other.farthest != other.nearest).then_some((one.nearest, other.farthest))
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
    let mut held: BTreeMap<usize, Branch> = BTreeMap::new();
    let mut crossed: BTreeSet<(usize, usize)> = BTreeSet::new();
    // One entry per raced channel rather than per racing pair: the fix is the
    // `reduce:` on that one declaration, and the pairs are w(w-1)/2 of them.
    let mut raced: BTreeMap<&'a str, Race<'a>> = BTreeMap::new();
    // One bit per ordered pair of *writers*, so a pair is decided once for the
    // whole flow however many forks separate it.
    let mut examined = vec![0u64; writers.count() * writers.words()];
    for pair in pairs {
        let Some((near, far)) = branches(graph, pair) else {
            continue;
        };
        // What a branch holds depends on the pair only through the node it
        // delivers to, so repeated branches are crossed once.
        if !crossed.insert((near.min(far), near.max(far))) {
            continue;
        }
        entries_race(&mut raced, &writers, near, far);
        for entry in [near, far] {
            held.entry(entry)
                .or_insert_with(|| writers.branch(graph, entry));
        }
        for one in &held[&near].members {
            let one = *one;
            let row = one * writers.words();
            for word in 0..writers.words() {
                // The writers of the far branch this one has never been paired
                // with. Every bit left standing is a pair decided for the first
                // and last time, so the scan costs a word per branch and the
                // decisions cost what the answer holds.
                let mut fresh = held[&far].mask[word] & !examined[row + word];
                while fresh != 0 {
                    let other = word * 64 + fresh.trailing_zeros() as usize;
                    fresh &= fresh - 1;
                    examined[row + word] |= 1 << (other % 64);
                    examined[other * writers.words() + one / 64] |= 1 << (one % 64);
                    if one == other {
                        continue;
                    }
                    let (one, other) = writers.order(one, other);
                    if sequential(graph, writers.node(one), writers.node(other)) {
                        continue;
                    }
                    races(&mut raced, &writers, one, other);
                }
            }
        }
    }
    for (name, race) in raced {
        report(ctx, cx, graph, &writers, name, &race);
    }
}

/// Record what the pair's own two entries race for: the nodes its edges deliver
/// to, which the **fork** schedules rather than either branch.
///
/// Grammar 7.6's step rule puts the targets of every edge taken in step *k* into
/// step *k+1* together, so a co-takeable pair that fires both ways runs both of
/// these in one and the same step — and "neither is reachable from the other"
/// cannot say otherwise about them. A transfer from one branch to the other's
/// entry adds a *second* run of that entry; it does not move the first, and the
/// sibling branch runs on regardless because concurrent branches are never
/// cancelled (grammar 7.6.3). So this one pair is decided by the pair alone,
/// outside the reachability test [`sequential`] applies to the rest of the
/// cross, and outside the examined-pair matrix, which is keyed by the writers
/// and cannot see which fork put them side by side.
fn entries_race<'a>(
    raced: &mut BTreeMap<&'a str, Race<'a>>,
    writers: &Writers<'a>,
    near: usize,
    far: usize,
) {
    let (Some(one), Some(other)) = (writers.at(near), writers.at(far)) else {
        return;
    };
    let (one, other) = writers.order(one, other);
    races(raced, writers, one, other);
}

/// Whether one node runs strictly **after** the other, which is what takes a
/// pair out of grammar 7.6.1's rule.
///
/// A node downstream of both branches runs after them rather than beside them,
/// and so does a node its predecessor only ever fails over to: the
/// control-transfer relation holds one way and not the other, and the same
/// transfer that puts a fallback target on its predecessor's branch is what
/// makes the two of them sequential.
///
/// Two nodes that reach **each other** are in a loop rather than in an order.
/// Neither runs after the other; both are re-entered on every pass of the cycle
/// they share, and their arrivals interleave — grammar 7.6.2 says outright that
/// a distance across a cycle is not static and leaves that case to the runtime
/// rule, of which grammar 10.2 has none. So a cycle reads as concurrent, and a
/// fan inside a bounded review loop is analysed rather than falling between the
/// two rules: 7.6.2 is silent for a cyclic node by construction
/// ([`Graph::distances`](super::graph::Graph::distances)).
fn sequential(graph: &Graph<'_>, one: usize, other: usize) -> bool {
    graph.reaches(one, other) != graph.reaches(other, one)
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

    /// The writer one node is, or `None` where it writes nothing.
    fn at(&self, node: usize) -> Option<usize> {
        self.slot[node]
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

/// The writers one branch holds (grammar 7.6.1), both ways round.
struct Branch {
    /// Their writer indexes, ascending.
    members: Vec<usize>,
    /// The same set as a bitset over writer indexes.
    mask: Vec<u64>,
}

/// The pair of writers one raced channel is reported through.
struct Race<'a> {
    /// The two of them, ascending by node — the order the message names them
    /// in ([`Writers::order`]).
    writers: (usize, usize),
    /// Where each write is: the `writes:` entry that makes it, or the node
    /// itself for a name-based one ([`Written::at`]), in the same order.
    sites: (Span, Span),
    /// The channel they both write.
    channel: &'a Channel,
}

/// Record the channels two concurrent nodes both write with no `reduce:`
/// policy.
///
/// A channel is kept once, through the racing pair whose nodes come first in
/// declaration order: a fan of *w* branches writing it races w(w-1)/2 ways and
/// is one missing `reduce:` (see this module's header). The rest of the cross is
/// still walked — every pair has to be *decided* to know the channel is raced at
/// all — and what the count bounds is the report.
fn races<'a>(
    raced: &mut BTreeMap<&'a str, Race<'a>>,
    writers: &Writers<'a>,
    one: usize,
    other: usize,
) {
    for first in writers.writes(one) {
        for second in writers.writes(other) {
            let name = first.channel.name.value.as_str();
            if second.channel.name.value.as_str() != name || first.channel.reduce.is_some() {
                continue;
            }
            let earlier = raced.get(name).is_none_or(|best| {
                (writers.node(one), writers.node(other))
                    < (writers.node(best.writers.0), writers.node(best.writers.1))
            });
            if !earlier {
                continue;
            }
            raced.insert(
                name,
                Race {
                    writers: (one, other),
                    sites: (first.at.clone(), second.at.clone()),
                    channel: first.channel,
                },
            );
        }
    }
}

/// The one diagnostic a raced channel gets (grammar 7.6.1, 10.2).
fn report<'a>(
    ctx: &mut Ctx<'a>,
    cx: &FlowCx<'a>,
    graph: &Graph<'a>,
    writers: &Writers<'a>,
    name: &str,
    race: &Race<'a>,
) {
    let (one, other) = race.writers;
    let (first, second) = &race.sites;
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::UnreducedWrite,
            second.clone(),
            format!(
                "nodes `{}` and `{}` of `{}` run concurrently and both write the unreduced channel `{name}`",
                graph.id(writers.node(one)),
                graph.id(writers.node(other)),
                cx.address
            ),
        )
        .with_label(first.clone(), "the other write is here")
        .with_label(race.channel.span.clone(), "the channel is declared here")
        .with_help(
            "two edges of one fork that are not provably exclusive can both fire, so the branches they start are concurrent: the channel they both write needs a declared `reduce:` policy — `append`, `merge`, or an explicit `last_wins` (grammar 7.6.1, 10.2, Decision D32)",
        ),
    );
}

/// Every channel one node writes: its effective write map, or — for a `map`
/// node, which has no output of its own — the effective write map of each
/// dispatch it issues (grammar 8.0, 8.6 rule 9).
fn written<'a>(ctx: &Ctx<'a>, node: &'a Node) -> Vec<Written<'a>> {
    // The node's id, not its block: the same anchor [`channels::node_writes`]
    // reports its own diagnostics at, for the same reason — a block mapping ends
    // where the next node's key begins, so a name-based write anchored on the
    // block draws this rule's "the other write is here" label across the
    // *following* node's declaration, which is not a party to the diagnostic
    // (PRD G3). A `map` node's `map:` block and each of its route blocks end the
    // same way and take the same anchor: the message compares two node names, so
    // both of its annotations land on one.
    let at = &node.id.span;
    let NodeKind::Map { map } = &node.kind else {
        return ctx.node_output(node).map_or_else(Vec::new, |output| {
            channels::effective(ctx, &output, node.writes.as_ref(), at)
        });
    };
    let sites: Vec<(
        &crate::ast::common::Address,
        Option<&crate::ir::binding::Writes>,
    )> = match &map.dispatch {
        MapDispatch::Homogeneous {
            node: target,
            writes,
            ..
        } => vec![(&target.value, writes.as_ref())],
        MapDispatch::Routed {
            routes, default, ..
        } => routes
            .iter()
            .chain(default.iter().map(|route| &**route))
            .map(|route| (&route.node.value, route.writes.as_ref()))
            .collect(),
    };
    sites
        .into_iter()
        .filter_map(|(target, writes)| {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn at(node: usize, nearest: usize, farthest: usize) -> Delivery {
        Delivery {
            node,
            nearest,
            farthest,
        }
    }

    /// Grammar 7.6.2 asks whether *some* distance on one side differs from some
    /// distance on the other, and Decision D112 forbids answering it from one
    /// side alone. Both sides delivering at one and the same step is the whole of
    /// balanced — and the two extremes decide it, whatever lies between them.
    #[test]
    fn two_sides_differ_unless_both_arrive_at_one_step() {
        assert_eq!(differ(at(0, 2, 2), at(0, 2, 2)), None, "one step each");
        assert_eq!(
            differ(at(0, 4, 4), at(0, 4, 4)),
            None,
            "one deeper step each"
        );
        assert_eq!(
            differ(at(0, 1, 1), at(0, 2, 2)),
            Some((1, 2)),
            "two singletons that differ"
        );
        // A side that arrives at several depths disagrees with a side that
        // arrives at one of them, because the others are arrivals too.
        assert_eq!(differ(at(0, 2, 3), at(0, 2, 2)), Some((3, 2)));
        assert_eq!(differ(at(0, 2, 2), at(0, 2, 3)), Some((2, 3)));
        // Every value named is one its own side really delivers at.
        for (one, other) in [
            (at(0, 2, 5), at(0, 2, 7)),
            (at(0, 3, 3), at(0, 1, 9)),
            (at(0, 1, 4), at(0, 6, 6)),
        ] {
            let (first, second) = differ(one, other).expect("the two sides differ");
            assert!((one.nearest..=one.farthest).contains(&first));
            assert!((other.nearest..=other.farthest).contains(&second));
            assert_ne!(first, second);
        }
    }

    /// A pair is read at the nearest convergence it unbalances: every node
    /// downstream inherits the same skew, and a page of diagnostics for one
    /// mistake is not a diagnostic (PRD G3). "Nearest" is the shallower of the
    /// two arrivals, and a tie on it keeps the earlier node.
    #[test]
    fn the_shallowest_unbalanced_convergence_is_the_one_reported() {
        let near = [at(1, 1, 1), at(3, 4, 4), at(7, 2, 2)];
        let far = [at(1, 1, 1), at(3, 9, 9), at(7, 3, 3)];
        assert_eq!(
            nearest(&near, &far),
            Some((7, 2, 3)),
            "node 7 is reached at step 2, node 3 not before step 4"
        );

        // A tie keeps the earlier node.
        let near = [at(2, 5, 5), at(4, 5, 5)];
        let far = [at(2, 6, 6), at(4, 8, 8)];
        assert_eq!(nearest(&near, &far), Some((2, 5, 6)));

        // Nodes only one side reaches are not this pair's error: both sides have
        // to supply a distance (grammar 7.6.2).
        assert_eq!(nearest(&[at(1, 1, 1)], &[at(2, 9, 9)]), None);
        assert_eq!(nearest(&[at(1, 1, 3)], &[]), None);
        assert_eq!(nearest(&[], &[]), None);
    }
}
