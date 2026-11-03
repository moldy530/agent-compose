//! A flow's `finally:` node, held to what makes it one (grammar 7.9, 8.7,
//! Decision D159, PRD resolved q76).
//!
//! `finally: <node id>` names a node of the same flow that runs **once**, when
//! an instance of that flow ends — completed, failed, or cancelled — with the
//! outcome readable inside it. The resolver has already checked that the id
//! names a node (in the words it uses for an edge naming a missing one); what is
//! left is what the node may be, and every rule here is stated over the node
//! and the flow around it together, which is why it is the validator's:
//!
//! 1. **It is outside routing.** No edge names it, at either end; no
//!    `on_error: { fallback: … }` and no `human.on_timeout:` names it either —
//!    nothing transfers control to it, because what schedules it is the
//!    instance ending rather than a branch arriving, and it transfers control
//!    to nothing, because there is no branch left for it to continue. Each
//!    offending position is one diagnostic, naming the flow, the node and the
//!    position (`routed-finally`). It needs no outgoing edge in exchange: it is
//!    the one node grammar 7.6.3 rule 1 exempts ([`routing`](super::routing)),
//!    and it is reachable because it is `finally:` — grammar 7.8 clause 4, read
//!    by [`reachable`](super::reachable) through
//!    [`Graph::addressable`](super::graph::Graph::addressable).
//! 2. **Its failure has one meaning.** It takes its own `retry:` and
//!    `timeout:` through grammar 9.3 like any node; an error it raises after
//!    them is `finally_error` on the execution's record, with the outcome
//!    unchanged. So an `on_error:` of its own is refused: `skip` and `fallback`
//!    would be routing out of a node that routes nowhere, and `fail` would be a
//!    second spelling of a meaning that is not the one it has. A strategy that
//!    reaches it from `defaults:` or an instantiating `policy:` is not an error
//!    — those levels are written for every node — and is not applied to it
//!    (`codegen::graph` resolves its `on_error` from no level).
//! 3. **It never parks.** A step that must always finish cannot wait on a
//!    person, so it is no `human` node and reaches none — through a `flow:` node
//!    it is, a `map` it is, or the `flow.*` tools of an agent it calls — over
//!    grammar 7.7's relation, the third interrupt-freedom rule beside the
//!    `respond: sync` one and the detached-dispatch one
//!    ([`reach::reached_by_node`], `finally-interrupt`).
//!
//! Nothing here is about the CEL scope inside the node: that `execution.outcome`
//! and `execution.error` are readable there and nowhere else is the expression
//! pass's ([`expr`](super::expr), [`FlowCx::finally`]).

use crate::ast::common::{ControlTarget, EdgeSource, EdgeTarget};
use crate::diag::{Diagnostic, DiagnosticCode, Span};
use crate::ir::flow::{Edge, NodeKind};
use crate::ir::policy::OnError;

use super::graph::Graph;
use super::{Ctx, FlowCx, reach};

/// Check one flow's `finally:` node, where it declares one that resolved.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    let Some(at) = graph.finally() else {
        return;
    };
    let Some(declared) = cx.flow.finally.as_ref() else {
        return;
    };
    let node = graph.node(at);
    let id = graph.id(at);

    for edge in &cx.flow.edges {
        routed_edge(ctx, cx, edge, id, &declared.span);
    }
    for other in graph.nodes() {
        if let Some(OnError::Fallback { target, .. }) = &other.policy.on_error
            && matches!(&target.value, ControlTarget::Node(to) if to.as_str() == id)
        {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::RoutedFinally,
                    target.span.clone(),
                    format!(
                        "node `{}` of `{}` falls back to `{id}`, which is the flow's `finally:` node",
                        other.id.value, cx.address
                    ),
                )
                .with_label(declared.span.clone(), "`finally:` names it here")
                .with_help(ROUTED_HELP),
            );
        }
        if let NodeKind::Human { human } = &other.kind
            && let Some(target) = human.on_timeout.as_ref()
            && matches!(&target.value, ControlTarget::Node(to) if to.as_str() == id)
        {
            ctx.push(
                Diagnostic::error(
                    DiagnosticCode::RoutedFinally,
                    target.span.clone(),
                    format!(
                        "the `human` node `{}` of `{}` times out to `{id}`, which is the flow's `finally:` node",
                        other.id.value, cx.address
                    ),
                )
                .with_label(declared.span.clone(), "`finally:` names it here")
                .with_help(ROUTED_HELP),
            );
        }
    }

    if let Some(strategy) = node.policy.on_error.as_ref() {
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::RoutedFinally,
                strategy_span(strategy).clone(),
                format!(
                    "`{id}`, the `finally:` node of `{}`, declares `on_error:`",
                    cx.address
                ),
            )
            .with_label(declared.span.clone(), "`finally:` names it here")
            .with_help(
                "a finally node's failure has one meaning: once its own `retry:` and `timeout:` are spent, the error is recorded as `finally_error` on the execution's record and the instance's outcome stays what it was — so no strategy can apply to it, and `skip` or `fallback` would route out of a node that routes nowhere. Delete the key (grammar 7.9, 9.2, Decision D159)",
            ),
        );
    }

    let reached = reach::reached_by_node(ctx.ir, cx.address, node);
    if let Some(((holder, human), at_human)) = reached.humans.iter().next() {
        let (message, label) = if matches!(node.kind, NodeKind::Human { .. }) {
            (
                format!(
                    "`{id}`, the `finally:` node of `{}`, is a `human` node",
                    cx.address
                ),
                "the `human` node is declared here",
            )
        } else {
            (
                format!(
                    "`{id}`, the `finally:` node of `{}`, reaches the `human` node `{human}` of `{holder}`",
                    cx.address
                ),
                "the `human` node is declared here",
            )
        };
        ctx.push(
            Diagnostic::error(DiagnosticCode::FinallyInterrupt, declared.span.clone(), message)
                .with_label(at_human.clone(), label)
                .with_help(
                    "a finally node runs once when its instance ends, and a step that must always finish cannot park on a question: it may be no `human` node and reach none — through a `flow:` node, a `map`'s dispatch targets, or the `flow.*` tools of an agent it calls. Take the pause out of what it runs (grammar 7.9, 8.7, 7.7, Decision D159)",
                ),
        );
    }
}

/// One edge, refused if it names the finally node at either end.
fn routed_edge(ctx: &mut Ctx, cx: &FlowCx, edge: &Edge, id: &str, declared: &Span) {
    let leaves = matches!(&edge.from.value, EdgeSource::Node(from) if from.as_str() == id);
    let enters = matches!(&edge.to.value, EdgeTarget::Node(to) if to.as_str() == id);
    if !leaves && !enters {
        return;
    }
    let from = match &edge.from.value {
        EdgeSource::Start => "start",
        EdgeSource::Node(from) => from.as_str(),
    };
    let to = match &edge.to.value {
        EdgeTarget::End => "end",
        EdgeTarget::Node(to) => to.as_str(),
    };
    // An edge that does both is one mistake and one diagnostic, and the word
    // for it is the one that names the node it starts from.
    let verb = if leaves { "leaves" } else { "routes to" };
    ctx.push(
        Diagnostic::error(
            DiagnosticCode::RoutedFinally,
            edge.span.clone(),
            format!(
                "the edge `{from}` → `{to}` of `{}` {verb} `{id}`, which is the flow's `finally:` node",
                cx.address
            ),
        )
        .with_label(declared.clone(), "`finally:` names it here")
        .with_help(ROUTED_HELP),
    );
}

/// Where an `on_error:` value was written.
const fn strategy_span(strategy: &OnError) -> &Span {
    match strategy {
        OnError::Fail { span } | OnError::Skip { span } | OnError::Fallback { span, .. } => span,
    }
}

/// The repair every routing position into or out of the finally node shares.
const ROUTED_HELP: &str = "a finally node is outside routing: the instance ending is what runs it, once, whatever the outcome — so nothing routes to it, it routes to nothing, and it needs no edge to be reachable (grammar 7.8 clause 4). Delete this, and route the branch to `end` where it is done (grammar 7.9, Decision D159)";
