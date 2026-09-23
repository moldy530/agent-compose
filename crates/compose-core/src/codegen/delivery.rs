//! `src/delivery.ts`: what a settled execution owes the outside world, and the
//! one place this app pays it (`docs/durability.md` §3.7, grammar §13.3, §14.5).
//!
//! The ledger has two kinds of delivery on it — a trigger's lifecycle
//! `callback:` and the deploy layer's `trace_sink:` — and they differ in who
//! asked rather than in how they are worked. So the *worker* is here: the
//! bounded schedule, the claim that keeps one row from being worked twice, the
//! journal writes that record each attempt, and the one `fetch` the lifecycle
//! makes. The half that is a **trigger's** — resolving a callback URL out of a
//! request payload and holding it to that trigger's `callback_allow:` — stays in
//! [`super::serve`], where the trigger is.
//!
//! # Why a module rather than a corner of `serve.ts`
//!
//! Because two commands settle executions. PRD resolved q50 puts the trace sink
//! "wherever executions settle under a target that declares it — `run` included,
//! not just `serve`", and `agent-compose run` mounts no app. A delivery worker
//! that lived inside the app would have left `run` either exporting nothing or
//! carrying a second implementation of a bounded, journaled schedule — and two
//! of those drift.
//!
//! It is a **constant** like [`super::runtime`] and [`super::serve`]: what
//! differs between two deployments is the sink [`super::deployment`] names.

use crate::ir::Ir;

/// The delivery worker's source, carried in the compiler and emitted verbatim.
const SOURCE: &str = include_str!("js/delivery.ts");

/// `src/delivery.ts`.
#[must_use]
pub fn module(ir: &Ir) -> super::GeneratedFile {
    let mut contents = super::header(ir, "// ");
    contents.push_str(SOURCE);
    super::GeneratedFile {
        path: "src/delivery.ts".to_string(),
        contents,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codegen::test_support::ir_of;

    /// The worker is a constant: two compositions emit the same bytes after the
    /// provenance line.
    #[test]
    fn the_delivery_worker_is_the_same_module_in_every_project() {
        let one = module(&ir_of("version: \"0.1\"\n")).contents;
        let two = module(&ir_of(
            "version: \"0.1\"\n\
provider.p:\n  kind: openai\n  api_key: ${SOME_KEY}\n\
model.m:\n  provider: provider.p\n  id: some-model\n",
        ))
        .contents;
        let body = |text: &str| {
            text.split_once('\n')
                .expect("every emitted file carries a provenance line")
                .1
                .to_string()
        };
        assert_eq!(body(&one), body(&two));
    }

    /// **A trace sink is never refused** (grammar §14.5, PRD resolved q50).
    ///
    /// `refused` is the status an allowlist miss produces, and a sink's address
    /// is admitted by no list — it is the operator's, written in the deploy file.
    /// The ledger states that in two places, and this reads the third: nothing in
    /// the module that ships a trace reaches a refusal at all, so the status
    /// cannot arrive by a route the type and the SQL predicate did not foresee.
    #[test]
    fn the_module_that_ships_a_trace_never_refuses_a_delivery() {
        assert!(
            !SOURCE.contains("refuseDelivery") && !SOURCE.contains("refuseRecordedDelivery"),
            "`src/delivery.ts` reaches a refusal, which is a `callback` row's outcome and not a \
             sink's: there is no list a sink's address could miss (grammar §14.5)"
        );
        // The once-per-execution guard asks which **kind** a row is — a callback
        // webhook is not an export — and, since PRD resolved q64 put a second
        // event class on the sink's rows, which **class**: a detached delivery's
        // envelope is not its parent's export either.
        let guard = SOURCE
            .split_once("async function exportedAlready(")
            .expect("`src/delivery.ts` guards the export")
            .1
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        assert!(
            guard.contains("executionExport"),
            "the once-per-execution guard no longer asks whether a row is the execution's **own** \
             export, so a detached delivery's envelope would silence its parent's trace"
        );
        let own = SOURCE
            .split_once("export function executionExport(")
            .expect("`src/delivery.ts` tells an execution's export from a delivery's envelope")
            .1
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        assert!(
            own.contains("record.kind !== \"trace_sink\"")
                && own.contains("traceSinkClass(record) === \"execution\""),
            "the export guard no longer asks the ledger which kind a row is and which class, so \
             a callback webhook or a detached delivery's envelope would stand in for an export \
             nobody made"
        );
        assert!(
            SOURCE.contains("kind: \"trace_sink\""),
            "a trace export is journaled under its own delivery kind, or the ledger cannot tell \
             one from a webhook"
        );
    }

    /// **The trace is journaled before it is sent, and once per execution.**
    ///
    /// The same order `docs/durability.md` §3.7 states for a webhook, read off
    /// the seam that ships a trace: the guard that a settle is once per
    /// execution comes first, then the body is serialized, then the intent goes
    /// down — and nothing here attempts anything, because the caller decides
    /// whether it is a process that can wait out a schedule.
    #[test]
    fn a_trace_is_guarded_then_journaled_and_never_sent_from_the_intent() {
        let ship = SOURCE
            .split_once("export async function shipTrace(")
            .expect("`src/delivery.ts` ships a settled trace")
            .1
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        let guarded = ship
            .find("exportedAlready(")
            .expect("a settle is once per execution, and the ledger is what says so");
        let journaled = ship
            .find("intendExport(")
            .expect("…and the intent is recorded");
        assert!(
            guarded < journaled,
            "the once-per-execution guard is asked **before** a second row is opened, or a \
             recovered execution ships its trace twice under two delivery ids"
        );
        assert!(
            !ship.contains("workDelivery("),
            "`shipTrace` attempts the delivery itself, so the hook that closes a lifecycle row \
             would wait on a collector (PRD resolved q50: never blocking the run it describes)"
        );
    }

    /// **A detached delivery's envelope is journaled, never sent from the
    /// intent, and never guarded as once-only** (PRD resolved q64).
    ///
    /// The sibling above, read for the sink's second event class, and its one
    /// deliberate difference. An execution's export is once per execution because
    /// a settle is; a detached delivery is re-run by its parent's recovery and
    /// re-ships on that settlement — at-least-once, deduped by a receiver on
    /// `(parent_execution, idempotency_key)` — so a guard here would be the
    /// runtime deciding a dedupe the ruling gives to the receiver, and would be
    /// wrong besides: a map node's own `retry:` re-issues a delivery under the
    /// same key, and that is a delivery the ledger has to carry.
    #[test]
    fn a_detached_envelope_is_journaled_unguarded_and_never_sent_from_the_intent() {
        let ship = SOURCE
            .split_once("export async function shipDetachedTrace(")
            .expect("`src/delivery.ts` ships a detached delivery's envelope")
            .1
            .split_once("\n}\n")
            .expect("…in a function with a closing brace")
            .0;
        assert!(
            ship.contains("intendExport(") && ship.contains("detachedTraceDocument("),
            "a detached delivery's envelope is the runtime's one writer of it, journaled through \
             the one export writer"
        );
        assert!(
            !ship.contains("exportedAlready("),
            "a detached delivery's envelope is guarded as once-only, so a recovered delivery \
             never re-ships (PRD resolved q64: at-least-once across recovery)"
        );
        assert!(
            !ship.contains("workDelivery("),
            "`shipDetachedTrace` attempts the delivery itself, so a settling delivery would \
             wait on a collector"
        );
    }
}
