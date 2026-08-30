//! Where a placed component actually executes (grammar 14.1, Decision D129).
//!
//! The whole generated artifact reaches every worker (PRD resolved q40), so a
//! placement decides *which* process runs a node rather than which code exists
//! there. That is what makes this rule necessary and what makes it small.
//!
//! An agent's `tools:` list is the one construct that runs another component
//! **inside** the agent's own process rather than handing it back to the hub's
//! scheduler. A tool attached to an agent is called from inside that agent's
//! tool loop, and a flow attached to an agent starts an instance in that same
//! loop (grammar 5.4). So an attached component's own placement never gets a
//! say: whichever worker took the agent's dispatch runs it too.
//!
//! # The direct case: an attached `tool.*`
//!
//! | the agent | the tool | verdict |
//! |---|---|---|
//! | placed `mac` | no placement | fine — the artifact is everywhere, and the tool runs where the agent runs |
//! | placed `mac` | placed `mac` | fine — the same answer written twice |
//! | placed `mac` | placed `gpu` | **refused** — two answers to which worker runs one tool call |
//! | no placement | placed `mac` | **refused** — the agent runs on the hub, so the tool would too |
//!
//! The last row is the one worth stating out loud, because it looks like a
//! placement that simply has not been used yet and is not: a `mac`-only tool
//! attached to an unplaced agent runs on the hub, where the signing keys are
//! not. Left legal, it would be a placement an author wrote, a compiler
//! accepted, and a deployment silently ignored.
//!
//! **"Runs on the hub" is a claim about the composition, not about one call
//! site**, and it survives the shape that looks like a counterexample: an agent
//! reachable *only* from inside some other agent's process — named by a flow
//! that a placed agent attaches, and by nothing else. Manual invocation is
//! universal. Every flow a composition declares is runnable on its own, whether
//! or not a `manual` trigger names it (PRD 5.11, Decision D64), and
//! `codegen::graph`'s registry is every flow rather than every triggered one —
//! so a flow attached as a tool is also a flow the hub starts directly, and the
//! `agent:` node inside it is then dispatched by the hub's scheduler like any
//! other. That is why this pass reads the attaching agent's **own** placement
//! and nothing else: an unplaced agent named anywhere has an execution on the
//! hub, so there is no composition in which a wider question would answer
//! differently.
//!
//! # The transitive case: an attached `flow.*`
//!
//! A `flow.*` in a `tools:` list cannot itself be placed — placing a flow is
//! deferred (PRD resolved q44) — but the components it reaches can be, and the
//! flow's instance runs in the agent's process just as a tool call does. Every
//! `agent.*` and `tool.*` that instance reaches therefore executes wherever the
//! attaching agent executes, and the same four rows apply to each of them.
//!
//! Reading the flow's own address as "carries no claim, so nothing to disagree
//! with" is what would leave the hole: the flow claims nothing and the nodes
//! inside it claim plenty. A placement reached only that way would be a
//! placement written, accepted, and silently ignored — the same failure the last
//! row of the table exists for, one indirection further out.
//!
//! What the walk collects is grammar 7.7 clauses 1 and 2 — the components a
//! flow's own nodes and maps name — and it stops at an agent's attached
//! `tool.*`, exactly where clauses 3 and 4 stop. That boundary is deliberate: an
//! attached tool is already governed by the direct case above, so following it
//! here would report one contradiction twice, and the second report would point
//! at a line whose repair is the first report's.
//!
//! # One contradiction, one diagnostic
//!
//! That boundary answers only the inner agent's attachments. Two shapes reach
//! one contradiction twice by paths it never crosses, and both are answered
//! here rather than by the traversal:
//!
//! * an agent attaches a placed `tool.*` **and** a `flow.*` whose own nodes name
//!   that same tool. The direct case and the transitive one then land on one
//!   address, and both are the same repair — place the agent, or drop the tool's
//!   placement. The **direct** report is the one kept, because it names the
//!   attachment the author wrote rather than a path to it, and it is kept
//!   whatever order the `tools:` list happens to be in.
//! * an agent attaches two flows that both reach one placed component. One
//!   contradiction again, reported once, labelled at the first site the walk
//!   found.
//!
//! Both are dropped on the address, never on the diagnostic: an agent holding
//! genuinely separate contradictions — two placed tools it cannot run — still
//! gets one report for each of them.
//!
//! # What is not here
//!
//! The tool reached without an agent. A `function:` node names a `tool.*`
//! directly (Decision D24), and there its own placement is the whole of the
//! answer — nothing to disagree with, so nothing to check. That is the case a
//! placed tool exists for. A `flow:` node is the same story one level up: the
//! hub schedules the nodes of the instance it starts, so every placement inside
//! it is honoured.
//!
//! The pass needs the composition (which agents attach what, and what those
//! attachments reach) and the active target's deploy layer (which placement
//! claims each), so it is a check rather than a parser or resolver rule:
//! neither of the two files decides it alone.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::Namespace;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::definition::DefinitionBody;

use super::{Ctx, reach};

/// Refuse every attachment whose placement disagrees with the agent's.
pub(crate) fn check(ctx: &mut Ctx<'_>) {
    // Which placement claims each component, by address. Built once: a
    // composition with no placements does no work here at all.
    let Some(section) = ctx.ir.deploy.placements.as_ref() else {
        return;
    };
    let mut claim: BTreeMap<String, &Spanned<crate::ast::common::Ident>> = BTreeMap::new();
    for placement in section.entries.values() {
        for member in &placement.members {
            // A component named by two placements has already been reported
            // (Decision D129); the first claim stands so this pass reports the
            // mismatch once rather than once per duplicate.
            claim
                .entry(member.value.to_string())
                .or_insert(&placement.name);
        }
    }
    if claim.is_empty() {
        return;
    }

    let mut reports = Vec::new();
    for (address, definition) in &ctx.ir.definitions {
        let DefinitionBody::Agent(agent) = &definition.body else {
            continue;
        };
        let on = claim.get(address);
        // The `tool.*` this agent attaches itself, and the addresses an attached
        // flow has already been reported for. Both keep one contradiction to one
        // diagnostic; `direct` is built before the loop so the report kept is
        // the direct one however the `tools:` list is ordered.
        let direct: BTreeSet<String> = agent
            .tools
            .iter()
            .filter(|attached| attached.value.namespace == Namespace::Tool)
            .map(|attached| attached.value.to_string())
            .collect();
        let mut reported: BTreeSet<String> = BTreeSet::new();
        for attached in &agent.tools {
            match attached.value.namespace {
                Namespace::Tool => {
                    let Some(runs_on) = claim.get(&attached.value.to_string()) else {
                        // The artifact is everywhere: a tool that claims nothing
                        // runs wherever the agent that called it runs.
                        continue;
                    };
                    if on.is_some_and(|held| held.value == runs_on.value) {
                        continue;
                    }
                    reports.push(
                        Diagnostic::error(
                            DiagnosticCode::ConflictingPlacement,
                            attached.span.clone(),
                            format!(
                                "`{}` is a member of placement `{}`, and `{address}` that attaches it has {}",
                                attached.value,
                                runs_on.value,
                                where_it_runs(on)
                            ),
                        )
                        .with_label(
                            runs_on.span.clone(),
                            format!("`{}` is declared here", runs_on.value),
                        )
                        .with_help(
                            "an attached tool is called from inside the agent's own tool loop, so it executes on whichever worker took the agent's dispatch: put the two in one placement, drop the tool's, or reach the tool from a `function:` node, where its placement is the whole answer (grammar 14.1, PRD resolved q40)",
                        ),
                    );
                }
                Namespace::Flow => {
                    // The flow carries no claim of its own — placing one is
                    // deferred — but the instance it starts runs here, so every
                    // placed component it reaches is holding a second answer.
                    let inside = reach::reached(ctx.ir, &attached.value.to_string());
                    for (reached, site) in &inside.placeable {
                        let Some(runs_on) = claim.get(reached) else {
                            continue;
                        };
                        if on.is_some_and(|held| held.value == runs_on.value) {
                            continue;
                        }
                        // The same contradiction reached a second way — by the
                        // agent's own `tools:` list, which the direct case
                        // reports better, or by another attached flow, which
                        // reports it identically. One repair, one diagnostic.
                        if direct.contains(reached) || !reported.insert(reached.clone()) {
                            continue;
                        }
                        reports.push(
                            Diagnostic::error(
                                DiagnosticCode::ConflictingPlacement,
                                attached.span.clone(),
                                format!(
                                    "`{reached}` is a member of placement `{}`, and `{address}` that reaches it through the attached `{}` has {}",
                                    runs_on.value,
                                    attached.value,
                                    where_it_runs(on)
                                ),
                            )
                            .with_label(
                                site.clone(),
                                format!("`{}` reaches `{reached}` here", attached.value),
                            )
                            .with_label(
                                runs_on.span.clone(),
                                format!("`{}` is declared here", runs_on.value),
                            )
                            .with_help(
                                "a flow attached as a tool runs its instance inside the agent's own tool loop (grammar 5.4), so every node of it executes on whichever worker took the agent's dispatch: put the agent in that placement, drop the placement from what the flow reaches, or reach the flow from a `flow:` node instead, where the hub schedules its nodes and each placement is honoured (grammar 14.1, PRD resolved q40)",
                            ),
                        );
                    }
                }
                _ => {}
            }
        }
    }
    for report in reports {
        ctx.push(report);
    }
}

/// Where the attaching agent runs, as the clause that completes both messages.
fn where_it_runs(on: Option<&&Spanned<crate::ast::common::Ident>>) -> String {
    on.map_or_else(
        || "no placement, so it runs on the hub".to_string(),
        |held| format!("placement `{}`", held.value),
    )
}
