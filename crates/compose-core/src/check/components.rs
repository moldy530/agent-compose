//! The two composition-wide checks stated over grammar 7.7's component
//! reachability: recursion, and a synchronous trigger reaching a human.
//!
//! Grammar 7.7 defines *reaching* once — a flow's own nodes, its maps' dispatch
//! targets, the stores and the tools of every agent it reaches, including the
//! `flow.*` entries of those tool lists, and everything the flows it reaches
//! reach in turn — precisely so that the three checks over it cannot drift apart
//! (Decision D86). The third, session coherence, is
//! [`stores`](super::stores)'; the traversal itself is [`reach`](super::reach)'s.
//!
//! # Recursion (grammar 7.5, Decision D26)
//!
//! A flow that reaches itself is a compile error naming the cycle. The check is
//! stated over the whole **invocation graph** rather than as a walk per flow:
//! the flows of one cycle all reach themselves, so a per-flow walk would report
//! `flow.a` and `flow.b` for one mistake. Finding the strongly connected
//! components of that graph reports each cycle once, and a shortest walk back to
//! the component's first member is the chain the diagnostic names.
//!
//! Recursion has no termination proof analogous to grammar 7.4's SCC rule and
//! would break the fan-out bounding guarantees, which is why it is refused
//! rather than bounded.
//!
//! # Interrupt-freedom (grammar 13.3, 8.7, PRD 5.11)
//!
//! A declared `http` trigger with `respond: sync` blocks and returns the flow's
//! outputs, so PRD 5.11's settled position is that such a flow is *statically*
//! interrupt-free: it must not reach a `human` node. The relation is grammar
//! 7.7's whole relation and not "the nodes of the flow" — an interrupt inside a
//! flow attached to an agent's `tools:` is still an interrupt in the middle of a
//! synchronous request, and that is the gap D86 exists to close.
//!
//! The check quantifies over **declared** triggers. Implicit `manual` invocation
//! is a CLI affordance, contributes no trigger, and is quantified over by
//! nothing (Decision D64).

use std::collections::{BTreeMap, BTreeSet, VecDeque};

use crate::ast::trigger::Respond;
use crate::diag::{Diagnostic, DiagnosticCode};
use crate::ir::definition::DefinitionBody;
use crate::ir::trigger::TriggerKind;

use super::{Ctx, graph, reach};

/// Run both composition-wide component checks.
pub(crate) fn check(ctx: &mut Ctx) {
    recursion(ctx);
    interrupt_freedom(ctx);
}

/// Grammar 7.5: no flow reaches itself.
fn recursion(ctx: &mut Ctx) {
    let flows: Vec<String> = ctx
        .ir
        .definitions
        .iter()
        .filter(|(_, definition)| matches!(definition.body, DefinitionBody::Flow(_)))
        .map(|(address, _)| address.clone())
        .collect();
    let index: BTreeMap<&str, usize> = flows
        .iter()
        .enumerate()
        .map(|(at, address)| (address.as_str(), at))
        .collect();

    let calls: Vec<Vec<reach::Call>> = flows
        .iter()
        .map(|address| reach::calls(ctx, address))
        .collect();
    let successors: Vec<Vec<usize>> = calls
        .iter()
        .map(|calls| {
            let mut targets: Vec<usize> = Vec::new();
            for call in calls {
                if let Some(at) = index.get(call.target.as_str())
                    && !targets.contains(at)
                {
                    targets.push(*at);
                }
            }
            targets
        })
        .collect();

    let (_, components) = graph::components(&successors);
    for members in &components {
        let Some(&root) = members.first() else {
            continue;
        };
        if members.len() == 1 && !successors[root].contains(&root) {
            continue;
        }
        let Some(cycle) = shortest_cycle(&successors, members, root) else {
            continue;
        };
        // The cycle is `root → cycle[0] → … → root`, so the invocation sites are
        // the call from each member to the next, wrapping at the end.
        let sites: Vec<&reach::Call> = std::iter::once(root)
            .chain(cycle.iter().copied())
            .zip(cycle.iter().copied().chain(std::iter::once(root)))
            .filter_map(|(from, to)| {
                calls[from]
                    .iter()
                    .find(|call| index.get(call.target.as_str()) == Some(&to))
            })
            .collect();
        let Some((first, rest)) = sites.split_first() else {
            continue;
        };
        let through: Vec<&str> = cycle.iter().map(|at| flows[*at].as_str()).collect();
        let message = if through.is_empty() {
            format!("`{}` reaches itself", flows[root])
        } else {
            format!(
                "`{}` reaches itself through {}",
                flows[root],
                crate::parse::reader::list(&through)
            )
        };
        let mut diagnostic =
            Diagnostic::error(DiagnosticCode::RecursiveFlow, first.span.clone(), message);
        for (call, from) in rest.iter().zip(cycle.iter()) {
            diagnostic = diagnostic.with_label(
                call.span.clone(),
                format!("`{}` {} `{}` here", flows[*from], call.verb, call.target),
            );
        }
        ctx.push(diagnostic.with_help(
            "a flow that reaches itself — through a `flow:` node, a `map` dispatch target, or the `flow.*` tools of an agent it reaches — has no termination proof analogous to grammar 7.4's SCC rule and would break the fan-out bounding guarantees: extract the shared part into a third flow, or break the cycle (grammar 7.5, 7.7, Decisions D26, D86)",
        ));
    }
}

/// The shortest walk from `root` back to itself inside one component, as the
/// members it passes through *between* the two visits — empty for a flow that
/// invokes itself directly.
fn shortest_cycle(successors: &[Vec<usize>], members: &[usize], root: usize) -> Option<Vec<usize>> {
    let inside: BTreeSet<usize> = members.iter().copied().collect();
    let mut previous: BTreeMap<usize, usize> = BTreeMap::new();
    let mut seen: BTreeSet<usize> = BTreeSet::from([root]);
    let mut queue = VecDeque::from([root]);
    while let Some(node) = queue.pop_front() {
        for next in &successors[node] {
            if *next == root {
                // Walk back from the node that closes the cycle.
                let mut chain = Vec::new();
                let mut at = node;
                while at != root {
                    chain.push(at);
                    at = *previous.get(&at)?;
                }
                chain.reverse();
                return Some(chain);
            }
            if inside.contains(next) && seen.insert(*next) {
                previous.insert(*next, node);
                queue.push_back(*next);
            }
        }
    }
    None
}

/// Grammar 13.3: a `respond: sync` trigger's flow reaches no `human` node.
fn interrupt_freedom(ctx: &mut Ctx) {
    let Some(triggers) = ctx.ir.triggers.as_ref() else {
        return;
    };
    for trigger in triggers.entries.values() {
        let TriggerKind::Http(http) = &trigger.kind else {
            continue;
        };
        // `respond:` defaults to `async`, which imposes nothing (grammar 13.3).
        if http.respond != Some(Respond::Sync) {
            continue;
        }
        let flow = trigger.flow.value.to_string();
        let reached = reach::reached(ctx, &flow);
        let Some(((holder, node), at)) = reached.humans.iter().next() else {
            continue;
        };
        // The trigger's *name*, not its block: a block mapping ends where the
        // next top-level key begins, so anchoring on the whole entry would
        // underline the definition after it and say nothing about the trigger
        // (PRD G3). The name is what the message already reads.
        ctx.push(
            Diagnostic::error(
                DiagnosticCode::SyncTriggerInterrupt,
                trigger.name.span.clone(),
                format!(
                    "the trigger `{}` responds synchronously, and `{flow}` reaches the `human` node `{node}` of `{holder}`",
                    trigger.name.value
                ),
            )
            .with_label(at.clone(), "the `human` node is declared here")
            .with_help(
                "a flow exposed with `respond: sync` must be statically interrupt-free: it must reach no `human` node through its own nodes, its `flow:` nodes, its maps' dispatch targets, or the `flow.*` tools of any agent it reaches — declare `respond: async` and take the result through `callback:` instead (grammar 13.3, 7.7, PRD 5.11)",
            ),
        );
    }
}
