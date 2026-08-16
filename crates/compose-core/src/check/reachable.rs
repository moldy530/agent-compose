//! Node reachability: every node of a flow is reachable from that flow's
//! `start` (grammar 7.8, Decision D95).
//!
//! The relation is the flow's **control-transfer** relation — edges, plus the
//! two positions that schedule a node exactly as an edge does,
//! `on_error: { fallback: … }` and `human.on_timeout:`. Reading edges alone
//! would reject a dedicated `cleanup` node reached solely by a fallback, which
//! is an ordinary shape and live code; reading them counts it, which is what
//! `examples/triage-fanout`'s `announce_failed` relies on.
//!
//! Guards are ignored. The question is whether a node is *ever* addressable, not
//! whether a path fires — grammar 7.3.1 is where guard coverage is decided, and
//! folding the two together would make an unroutable-in-practice node either an
//! error twice or an error nowhere.
//!
//! This is not grammar 7.7's relation, which asks which *components* a flow can
//! cause to run, across the composition ([`components`](super::components)); nor
//! does it feed grammar 7.6.2's `dist`, which counts edges because it measures
//! steps. Three relations, three questions (Decision D95).

use crate::diag::{Diagnostic, DiagnosticCode};

use super::graph::Graph;
use super::{Ctx, FlowCx};

/// Check every node of one flow.
pub(crate) fn check<'a>(ctx: &mut Ctx<'a>, cx: &FlowCx<'a>, graph: &Graph<'a>) {
    for (at, addressable) in graph.addressable().iter().enumerate() {
        if *addressable {
            continue;
        }
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::UnreachableNode,
                graph.node(at).id.span.clone(),
                format!(
                    "node `{}` of `{}` is not reachable from `start`",
                    graph.id(at),
                    cx.address
                ),
            )
            .with_help(
                "reachability counts edges and the two control-transfer positions — `on_error: { fallback: … }` and `human.on_timeout:` — so a node none of them addresses is code no execution can reach: give it an inbound edge, name it as a fallback, or delete it (grammar 7.8, Decision D95)",
            ),
        );
    }
}
