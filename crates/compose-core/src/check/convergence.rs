//! Forks, co-takeable pairs, and the two rules stated over them: balanced
//! convergence and concurrent writes (grammar 7.6.1, 7.6.2, 10.2).
//!
//! # Co-takeability is a relation on a pair
//!
//! Two outgoing edges of one vertex are **exclusive** when the validator can
//! prove they are never taken together — one carries `else: true` and the other
//! `when:` (grammar 7.3 rule 4 makes those mutually exclusive by construction);
//! or both carry `when:` guards whose `possible` sets are disjoint for some
//! enum-typed field of the source node's output ([`guards`](super::guards), read
//! for disjointness instead of coverage); or both carry `when:` guards, one of
//! which holds a `bool`-typed read `q` as a top-level conjunct and the other
//! `!q` — `a.output.ok && input.x` and `a.output.ok && !input.x` — read off the
//! same parse ([`guards::literals`](super::guards::literals), Decision D169).
//! Every other pair is **co-takeable**, and an edge is never co-takeable on its
//! own: it is one or the other *with respect to a named sibling* (Decision
//! D99).
//!
//! A vertex with a co-takeable pair is a **fork**. `start` is one too — grammar
//! 7.6's step 0 runs the nodes its taken edges target, and grammar 7.3 rule 7
//! says outright that routing applies to `start` as well — but rule 2 never
//! proves its edges exclusive, because `start` has no output for it to read
//! (grammar 2.4). Rules 1 and 3 read `start`'s edges as they read any other's:
//! a guard leaving `start` reads `input` and `state`, and `input.x` and
//! `!input.x` are exclusive wherever they are written.
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
//! 4. *One guard read per edge* ([`readings`]). Grammar 7.6.1's rule 2 asks what
//!    each guard leaves possible for each enum-typed field of the fork's output,
//!    and rule 3 which `bool`-typed reads its conjuncts hold or negate; both are
//!    properties of the guard rather than of the pair: read per pair instead,
//!    every guard's AST is walked and its sets rebuilt *w* times over. It is
//!    parsed once per edge for both rules, and a pair costs the disjointness
//!    test over sets bounded by the field's declared variants, plus one lookup
//!    per literal. Rule 3 needs a `!` on one side, so a fork whose guards write
//!    none — and whose output has no enum for rule 2 — parses nothing at all.
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
///
/// Opaque outside this module: what a caller does with these is ask
/// [`concurrent_among`] a question about them, and the enumeration itself is
/// this file's business (grammar 7.6.1).
pub(crate) struct Pair {
    /// The fork they leave.
    source: Vertex,
    /// The two edge indexes, in declaration order.
    edges: (usize, usize),
}

/// Check one flow's forks, and hand back the co-takeable pairs they came from.
///
/// The pairs are returned rather than dropped because the shared-workspace
/// warning asks a second question of the **same** enumeration
/// ([`concurrent_among`], PRD resolved q61 ruling b), and enumerating them again
/// would re-read every guard of every fork with it: [`readings`] is hoisted out
/// of the pair loop precisely because a 240-branch guarded fork costs 10.2 s the
/// naive way, and a second pass pays that cost over again for a rule that adds
/// no pair of its own. One enumeration per flow, two rules stated over it
/// (`tests/check_scale.rs`, PRD 5.12).
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) -> Vec<Pair> {
    let pairs = pairs(ctx, graph);
    balanced(ctx, cx, graph, &pairs);
    concurrent(ctx, cx, graph, &pairs);
    pairs
}

/// Which of a handful of nodes can run **concurrently** with which
/// (grammar 7.6.1).
///
/// [`concurrent`] below asks this question of every pair of *writers* and is
/// written for that scale — a bitset over the writers, an examined-pair matrix,
/// one cross per branch pair. This answers it for a named few instead, and the
/// caller is PRD resolved q61 ruling b's second half: two `coder:` nodes that
/// may be in flight at once and name one workspace are two harnesses in one
/// directory. A flow has a handful of coder nodes and no bound on its writers,
/// so the two callers want different shapes of the same relation, and the
/// relation is stated once — here, beside the rule it was written for.
///
/// The pairs come back keyed by node index, ascending, which is the order a
/// diagnostic names two nodes in whichever branch each was found in.
///
/// The fork enumeration is the caller's to supply, and it is [`check`]'s own:
/// this rule reads the co-takeable pairs and adds none, so re-deriving them here
/// would re-read every guard of every fork for nothing
/// (`tests/check_scale.rs`).
pub(crate) fn concurrent_among(
    graph: &Graph<'_>,
    pairs: &[Pair],
    of_interest: &[usize],
) -> BTreeSet<(usize, usize)> {
    let mut found = BTreeSet::new();
    if of_interest.len() < 2 {
        return found;
    }
    let interest: BTreeSet<usize> = of_interest.iter().copied().collect();
    let mut held: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    let mut crossed: BTreeSet<(usize, usize)> = BTreeSet::new();
    for pair in pairs {
        let Some((near, far)) = branches(graph, pair) else {
            continue;
        };
        if !crossed.insert((near.min(far), near.max(far))) {
            continue;
        }
        // The two **entries** run in one step whatever else is true of them,
        // which is [`entries_race`]'s rule read for this relation: grammar
        // 7.6's step rule puts the targets of every edge taken in step *k* into
        // step *k+1* together, so the reachability test below cannot say
        // otherwise about the pair the fork itself scheduled.
        if interest.contains(&near) && interest.contains(&far) {
            found.insert((near.min(far), near.max(far)));
        }
        for entry in [near, far] {
            held.entry(entry).or_insert_with(|| {
                graph
                    .reachable_through(entry)
                    .into_iter()
                    .filter(|node| interest.contains(node))
                    .collect()
            });
        }
        for one in &held[&near] {
            for other in &held[&far] {
                if one == other || sequential(graph, *one, *other) {
                    continue;
                }
                found.insert(((*one).min(*other), (*one).max(*other)));
            }
        }
    }
    found
}

/// Every co-takeable pair of every fork (grammar 7.6.1, Decision D99).
fn pairs<'a>(ctx: &Ctx<'a>, graph: &Graph<'a>) -> Vec<Pair> {
    let mut pairs = Vec::new();
    for source in graph.sources() {
        let outgoing = graph.outgoing(source);
        if outgoing.len() < 2 {
            continue;
        }
        let readings = readings(ctx, graph, source, outgoing);
        for left in 0..outgoing.len() {
            for right in (left + 1)..outgoing.len() {
                if exclusive(graph, outgoing, &readings, left, right) {
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

/// What one out-edge's guard says about exclusivity, read once (grammar 7.6.1).
#[derive(Clone, Default)]
struct Reading {
    /// What the guard leaves **possible** for each enum-typed field of the
    /// fork's output, in one order (rule 2, 7.3.1) — `None` where rule 2 cannot
    /// read the edge at all.
    possible: Option<Vec<BTreeSet<String>>>,
    /// The guard's conjuncts that are a `bool`-typed read or its negation
    /// (rule 3, Decision D169) — empty where rule 3 cannot read the edge.
    literals: BTreeSet<guards::Literal>,
}

/// What each out-edge of one vertex says for rules 2 and 3, in one order: what
/// its guard leaves possible for each enum-typed field of the vertex's output
/// (grammar 7.6.1 rule 2, 7.3.1), and the `bool`-typed reads its conjuncts hold
/// or negate (rule 3, Decision D169). An edge with no guard, or one the CEL
/// front-end rejected, reads as nothing, and its exclusivity is rule 1's
/// question alone.
///
/// Read once per **edge**, not once per pair. What a guard admits is a property
/// of that guard, while the rules that consume it are stated over pairs and a
/// fork of `w` out-edges has w(w-1)/2 of them — so a guard read per pair is a
/// guard walked `w` times over, for every enum field, and the variant sets it
/// builds are rebuilt with it. A 240-branch guarded fork over an output of five
/// enum fields cost 10.2 s of `check` that way against 1.06 s this way, on a
/// composition with nothing wrong with it and against a command whose budget is
/// milliseconds (PRD 5.12). What a pair costs is then the disjointness test
/// alone, over sets bounded by the field's declared variants, and a lookup of
/// each of one side's literals in the other's (`tests/check_scale.rs`). Each
/// guard is parsed once for both rules.
fn readings<'a>(
    ctx: &Ctx<'a>,
    graph: &Graph<'a>,
    source: Vertex,
    outgoing: &[usize],
) -> Vec<Reading> {
    // `start` has no output for rule 2 to read (grammar 2.4), and neither has a
    // node whose output declares no enum-typed field: nothing to be disjoint
    // over for that rule.
    let enums = source_enums(ctx, graph, source);
    let id = match source {
        Vertex::Node(at) if !enums.is_empty() => Some(graph.id(at)),
        _ => None,
    };
    // Rule 3 needs a negation on one side of the pair, so a fork none of whose
    // guards writes a `!` has no pair it can prove — and then, with no enum for
    // rule 2 either, no guard of it is worth parsing.
    let negates = outgoing.iter().any(|edge| {
        graph
            .edge(*edge)
            .when
            .as_ref()
            .is_some_and(|guard| guard.value.as_str().contains('!'))
    });
    if id.is_none() && !negates {
        return vec![Reading::default(); outgoing.len()];
    }
    outgoing
        .iter()
        .map(|edge| {
            let Some(guard) = graph.edge(*edge).when.as_ref() else {
                return Reading::default();
            };
            // A guard the CEL front-end rejected proves nothing about which
            // variants it admits or which reads it holds, and reading it would
            // turn one mistake into a second, unrelated diagnostic downstream.
            let Some(program) = ctx
                .guard_type_checked(&guard.span)
                .then(|| guards::parse(guard.value.as_str()))
                .flatten()
            else {
                return Reading::default();
            };
            let possible = id.map(|id| {
                enums
                    .iter()
                    .map(|(field, variants)| {
                        guards::coverage(program.expression(), id, field, variants).possible
                    })
                    .collect()
            });
            let literals = if negates {
                literals_of(
                    guard.value.as_str(),
                    program.expression(),
                    ctx.guard_booleans(&guard.span),
                )
            } else {
                BTreeSet::new()
            };
            Reading { possible, literals }
        })
        .collect()
}

/// Rule 3's reading of one guard: its conjuncts that are a `bool`-typed read or
/// the negation of one ([`guards::literals`], Decision D169), with "`bool`-typed"
/// answered by the reads the CEL front-end's walk of this guard typed `bool`.
///
/// The pinned parser reads `!!q` as `!q` ([`crate::cel::repeats_a_negation`]),
/// so a guard that writes one carries a negation in its AST that is not the one
/// in its source — and `!!q` against `q` would read as exclusive where the two
/// are the same guard. Such a guard proves nothing under rule 3.
fn literals_of(
    source: &str,
    expression: &::cel::common::ast::IdedExpr,
    booleans: &[crate::cel::Read],
) -> BTreeSet<guards::Literal> {
    if crate::cel::repeats_a_negation(source) {
        return BTreeSet::new();
    }
    guards::literals(expression, &|root, path| {
        booleans
            .iter()
            .any(|read| !read.indexed && read.root == root && read.path == path)
    })
}

/// Rule 3 over two guards' literals: one holds a read the other negates.
fn negate_each_other(one: &BTreeSet<guards::Literal>, other: &BTreeSet<guards::Literal>) -> bool {
    one.iter()
        .any(|literal| other.contains(&literal.negation()))
}

/// The enum-typed fields of a fork's output, with their variant sets — what
/// rule 2 reads. `start` has no output, so rule 2 never applies to its edges;
/// rules 1 and 3 do, as they do to any other vertex's (the module header).
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

/// Grammar 7.6.1's three exclusivity rules.
///
/// This is the one place a pair of sibling edges is decided exclusive, and
/// everything that asks whether two branches can both run asks it through here:
/// [`pairs`] keeps only the pairs this answers `false` for, and balanced
/// convergence, the concurrent-writer rule and [`concurrent_among`]'s
/// shared-workspace question all read those pairs and nothing else.
fn exclusive(
    graph: &Graph<'_>,
    outgoing: &[usize],
    readings: &[Reading],
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
    let (one, other) = (&readings[left], &readings[right]);
    // Rule 2: two guards no enum value satisfies at once. Both lists carry one
    // entry per enum-typed field of the source's output, in one order
    // ([`readings`]), so the fields are compared by walking them together.
    if let (Some(one), Some(other)) = (&one.possible, &other.possible)
        && one
            .iter()
            .zip(other)
            .any(|(one, other)| one.is_disjoint(other))
    {
        return true;
    }
    // Rule 3: one guard holds a `bool`-typed read as a conjunct and the other
    // holds its negation, so the two cannot both be true whatever else either
    // conjoins (Decision D169).
    negate_each_other(&one.literals, &other.literals)
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

    /// An edge guard's scope for rule 3's tests: a node `a` whose output holds a
    /// boolean, an integer and an optional string, and a flow input of two
    /// booleans and an integer.
    fn guard_scope() -> crate::cel::Scope {
        use crate::cel::{Origin, Property, Type};
        let property = |name: &str, ty: Type, optional: bool| Property {
            name: name.to_string(),
            ty,
            optional,
            defaulted: false,
        };
        let output = Type::object(
            Origin::Declared("`a`'s output".to_string()),
            vec![
                property("ok", Type::Bool, false),
                property("count", Type::Int, false),
                property("note", Type::String, true),
            ],
        );
        crate::cel::Scope::new(
            "an edge guard",
            vec![
                (
                    "a".to_string(),
                    Type::object(
                        Origin::Node {
                            id: "a".to_string(),
                            has_output: true,
                        },
                        vec![property("output", output, false)],
                    ),
                ),
                (
                    "input".to_string(),
                    Type::object(
                        Origin::Declared("`flow.f`'s inputs".to_string()),
                        vec![
                            property("x", Type::Bool, false),
                            property("y", Type::Bool, false),
                            property("n", Type::Int, false),
                        ],
                    ),
                ),
            ],
        )
    }

    /// Rule 3's reading of one guard, as [`readings`] takes it: the guard
    /// type-checks, and its literals are read with the `bool`-typed reads the
    /// front-end's walk found.
    #[track_caller]
    fn literals(guard: &str) -> BTreeSet<guards::Literal> {
        let analysis = crate::cel::analyze(guard, &guard_scope());
        assert!(
            analysis.problems.is_empty(),
            "`{guard}` should type-check: {:?}",
            analysis.problems
        );
        let program = guards::parse(guard).expect("the guard parses");
        literals_of(guard, program.expression(), &analysis.booleans)
    }

    #[track_caller]
    fn exclusive_by_rule_3(one: &str, other: &str) -> bool {
        let (one, other) = (literals(one), literals(other));
        let forward = negate_each_other(&one, &other);
        assert_eq!(
            forward,
            negate_each_other(&other, &one),
            "exclusivity is symmetric"
        );
        forward
    }

    /// The case PRD resolved q86 was raised for, and the rule's own shape: one
    /// guard holds a `bool`-typed read, the other its negation, whatever else
    /// either conjoins (grammar 7.6.1 rule 3, Decision D169).
    #[test]
    fn a_read_and_its_negation_make_two_guards_exclusive() {
        assert!(exclusive_by_rule_3(
            "a.output.ok && input.x",
            "a.output.ok && !input.x"
        ));
        // Either side may carry the negation, and the shared read may be the
        // only conjunct of either guard.
        assert!(exclusive_by_rule_3("!a.output.ok", "a.output.ok"));
        assert!(exclusive_by_rule_3(
            "input.x && a.output.count > 2",
            "!input.x && input.n == 0"
        ));
        // Two different reads are not the same read.
        assert!(!exclusive_by_rule_3("input.x", "!input.y"));
        assert!(!exclusive_by_rule_3(
            "input.x && input.y",
            "input.x && input.y"
        ));
    }

    /// The read is compared as the parse spells it, so whitespace and grouping —
    /// which the parse discards — change nothing, and neither does where in the
    /// conjunction the read sits.
    #[test]
    fn whitespace_and_parentheses_do_not_change_the_read() {
        for (one, other) in [
            ("a.output.ok && input.x", "a . output . ok&&!input.x"),
            ("a.output.ok && input.x", "!(input.x) && a.output.ok"),
            ("(a.output.ok && (input.y && input.x))", "( !input.x )"),
            (
                "input.x && (a.output.ok && input.y)",
                "input.y && !(input.x)",
            ),
            ("a.output.ok\n  && input.x", "! input.x"),
        ] {
            assert!(exclusive_by_rule_3(one, other), "`{one}` against `{other}`");
        }
    }

    /// `has()` is a `bool`-typed read in its own right — it answers whether the
    /// member is there — and is a different read from the member itself.
    #[test]
    fn a_presence_test_is_a_read_of_its_own() {
        assert!(exclusive_by_rule_3(
            "has(a.output.note) && a.output.ok",
            "!has(a.output.note)"
        ));
        assert!(exclusive_by_rule_3(
            "!has(a.output.note) && input.x",
            "a.output.ok && (has(a.output.note))"
        ));
        // The presence of a member and the member's value are two reads.
        assert!(!exclusive_by_rule_3("has(input.x)", "!input.x"));
        assert!(!exclusive_by_rule_3(
            "has(a.output.note)",
            "!has(a.output.ok)"
        ));
    }

    /// What the rule does not read, each of which the PRD's "a conjunct `q` and
    /// `!q`" leaves out: a comparison against a literal, a read under an `||`,
    /// an index rather than a selection, and a doubled negation — which the
    /// pinned parser collapses to a single one, so `!!input.x` would otherwise
    /// read as the negation of `input.x` it is the opposite of.
    #[test]
    fn the_rule_reads_a_conjunct_and_its_negation_and_nothing_wider() {
        for (one, other) in [
            ("input.x == true", "input.x == false"),
            ("input.x == true", "!input.x"),
            ("!input.x || a.output.ok", "input.x"),
            ("(!input.x || a.output.ok) && input.y", "input.x"),
            ("input['x']", "!input.x"),
            ("!!input.x", "!input.x"),
            ("!!input.x", "input.x"),
            ("! !input.x", "input.x"),
            ("!(!input.x)", "!input.x"),
            ("input.n > 0", "!(input.n > 0)"),
        ] {
            assert!(
                !exclusive_by_rule_3(one, other),
                "`{one}` against `{other}`"
            );
        }
        // A disjunction is one conjunct the rule does not read, and the rest of
        // the conjunction is still read: these two share `input.y`.
        assert!(exclusive_by_rule_3(
            "(!input.x || a.output.ok) && input.y",
            "input.x && !input.y"
        ));
    }

    /// Only a read whose checked type is `bool` counts. A guard that negates an
    /// integer does not type-check, so the case is a read the walk typed as
    /// something else — here, every read a scope does not type `bool`.
    #[test]
    fn a_read_that_is_not_bool_typed_is_not_a_literal() {
        let program = guards::parse("a.output.ok && input.x").expect("the guard parses");
        assert!(
            literals_of("a.output.ok && input.x", program.expression(), &[]).is_empty(),
            "with no read typed `bool`, nothing is a literal"
        );
        assert_eq!(literals("a.output.ok && input.x").len(), 2);
        assert_eq!(
            literals("has(a.output.note)").len(),
            1,
            "a presence test needs no type: it is `bool` by construction"
        );
    }

    /// The source test behind the doubled-negation case: a `!` applied to a `!`,
    /// whitespace and comments aside, and nothing that only looks like one.
    #[test]
    fn a_repeated_negation_is_told_apart_from_its_lookalikes() {
        for repeated in ["!!x", "! !x", "a && !!b", "!!!x", "!\n!x"] {
            assert!(crate::cel::repeats_a_negation(repeated), "{repeated}");
        }
        for single in [
            "!x",
            "!(!x)",
            "a != !b",
            "a != b",
            "!a && !b",
            "x == '!!'",
            "x == r'!!'",
        ] {
            assert!(!crate::cel::repeats_a_negation(single), "{single}");
        }
    }

    /// A `//` comment is hidden from the parser as whitespace is, so a run of
    /// `!` split by one is still one run to the pinned parser — `!` ending a
    /// commented line and `!input.x` on the next arrive as `!input.x` — and
    /// rule 3 has to see through it the same way, a quote inside the comment
    /// included. Each of these is `input.x` (or `input.x` conjoined), so reading
    /// any of them as `!input.x` would prove it exclusive with itself.
    #[test]
    fn a_repeated_negation_split_by_a_comment_proves_nothing() {
        for repeated in [
            "!// c\n!input.x",
            "a.output.ok && ! // not the slow path\n!input.x",
            "! // one\n// two\n! input.x",
            "! // \"\"\"\n!input.x && \"\"\"a\"\"\" == 'a'",
            "! // '\n!input.x",
        ] {
            assert!(crate::cel::repeats_a_negation(repeated), "{repeated:?}");
            assert!(
                !exclusive_by_rule_3(repeated, "a.output.ok && input.x"),
                "{repeated:?} against `a.output.ok && input.x`"
            );
        }
        // A comment that only follows a negation, or holds what looks like
        // one, splits no run — and the conjuncts around it are still read.
        for single in [
            "!input.x // !!",
            "!input.x // a note\n&& !input.y",
            "input.y == ('//' == '//') && !input.x",
            "'// !' == '// !' && !input.x",
        ] {
            assert!(!crate::cel::repeats_a_negation(single), "{single:?}");
            assert!(
                exclusive_by_rule_3(single, "input.x"),
                "{single:?} against `input.x`"
            );
        }
    }
}
