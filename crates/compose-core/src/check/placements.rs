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
//!
//! # The second rule: a store only one process can see
//!
//! [`check_stores`] is grammar 14.1 rule 5 (PRD resolved q45, Decision D131),
//! and it is here rather than in [`super::stores`] because it is decided by the
//! same fact the rule above is: **where a component executes**. A store on a
//! backend the reaching process opens for itself — `memory`, `sqlite`,
//! `sqlite_vec`, `local_fs` — is one store per process, and a mesh runs a placed
//! component in more than one process by design: several workers claim one name
//! (PRD resolved q38's pools) and the hub dispatches whatever else reaches it.
//! So each opens its own copy, a write on one side is never a read on another,
//! and the flow carries on with data that is not there.
//!
//! What it reads is the **environment partition** — `codegen::env::Partition`,
//! `docs/distributed.md` §9.1 — rather than a walk of its own, and that is the
//! point rather than an economy. §9.1's closure is already the answer to "which
//! processes can this surface execute in", including everything reached through
//! attachment, and a second derivation of it would agree on the day it was
//! written. The partition also records *how* each process arrived, so the
//! diagnostic points at the `stores:` entry or `store:` node that binds the
//! store, at every attachment between it and the `members:` entry, and at that
//! entry — the chain rather than the verdict.

use std::collections::{BTreeMap, BTreeSet};

use crate::ast::common::Namespace;
use crate::codegen::env::{Partition, Process};
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

/// Refuse every store on a process-local backend that a placement's process can
/// open (grammar 14.1 rule 5, PRD resolved q45, Decision D131).
pub(crate) fn check_stores(ctx: &mut Ctx<'_>) {
    // A target that declares no placement has one process, so no store forks
    // and nothing here is computed — including the partition, which is the only
    // expensive thing this pass does.
    if ctx
        .ir
        .deploy
        .placements
        .as_ref()
        .is_none_or(|section| section.entries.is_empty())
    {
        return;
    }
    let partition = Partition::of(ctx.ir);

    let mut reports = Vec::new();
    for (address, definition) in &ctx.ir.definitions {
        let DefinitionBody::Store(store) = &definition.body else {
            continue;
        };
        let backend = crate::ir::deploy::backend_of(ctx.ir, store);
        if !backend.provider.opens_in_process() {
            continue;
        }
        // The first placement in name order. Several placements reaching one
        // store is one fault with one repair — the backend — so it is reported
        // once, on the route the labels can actually draw.
        //
        // **No placement at all is the accept row**, and the common one: a store
        // every process that reaches it opens on the hub is a store with one
        // process, whatever its backend. That is the rule's whole floor — a
        // composition with no `placements:` reaches it for every store it has.
        let Some((process, placement)) =
            partition
                .processes_of(address)
                .find_map(|process| match process {
                    Process::Placement(name) => Some((process, name)),
                    Process::Hub => None,
                })
        else {
            continue;
        };
        // The route starts at the `members:` entry that holds the placement and
        // ends at the store, so it has at least two steps: a `store.*` is never
        // a member of a placement — the parser refuses one outright (grammar
        // 14.1 rule 2), so a one-step route cannot be built by any spec that
        // reaches this pass. `hops` is everything between the two ends.
        //
        // **What decides the refusal is the partition, not the chain.** The
        // store opens in a placement's process — `processes_of` has already said
        // so — and that is grammar 14.1 rule 5's whole condition; the route only
        // supplies the labels that draw *how*. So a route too short to name a
        // binder costs the diagnostic its chain and never its verdict: the arm
        // below reports the store with the sentence that needs no chain, rather
        // than skipping and retiring the rule on whatever shape produced it.
        // The `debug_assert` is the louder half of the same guard — a build
        // under test panics on the shape rather than degrading quietly — and
        // between the two there is no build in which an unforeseen arrival
        // accepts a store that forks.
        let route = partition.route(address, process);
        debug_assert!(
            route.len() >= 2,
            "`{address}` arrives in placement `{placement}` in {} step(s): a store holding a \
             process in its own right has no chain to draw, and the refusal below has no binder \
             to name",
            route.len()
        );
        let [root, hops @ .., opened] = route.as_slice() else {
            reports.push(
                Diagnostic::error(
                    DiagnosticCode::ProcessLocalStore,
                    definition.span.clone(),
                    format!(
                        "`{address}` is on the process-local `{}` backend ({}), and it executes \
                         in placement `{placement}`",
                        backend.provider.as_str(),
                        backend.from,
                    ),
                )
                .with_help(repair(ctx, backend.provider)),
            );
            continue;
        };
        let binder = hops.last().unwrap_or(root);
        let mut report = Diagnostic::error(
            DiagnosticCode::ProcessLocalStore,
            opened.site.unwrap_or(&definition.span).clone(),
            format!(
                "`{address}` is on the process-local `{}` backend ({}), and `{}` that binds it \
                 executes in placement `{placement}`",
                backend.provider.as_str(),
                backend.from,
                binder.owner,
            ),
        );
        // The chain: every hop from the `members:` entry inward to the binding
        // the primary span already points at, in route order — and then the
        // entry itself, because it is the one label in the other file.
        //
        // **What the pushes fix is the chain, not what the reader meets.** The
        // renderer lays a file's labels out by source position, so a reader sees
        // the composition's own lines in *file* order — for the canonical chain,
        // the reverse of the walk's — and the deploy file's line in a section of
        // its own after them, which is where the repair is. Each label therefore
        // has to name the step it *takes* rather than rely on its neighbours,
        // which is what lets the route be reassembled from either end.
        // `process_local_store_in_a_mesh.rs`'s
        // `the_refusal_draws_the_chain_from_the_members_entry_to_the_binding`
        // and the `# label:` lines of the `process-local-store-*` fixtures pin
        // the built order, so a reordering here is a test failure — but it is
        // not a re-rendering, and adding a hop is what changes what is drawn.
        let mut previous = root;
        for step in hops {
            if let Some(site) = step.site {
                report = report.with_label(
                    site.clone(),
                    format!("`{}` reaches `{}` here", previous.owner, step.owner),
                );
            }
            previous = step;
        }
        if let Some(site) = root.site {
            report = report.with_label(
                site.clone(),
                format!("`{}` is a member of placement `{placement}`", root.owner),
            );
        }
        reports.push(report.with_help(repair(ctx, backend.provider)));
    }
    for report in reports {
        ctx.push(report);
    }
}

/// What the author writes instead, which is a different sentence under `local`.
///
/// Under any named target the design's repair is a networked backend, whose
/// credentials §9.1's partition already carries to every placement that reaches
/// the store. Under `local` there is no such edit — the target substitutes local
/// storage for **every** store unconditionally and refuses a `storage_backends:`
/// block outright (grammar 14, Decision D87) — so offering one would send an
/// author to a key the next compile refuses.
///
/// **Both sentences end in the same caveat, and what it says depends on the
/// store's kind.** For a `kv` store the first repair is one a build of this
/// release really runs: `postgres` and `mysql` are implemented behind the store
/// interface (PRD resolved q63), so an author told to bind one is told something
/// that works, and the caveat's job is to say which of the offered names those
/// are. For a `vector` or `blob` store none of them is yet: `src/stores.ts`
/// refuses every other provider at the first store op, because the rest of
/// production `storage_backends` land behind the store plugin interface in M3
/// (PRD §7) — so a diagnostic that offered `chroma` and stopped would send an
/// author to a build that compiles and then throws, and the repair the release
/// does have is the second one, taking the component out of `placements:`.
/// Naming both, in that order, is what keeps this an error message an author can
/// act on today without hiding the shape the deployment is heading for (PRD G3).
fn repair(ctx: &Ctx<'_>, provider: crate::ast::deploy::BackendProvider) -> String {
    // Only the ones that serve this store's kind: a `kv` store cannot be bound
    // to `s3`, so offering it would be a repair the next compile refuses
    // (grammar 14.3).
    let networked: Vec<String> = crate::ast::deploy::BackendProvider::networked()
        .filter(|other| other.kind() == provider.kind())
        .map(|other| format!("`{}`", other.as_str()))
        .collect();
    let list = match networked.split_last() {
        Some((last, [])) => last.clone(),
        Some((last, rest)) => format!("{} or {last}", rest.join(", ")),
        None => "a networked backend".to_string(),
    };
    // …and which of those this release opens, which is what decides whether the
    // sentence above is a repair or a direction of travel.
    let live: Vec<String> = crate::ast::deploy::BackendProvider::networked()
        .filter(|other| other.kind() == provider.kind() && other.implemented())
        .map(|other| format!("`{}`", other.as_str()))
        .collect();
    // The caveat both sentences end in. See this function's own note.
    let today = match live.split_last() {
        None => "This release opens only the process-local backends for this kind — a networked \
                 one compiles and then refuses at the first store op, since production \
                 `storage_backends` land behind the store plugin interface in M3 (PRD §7) — so \
                 taking the component out of `placements:` is the repair a build of this release \
                 runs"
            .to_string(),
        Some((last, rest)) => {
            let live = if rest.is_empty() {
                last.clone()
            } else {
                format!("{} and {last}", rest.join(", "))
            };
            format!(
                "Of those, this release opens {live}; the others compile and then refuse at the \
                 first store op, since they land behind the store plugin interface in M3 (PRD §7)"
            )
        }
    };
    if ctx.ir.target == crate::DEFAULT_TARGET {
        format!(
            "a mesh runs this store's component in more than one process — several workers may \
             claim one placement, and the hub dispatches whatever else reaches the store — and each \
             one opens its own copy, so a write on one side is never a read on another. `local` \
             substitutes local storage for every store and admits no `storage_backends:`, so a \
             mesh that shares this store is a target of its own: declare `deploy/<target>.yml` \
             binding it to {list}, or take the component that binds it out of `placements:` so \
             only the hub ever opens it. {today} (grammar 14, 14.1 rule 5, PRD resolved q45)"
        )
    } else {
        format!(
            "a mesh runs this store's component in more than one process — several workers may \
             claim one placement, and the hub dispatches whatever else reaches the store — and each \
             one opens its own copy, so a write on one side is never a read on another: bind {list} \
             instead, whose variables the environment partition already carries to every placement \
             that reaches the store, or take the component that binds it out of `placements:` so \
             only the hub ever opens it. {today} (grammar 14.1 rule 5, PRD resolved q45)"
        )
    }
}
