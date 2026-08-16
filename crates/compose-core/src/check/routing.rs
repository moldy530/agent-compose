//! Routing exhaustiveness, and the two no-dead-end rules stated over a node
//! (grammar 7.3.1, 7.6.3 rules 1 and 3).
//!
//! # Exhaustiveness
//!
//! PRD 5.3's flagship promise — an enum with three variants, a node with
//! guarded edges and an optional default, and a compile error if any variant is
//! unroutable — is decided over [`guards`](super::guards)' closed table rather
//! than over a satisfiability solver, so that two conforming validators accept
//! exactly the same compositions (Decision D82).
//!
//! The check **fires on mention**: a node is routing on an enum field `f` of its
//! output as soon as one of its guards names `<node>.output.<f>` syntactically,
//! whatever else that guard says. A sibling edge whose guard never mentions `f`
//! does not exempt the node — it simply contributes `∅` to `f`'s coverage. What
//! satisfies the check is either an outgoing edge that is unconditional or
//! carries `else: true`, which fires for every value of every field, or **one**
//! field whose variants the `guaranteed` sets cover between them. One field
//! suffices because edges are multicast (grammar 7.3 rule 6).
//!
//! A node whose guards did not type-check is not asked the question at all. The
//! coverage table reads an unrecognized term as `∅`, so a guard the CEL
//! front-end already rejected would otherwise be reported twice: once as the
//! mistake it is, and once as a variant it fails to cover (PRD G3 — one mistake,
//! one diagnostic).
//!
//! # No dead ends
//!
//! Grammar 7.6.3 states three rules and the parser owns the one decidable from
//! an `edges:` array alone — that some edge leaving `start` is guaranteed to
//! fire. The other two relate an edge to a *node*, so they are here:
//!
//! 1. **every node has an outgoing edge**, because a branch retires only at
//!    `end` and a node with no exit swallows it (rule 1);
//! 2. **a node declaring `on_error: skip` has an unconditional or `else: true`
//!    outgoing edge**, because a skipped node produces no output, so every guard
//!    that reads that output evaluates false and a node whose edges are all
//!    guarded has a pass on which none is taken (rule 3, Decision D97).
//!
//! Rule 3 is stated over the node's **own** `on_error:` key, which is where
//! grammar 7.6.3 and 9.2 both state it. The chain of grammar 9.3 can also
//! deliver `skip` from `defaults:` or from an instantiating `flow:` node's
//! `policy:`, and those levels are deliberately not read here: the rule names a
//! node that declares the strategy, and reading the chain would make one
//! `defaults:` block demand a guaranteed edge on every node of every flow in the
//! composition.

use std::collections::BTreeSet;

use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::policy::OnError;
use crate::ir::schema::TypeForm;

use super::graph::{Graph, Vertex};
use super::{Ctx, FlowCx, guards};

/// Check every node of one flow.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    for at in 0..graph.nodes().len() {
        if exits(ctx, cx, graph, at) {
            exhaustive(ctx, cx, graph, at);
        }
    }
}

/// Grammar 7.6.3 rules 1 and 3. Returns whether the node has any outgoing edge,
/// which is what makes a routing question worth asking of it.
fn exits<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, at: usize) -> bool {
    let node = graph.node(at);
    let id = graph.id(at);
    let outgoing = graph.outgoing(Vertex::Node(at));
    if outgoing.is_empty() {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::DeadEnd,
                node.id.span.clone(),
                format!("node `{id}` of `{}` has no outgoing edge", cx.address),
            )
            .with_help(
                "a branch retires only at `end`, so a node with no exit swallows it instead of reaching quiescence: `- { from: <node>, to: end }` is the one-line way to say \"this branch is done here\" (grammar 7.6.3 rule 1, Decision D71)",
            ),
        );
        return false;
    }
    // An edge carrying neither `when:` nor `else:` is unconditional and an
    // `else: true` edge fires whenever no guarded sibling was taken, so both are
    // guaranteed to fire — and neither can carry a budget to exhaust, because
    // `max_iterations` requires a `when:` (Decision D90).
    let guaranteed = outgoing.iter().any(|edge| graph.edge(*edge).when.is_none());
    if let Some(OnError::Skip { span }) = &node.policy.on_error
        && !guaranteed
    {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::DeadEnd,
                node.id.span.clone(),
                format!(
                    "node `{id}` of `{}` declares `on_error: skip`, and every edge leaving it is guarded",
                    cx.address
                ),
            )
            .with_label(span.clone(), "the strategy is declared here")
            .with_help(
                "a skipped node produces no output, so every guard that reads that output evaluates false and some pass takes no edge at all: give the node one edge that is unconditional or carries `else: true` (grammar 7.6.3 rule 3, Decision D97)",
            ),
        );
    }
    true
}

/// Grammar 7.3.1: a node routing on an enum field covers its variants, or
/// carries an edge guaranteed to fire.
fn exhaustive<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>, at: usize) {
    let node = graph.node(at);
    let outgoing = graph.outgoing(Vertex::Node(at));
    // Clause 1: one edge that fires for every value of every field. Nothing
    // below can then be a mistake, so the node is not read at all.
    if outgoing.iter().any(|edge| graph.edge(*edge).when.is_none()) {
        return;
    }
    let Some(output) = ctx.node_output(node) else {
        // A `map` node has no output of its own, so it routes on nothing
        // (grammar 8.6 rule 9).
        return;
    };
    let id = graph.id(at);

    let mut parsed = Vec::new();
    for edge in outgoing {
        let Some(guard) = &graph.edge(*edge).when else {
            continue;
        };
        if !ctx.guard_type_checked(&guard.span) {
            return;
        }
        let Some(program) = guards::parse(guard.value.as_str()) else {
            return;
        };
        parsed.push(program);
    }

    // The field with the largest covered set is the one the diagnostic names,
    // because it is the one closest to satisfying clause 2 (grammar 7.3.1).
    let mut widest: Option<(usize, String, Vec<String>, crate::diag::Span)> = None;
    for field in &output.fields {
        let TypeForm::Enum(declared) = &field.ty.form else {
            continue;
        };
        let name = field.name.value.as_str();
        if !parsed
            .iter()
            .any(|program| guards::mentions(program.expression(), id, name))
        {
            continue;
        }
        let variants: Vec<String> = declared
            .variants
            .iter()
            .map(|variant| variant.value.clone())
            .collect();
        let all: BTreeSet<String> = variants.iter().cloned().collect();
        let mut covered: BTreeSet<String> = BTreeSet::new();
        for program in &parsed {
            covered.extend(guards::coverage(program.expression(), id, name, &all).guaranteed);
        }
        if covered.len() == all.len() {
            // Clause 2: one field is fully covered, which already proves no
            // combination of output values leaves the node with no edge.
            return;
        }
        let uncovered: Vec<String> = variants
            .iter()
            .filter(|variant| !covered.contains(*variant))
            .cloned()
            .collect();
        if widest
            .as_ref()
            .is_none_or(|(best, ..)| covered.len() > *best)
        {
            widest = Some((
                covered.len(),
                name.to_string(),
                uncovered,
                field.ty.span.clone(),
            ));
        }
    }

    let Some((_, field, uncovered, declared_at)) = widest else {
        return;
    };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::NonExhaustive,
            node.id.span.clone(),
            format!(
                "node `{id}` of `{}` routes on `{field}` and leaves {} unrouted",
                cx.address,
                crate::parse::reader::list(&uncovered)
            ),
        )
        .with_label(declared_at, "the enum is declared here")
        .with_help(
            "cover every variant with guards the exhaustiveness table reads — `==`, `!=`, `in`, and their `&&`/`||`/`!` combinations — or give the node one outgoing edge that is unconditional or carries `else: true`; a term the table does not recognize proves nothing and covers no variant (grammar 7.3.1, Decision D82)",
        ),
    );
}
