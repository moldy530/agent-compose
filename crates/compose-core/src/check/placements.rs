//! Where a placed tool actually executes (grammar 14.1, Decision D129).
//!
//! The whole generated artifact reaches every worker (PRD resolved q40), so a
//! placement decides *which* process runs a node rather than which code exists
//! there. That is what makes this rule necessary and what makes it small.
//!
//! A tool attached to an agent is called from **inside that agent's tool loop**,
//! in the process running the agent. So an attached tool's own placement never
//! gets a say: whichever worker took the agent's dispatch runs the tool call
//! too. Three of the four combinations are therefore fine and one is a
//! contradiction:
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
//! What is **not** here is the tool reached without an agent. A `function:`
//! node names a `tool.*` directly (Decision D24), and there its own placement
//! is the whole of the answer — nothing to disagree with, so nothing to check.
//! That is the case a placed tool exists for.
//!
//! The pass needs the composition (which agents attach which tools) and the
//! active target's deploy layer (which placement each is a member of), so it is
//! a check rather than a parser or resolver rule: neither of the two files
//! decides it alone.

use std::collections::BTreeMap;

use crate::ast::common::Namespace;
use crate::diag::{Diagnostic, DiagnosticCode, Spanned};
use crate::ir::definition::DefinitionBody;

use super::Ctx;

/// Refuse every attached tool whose placement disagrees with the agent's.
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
        for tool in &agent.tools {
            if tool.value.namespace != Namespace::Tool {
                // A `flow.*` used as a tool cannot be placed at all
                // (Decision D129), so it carries no claim to disagree with.
                continue;
            }
            let Some(runs_on) = claim.get(&tool.value.to_string()) else {
                // The artifact is everywhere: a tool that claims nothing runs
                // wherever the agent that called it runs.
                continue;
            };
            if on.is_some_and(|held| held.value == runs_on.value) {
                continue;
            }
            let where_the_agent_runs = on.map_or_else(
                || "no placement, so it runs on the hub".to_string(),
                |held| format!("placement `{}`", held.value),
            );
            reports.push(
                Diagnostic::error(
                    DiagnosticCode::ConflictingPlacement,
                    tool.span.clone(),
                    format!(
                        "`{}` is a member of placement `{}`, and `{address}` that attaches it has {where_the_agent_runs}",
                        tool.value, runs_on.value
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
    }
    for report in reports {
        ctx.push(report);
    }
}
