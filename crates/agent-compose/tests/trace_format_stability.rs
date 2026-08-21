//! The trace format, pinned against real runs (`docs/trace.md`).
//!
//! `crates/compose-core/tests/trace_format_inventory.rs` is the other half of
//! this pair, and the two answer different questions. That one reads the emitted
//! `src/runtime.ts` and asks whether the **document** covers what the code
//! declares. This one runs compiled graphs against the mock provider and asks
//! what a reader actually receives — which is not the same question: a field
//! declared and never recorded, a presence rule the document states and the
//! runtime does not keep, and a rename that moved both the declaration and the
//! prose in one commit are all invisible to a source-level check.
//!
//! # What a snapshot here holds
//!
//! The trace **document** a run wrote to disk, with its volatile values
//! redacted: the execution id becomes `<execution-id>` wherever it appears —
//! including inside the idempotency keys, where the remainder is the instance
//! path (grammar §9.4) and is exactly what should stay legible — the built
//! project's own directory becomes `<project>`, and a wall clock reading becomes
//! `<instant>`, which pins that the key is there and holds an RFC 3339 instant
//! without pinning the second it was written in. Everything else is kept,
//! because everything else is deterministic: the fixtures script every model
//! answer, and `docs/trace.md` fixes the order of entries, edge decisions and
//! dispatch records so that two runs of one composition produce one document.
//!
//! So an unannounced rename or removal is a diff in a pull request. A snapshot
//! that changes is not by itself a failure — `docs/trace.md` §10.2 lists the
//! changes that are compatible — it is a change to a public surface that has to
//! be looked at, which is what a golden is for.
//!
//! ```sh
//! INSTA_UPDATE=always cargo test -p agent-compose --test trace_format_stability
//! ```
//!
//! # Why these seven runs
//!
//! Between them they reach every record **type** the format has, every member of
//! `TraceDocument.status`, and the entry shapes a reader meets first: a bounded
//! cycle for guarded edges, budgets and an `else:` escape; a routed fan-out for
//! dispatch records, variants, idempotency keys and a nested subflow trace; a
//! store round trip for read-replay and write-dedupe records, and for the
//! tool-invoked write that carries neither key nor dedupe flag; a failover for a
//! model call that was served by its second member; a spent route for a
//! **failed** run — the shape a reader most often opens a trace for, and the one
//! whose rules (no writes, routing only where routing failed) exist nowhere
//! else; a run that ended holding a `human` pause, for the `human` record
//! and the `"interrupted"` document status version `2` introduced (§10.3.1);
//! and a flow attached as a tool and called twice, for the tool-call record and
//! the second dispatch-record carrier version `3` introduced (§10.3.2).
//!
//! A run added here is what keeps that first sentence true: the count is a claim
//! about coverage, so a record type or a status member added to the format
//! without a run that reaches it is a claim this file stopped keeping.
//!
//! It is not every *shape* the format admits, and the header should not be read
//! as claiming so: no snapshot here holds a `"skipped"` entry, a `fallback`
//! entry, a no-viable-route entry, a dispatch that skipped, failed or detached,
//! or the `"$default"` route sigil. `TraceEntry.fallback`'s **expiry** form is
//! in that list too — the widening version `2` made, where the key names an
//! `on_timeout:` route rather than an `on_error:` one — because reaching it
//! needs a resume surface, which `agent-compose run` is not. Those are held by
//! `tests/compiled_graph_acceptance.rs`, which asserts about them by name rather
//! than by shape — the expiry form by
//! [`an_expired_wait_takes_its_route_and_refuses_the_answer_that_arrives_after_it`]
//! for a route naming a node and by
//! [`a_wait_that_expires_into_end_retires_its_branch_and_completes_the_execution`]
//! for the `"__end__"` spelling — the two files divide the surface, and a run
//! added here is worth adding when a shape has no home in either. The first two
//! are reached
//! here all the same, by the §3 rule below rather than by a snapshot: a rule
//! about a field on every entry is not kept by pinning the entries that happen
//! to be in a golden.
//!
//! Each goes through `agent-compose run`, so what is snapshotted is the file a
//! reader is handed rather than an in-process value a test could shape for
//! itself.
//!
//! # …and the promises a snapshot cannot make
//!
//! Seven claims of `docs/trace.md` are about a *rule* rather than about a shape,
//! and each is asserted directly, because a snapshot of a document that happens
//! to satisfy a rule would go on passing after the rule was dropped: that the
//! third delivery surface carries the version beside its trace **and only
//! beside it** (§1), that both halves of a routing record are in declaration
//! order even where a router cannot decide in that order (§4), that a write's
//! `idempotencyKey` and `deduped` are a store-op **node**'s and not an agent
//! tool's (§6), that an activity's failure names the `${ENV}` reference its
//! author wrote rather than the value it resolved to (§11.1), that
//! `TraceEntry.error` is `<error name>: <message>` on **every** entry that
//! carries it (§3) — which is three sites of the emitted runtime rather than
//! one, so [`document`] holds every snapshot run to it and
//! [`a_failure_a_run_survived_carries_its_class_like_one_that_ended_a_run`]
//! reaches the two no snapshot here does — that an edge decision carries a
//! `reason` for the three decisions §4.1 tabulates and for no others, which
//! [`document`] also holds every snapshot run to, because §10.1 makes a
//! presence column a promise and a field appearing where the document says it
//! does not is that promise broken — and that a `human` record is on the node
//! that held the wait and on no node above it (§3), which
//! [`document`] holds every snapshot run to as well and
//! [`a_pause_is_recorded_on_the_node_that_held_it_and_on_no_node_above_it`]
//! reaches at the two constructs a pause can be nested under.
//!
//! Two of the seven ride on a run this file already makes rather than on a run
//! of their own: the §6 pair on the store run — a presence rule is a claim about
//! a record the snapshot already holds, and the two are asserted before
//! `assert_snapshot!` so a change to the rule fails as itself rather than as a
//! diff in a document — and §4.1's presence rule on
//! [`a_routing_records_edges_and_targets_are_both_in_declaration_order`]'s
//! composition, which is the only one here whose guard answers `false` at all.

// See the note on the same line in `tests/compiled_graph_acceptance.rs`: a test
// target is a crate root, so the shared harness is reached by path.
#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::time::Duration;

use mock_provider::{Client, MockProvider, Outcome, Script, ToolCall};
use serde_json::{Value, json};

/// `model.smart` in every fixture that has one.
const SONNET: &str = "claude-sonnet-4-6";
/// `model.fast`.
const HAIKU: &str = "claude-haiku-4-5";

/// The trace document a run wrote, as a snapshot reads it.
///
/// Three substitutions, and only three, because only three things about a trace
/// differ between runs of one composition:
///
///  * the **execution id**, which is minted per run. It is replaced wherever it
///    appears rather than only at `execution_id`, which is the point: an
///    idempotency key is the id followed by an instance path (grammar §9.4), and
///    substituting the id leaves the path — the part worth reading — in the
///    snapshot;
///  * the **project directory**, which a scratch build puts under a temporary
///    path. Nothing in the document holds one today; it is redacted anyway, so a
///    field that starts carrying a path lands in the snapshot as `<project>`
///    instead of as a value that changes every run;
///  * an **instant**, which is a wall clock reading. §3.4's `pausedAt`,
///    `expiresAt` and `settledAt` are the fields of the format that carry one,
///    and a snapshot holding any of them would fail on the second run — so the
///    *shape* is what is pinned: the key is there, with a value that was an
///    RFC 3339 instant, which is exactly the promise §3.4's table makes about
///    it. Recognized **by key** ([`INSTANTS`]) rather than by what a value looks
///    like: a redaction that fired on any 24-character date-shaped string would
///    hide a *datum* that happens to be spelled like a timestamp — a store's
///    answer, a model's completion, an error's text — and hiding data is the one
///    thing a golden must not do. What a key-based rule costs is that a field
///    which starts carrying a clock reading has to be added here; what it buys
///    is that the redaction never reaches beyond the fields the document says
///    hold one.
fn document(run: &harness::Run) -> String {
    let held = run.trace_document();
    every_entrys_error_is_in_one_shape(&held);
    every_edge_decisions_reason_follows_its_rule(&held);
    every_human_record_is_on_the_node_that_paused(&held);
    let execution = held["execution_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the trace document names its execution: {held}"))
        .to_string();
    // `<project>/.agent-compose/traces/<file>` is where the run wrote it, so the
    // project root is the path with those three components removed.
    let path = run.trace_path();
    let project = std::path::Path::new(&path)
        .ancestors()
        .nth(3)
        .map(|root| root.display().to_string())
        .unwrap_or_default();

    let mut redacted = held;
    redact(&mut redacted, &execution, &project);
    serde_json::to_string_pretty(&redacted).expect("the trace document serializes")
}

/// Every key of the format that carries a wall clock reading, enumerated from
/// `docs/trace.md`.
///
/// §3.4's pause record is the whole list, and it is the only record in the
/// document with a clock in it: `pausedAt`, `expiresAt` and `settledAt`.
///
/// Both spellings are here because §2's key seam runs through this list. An
/// entry is a runtime record and spells its keys `camelCase`; a *document*
/// spells its own `snake_case`, and the status route a `serve` report comes off
/// publishes a pending pause as `paused_at`/`expires_at` (PRD 5.11). Only the
/// `camelCase` three reach a snapshot here today — every run in this file goes
/// through `agent-compose run`, which writes the trace document — so the other
/// two are stated against the format rather than against the surface, and cost
/// nothing while no key of that spelling appears. `settled_at` is deliberately
/// absent: a settled wait is not a pending one, so no published pause has ever
/// carried it.
const INSTANTS: &[&str] = &[
    "pausedAt",
    "paused_at",
    "expiresAt",
    "expires_at",
    "settledAt",
];

/// Replace the three volatile values everywhere in `value`.
///
/// An instant is replaced **by its key** rather than by what the value looks
/// like, and the shape is asserted instead of being the trigger: a value under
/// one of [`INSTANTS`] that is not an RFC 3339 instant is a broken promise of
/// §3.4's table, and failing on it is the point. Everything else — including a
/// datum that happens to be spelled like a timestamp, which a store's `answer`
/// or a model's completion can hold — keeps its value and reaches the snapshot.
fn redact(value: &mut Value, execution: &str, project: &str) {
    match value {
        Value::String(text) => {
            let mut held = text.replace(execution, "<execution-id>");
            if !project.is_empty() {
                held = held.replace(project, "<project>");
            }
            *text = held;
        }
        Value::Array(items) => {
            for item in items {
                redact(item, execution, project);
            }
        }
        Value::Object(fields) => {
            for (key, held) in fields.iter_mut() {
                if INSTANTS.contains(&key.as_str()) {
                    assert!(
                        held.as_str().is_some_and(is_instant),
                        "`{key}` carries an RFC 3339 instant (`docs/trace.md` §3.4): {held}"
                    );
                    *held = Value::String("<instant>".to_string());
                    continue;
                }
                redact(held, execution, project);
            }
        }
        _ => {}
    }
}

/// Whether `text` is the instant `JSON.stringify(new Date(…))` writes.
///
/// `YYYY-MM-DDTHH:MM:SS.sssZ`, which is the one spelling the emitted runtime
/// produces (`toISOString`) and the one `docs/trace.md` §3.4 names. Matched
/// tightly, because it is what an [`INSTANTS`] key is *held to* rather than what
/// selects a value for redaction: a field of that table that started answering
/// with a second-resolution stamp, or with a local-time one, would be a change
/// to a published surface.
fn is_instant(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 24 {
        return false;
    }
    let digits = [0, 1, 2, 3, 5, 6, 8, 9, 11, 12, 14, 15, 17, 18, 20, 21, 22];
    let dashes = [
        (4, b'-'),
        (7, b'-'),
        (10, b'T'),
        (13, b':'),
        (16, b':'),
        (19, b'.'),
        (23, b'Z'),
    ];
    digits.iter().all(|at| bytes[*at].is_ascii_digit())
        && dashes.iter().all(|(at, held)| bytes[*at] == *held)
}

/// Every `TraceEntry` in `document`, in document order — the entries a subflow
/// nested under `inner` and a dispatched instance's under its record included.
///
/// An entry is told from the other records by `traversal`, which only an entry
/// carries: a dispatch record has an `outcome` and an `error` of its own, and it
/// is not what §3 speaks about.
fn entries(document: &Value) -> Vec<&Value> {
    let mut found: Vec<&Value> = Vec::new();
    walk_entries(document, &mut found);
    found
}

fn walk_entries<'a>(value: &'a Value, found: &mut Vec<&'a Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                walk_entries(item, found);
            }
        }
        Value::Object(fields) => {
            if fields.contains_key("traversal") {
                found.push(value);
            }
            for held in fields.values() {
                walk_entries(held, found);
            }
        }
        _ => {}
    }
}

/// Every `TraceEntry.error` in `document`, node-qualified, nested entries
/// included.
fn entry_errors(document: &Value) -> Vec<(String, String)> {
    entries(document)
        .into_iter()
        .filter_map(|entry| {
            let node = entry["node"].as_str()?;
            let error = entry["error"].as_str()?;
            Some((node.to_string(), error.to_string()))
        })
        .collect()
}

/// Every entry of `document` that carries a `human` record, as `(flow, node)`.
fn nodes_holding_a_pause(document: &Value) -> Vec<(String, String)> {
    entries(document)
        .into_iter()
        .filter(|entry| entry.get("human").is_some())
        .map(|entry| {
            let flow = entry["flow"]
                .as_str()
                .unwrap_or_else(|| panic!("an entry names the flow its node belongs to: {entry}"));
            let node = entry["node"]
                .as_str()
                .unwrap_or_else(|| panic!("an entry names its node: {entry}"));
            (flow.to_string(), node.to_string())
        })
        .collect()
}

/// `docs/trace.md` §3: a node that is not a `human` node never carries `human`.
///
/// A rule rather than a shape, and one a snapshot is a poor keeper of, because
/// the entries a `human` record can wrongly reach are the ones nested runs put
/// *outside* the flow a snapshot was written for. It is checkable without
/// knowing which nodes a composition declares as `human`, because §3 gives three
/// other keys the same kind of presence rule over a **different** kind of node:
/// `inner` is a `flow:` node's, `dispatches` is a `map` node's, and
/// `toolDispatches` is an `agent:` node's (§5). A node is one kind, so an entry
/// carrying `human` beside any of them is an entry claiming to be two — and that
/// is exactly the shape the mistake takes, since a pause is reached below one of
/// those three constructs or not nested at all.
///
/// The failure it guards is a real one and is invisible to every other check
/// here: an error raised inside a subflow or a dispatched instance travels up
/// the `cause` chain, so an enclosing node that recovers the pause from it
/// reports N+1 waits for one — each of the extra ones with no settlement, and so
/// each reading as a wait the run ended holding.
fn every_human_record_is_on_the_node_that_paused(document: &Value) {
    for entry in entries(document) {
        if entry.get("human").is_none() {
            continue;
        }
        assert!(
            entry.get("inner").is_none(),
            "a `flow:` node's entry carries a pause that belongs to a node inside \
             the instance it ran, which `docs/trace.md` §3 says it never does: \
             {entry}"
        );
        assert!(
            entry.get("dispatches").is_none(),
            "a `map` node's entry carries a pause that belongs to a node inside \
             an instance it dispatched, which `docs/trace.md` §3 says it never \
             does: {entry}"
        );
        assert!(
            entry.get("toolDispatches").is_none(),
            "an `agent:` node's entry carries a pause that belongs to a node \
             inside a flow its model called, which `docs/trace.md` §3 says it \
             never does: {entry}"
        );
    }
}

/// `docs/trace.md` §3: `TraceEntry.error` is `<error name>: <message>` on
/// **every** entry that carries it.
///
/// A rule rather than a shape, so a snapshot cannot hold it: a document whose
/// entries happen to satisfy the rule goes on matching its snapshot after the
/// rule is dropped. The field is written at three separate sites of the emitted
/// runtime — the entry a failure aborted the run at, the entry a `fallback:`
/// routed around, and the entry `on_error: skip` absorbed — and it is still one
/// field, so a reader handed it should not have to know which of the three wrote
/// it to know what it is holding.
fn every_entrys_error_is_in_one_shape(document: &Value) {
    for (node, error) in entry_errors(document) {
        let named = error.split_once(": ").is_some_and(|(class, rest)| {
            !class.is_empty()
                && !rest.is_empty()
                && class.chars().all(|c| c.is_alphanumeric() || c == '_')
        });
        assert!(
            named,
            "`{node}`'s entry carries an `error` that does not open with the class \
             that raised it, which `docs/trace.md` §3 says every entry's does: \
             {error:?}"
        );
    }
}

/// Every `EdgeDecision` in `document`, nested traces included.
///
/// A decision is told from the other records by carrying both `to` and `taken`:
/// an entry names its `node`, a dispatch record its `target`, and neither
/// carries a `taken`.
fn edge_decisions(document: &Value) -> Vec<Value> {
    let mut found: Vec<Value> = Vec::new();
    walk_edge_decisions(document, &mut found);
    found
}

fn walk_edge_decisions(value: &Value, found: &mut Vec<Value>) {
    match value {
        Value::Array(items) => {
            for item in items {
                walk_edge_decisions(item, found);
            }
        }
        Value::Object(fields) => {
            if fields.get("to").is_some_and(Value::is_string)
                && fields.get("taken").is_some_and(Value::is_boolean)
            {
                found.push(value.clone());
            }
            for held in fields.values() {
                walk_edge_decisions(held, found);
            }
        }
        _ => {}
    }
}

/// The `reason` `docs/trace.md` §4.1 gives this decision, read off the rest of
/// the record — `None` where its table describes none.
///
/// The three the table names are each recognizable from the other fields, which
/// is what makes the presence rule checkable at all: the shape of a decision
/// says which of the three rows it is, or that it is neither.
fn documented_reason(edge: &Value) -> Option<&'static str> {
    let taken = edge["taken"]
        .as_bool()
        .unwrap_or_else(|| panic!("an edge decision says whether it was taken: {edge}"));
    if edge.get("else").is_some() {
        // Row 3, or a catch-all that was taken — which nothing suppressed, and
        // which §4.1 therefore leaves without a reason.
        return (!taken).then_some("a guarded sibling was taken");
    }
    if edge.get("when").is_none() {
        // Row 1: no guard and no `else:` is an edge that is always taken.
        return Some("unconditional");
    }
    // A guarded edge. `value: false` is its own explanation; a guard that
    // answered `true` over an edge that was not taken is row 2, and is the one
    // decision the guard value does not account for.
    (edge["value"].as_bool() == Some(true) && !taken)
        .then_some("the `max_iterations` budget is spent")
}

/// `docs/trace.md` §4.1: an edge decision carries `reason` for the three
/// decisions its table names, and for no others.
///
/// A rule rather than a shape, and one a snapshot is especially poor at holding:
/// §10.1 makes each table's *presence* column something a reader may rely on, so
/// a `reason` that appeared on decisions the table does not describe would be a
/// promise broken rather than a field added — and a reader following §4.1 would
/// read `undefined`. Every document this file snapshots goes through here, and
/// [`an_edge_decision_carries_a_reason_only_where_the_document_names_one`]
/// reaches the two absences none of them holds.
fn every_edge_decisions_reason_follows_its_rule(document: &Value) {
    for edge in edge_decisions(document) {
        let held = edge.get("reason").and_then(Value::as_str);
        let expected = documented_reason(&edge);
        assert_eq!(
            held, expected,
            "this edge decision carries `reason: {held:?}` where `docs/trace.md` \
             §4.1's table gives it {expected:?}: {edge}"
        );
    }
}

/// A bounded cycle: guarded edges, the budget that ends it, and the `else:`
/// escape (`docs/trace.md` §4).
///
/// The model never approves, so only `max_iterations: 3` can end this run — and
/// the entries are where "what terminated a cycle" is readable: three passes
/// with `budget.used` climbing, then a fourth where the guard still answers
/// `true`, the budget is found spent, and the escape fires.
#[test]
fn a_bounded_cycles_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" })))
            .times(4)
            .matching("a research writer"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "again" })),
        )
        .times(4)
        .matching("meticulous technical reviewer"),
    ]);

    let Some(run) = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A routed fan-out: dispatch records, the variant each item carried, the
/// idempotency key each derived, and a subflow's nested trace
/// (`docs/trace.md` §5, §8).
#[test]
fn a_routed_fan_outs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a report" })),
        )
        .matching("a raw report"),
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "needs_human", "summary": "unclear", "severity": "high" },
                ],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "a patch" }))).matching("a.rs"),
        Script::new(HAIKU, Outcome::structured(json!({ "ticket": "a ticket" })))
            .matching("unclear"),
    ]);

    let Some(run) = harness::run(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A flow attached to an agent as a tool, called twice: the two record surfaces
/// version `3` added (`docs/trace.md` §5, §7.3, §10.3.2).
///
/// One run reaches all three halves of PRD §9.20's join at once — the dispatch
/// records under `toolDispatches`, each carrying an instance path no other call
/// derives (PRD §9.19) and the instance's whole trace, and the `toolCalls` entry
/// inside each model call linking to one by its key. It is also the only
/// snapshot here whose entries hold a nested trace reached from something other
/// than a `map`, which is what §8's two-place rule is about.
#[test]
fn a_flow_tool_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "the first passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the first line" })),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "the second passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the second line" })),
        ),
        Script::new(SONNET, Outcome::text("I have both lines.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says two lines" })),
        ),
    ]);

    let Some(run) = harness::run(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// The same call site with a **refused** call in front of the one that worked:
/// the third `ToolCallRecord.outcome` version `4` added (`docs/trace.md` §7.3,
/// §10.3.3).
///
/// Two shapes are pinned here that no other snapshot in this file can hold. The
/// refused record is one of them — an `outcome: "refused"` whose `error` carries
/// the sentence the model was handed under §3's `<error name>:` envelope
/// (`ToolCallRefused: …`, which the model's own copy does not carry — the two are
/// pinned against each other in `compiled_graph_acceptance`), with neither
/// `instance` nor `result` beside it, which is
/// the absence §7.3 makes a record in its own right. The other is what follows
/// it: a record after a refusal *exists*, which is the whole of how version `4`'s
/// third member differs from `"failed"`, and the instance the corrected call ran
/// is keyed `condense/0` because a refused call spends **no** ordinal — grammar
/// §9.4 counts a flow-tool's invocations, and this one reached no flow (D119).
///
/// A snapshot rather than an assertion because the claim is about the document's
/// shape: a release that started writing `result: null` on a refusal, or that
/// stopped writing `error`, would be a reviewable diff here rather than a
/// discovery made downstream (§10.4).
#[test]
fn a_refused_tool_calls_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `flow.condense` declares `passage: { min_length: 1 }`.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("condense", json!({ "passage": "" }))]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "the first passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the first line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::run(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A store round trip: a read's recorded answer, a write's key and dedupe flag,
/// and both of PRD 5.8's consumption surfaces (`docs/trace.md` §6).
///
/// The flow's own `store:` nodes supply the `via: "node"` records; the agent in
/// the middle of it calls **synthesized store tools**, which is the other
/// surface — "recorded as tool calls" in PRD 5.8's own words, and a record the
/// provider transcript alone could not stand in for.
///
/// The agent both reads and **writes** through that surface, because the two are
/// not the same record: §6 gives `idempotencyKey` and `deduped` to a store-op
/// *node*'s write and to nothing else, and a fixture whose only tool call was a
/// read would leave the whole of that rule to prose.
#[test]
fn a_store_using_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    // `agent.grounded` has stores attached, so its tools open a loop: the calls
    // that offer them pin nothing, and the pinned output call ends it.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "prefs_get",
                json!({ "key": "preferences" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "prefs_set",
                json!({
                    "key": "preferences",
                    "value": { "theme": "dark", "verbosity": "high" },
                }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    let Some(run) = harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    // §6's presence rule for the two fields a write carries, asserted before the
    // snapshot so a change to it fails as itself rather than as a diff: a write
    // the *agent* made through a synthesized tool (grammar §11.5) has neither,
    // and grammar §9.4 is why — it names the store-op node catalog (§11.4) as
    // the carrier, and a tool call's outcome goes straight back to the model
    // that asked for it, so there is no unobserved effect for a key to stand in
    // for.
    let tooled: Vec<Value> = run
        .entries("ask")
        .iter()
        .flat_map(|entry| {
            entry["stores"]
                .as_array()
                .cloned()
                .unwrap_or_else(|| panic!("the agent's entry carries its store ops: {entry}"))
        })
        .filter(|record| record["effect"] == "write")
        .collect();
    assert_eq!(
        tooled.len(),
        1,
        "the agent wrote through exactly one synthesized tool: {tooled:?}"
    );
    for record in &tooled {
        assert_eq!(record["via"], "tool", "{record}");
        assert!(
            record.get("idempotencyKey").is_none(),
            "a tool-invoked write carries no idempotency key (`docs/trace.md` §6): {record}"
        );
        assert!(
            record.get("deduped").is_none(),
            "…and no dedupe flag, which travels with the key: {record}"
        );
    }

    // …and the node writes beside it do carry both, so the rule is a
    // distinction rather than a field the runtime stopped recording.
    let noded: Vec<Value> = run
        .trace()
        .iter()
        .flat_map(|entry| entry["stores"].as_array().cloned().unwrap_or_default())
        .filter(|record| record["effect"] == "write" && record["via"] == "node")
        .collect();
    assert!(
        !noded.is_empty(),
        "the flow's `store:` nodes wrote: {noded:?}"
    );
    for record in &noded {
        assert!(
            record["idempotencyKey"].is_string() && record["deduped"].is_boolean(),
            "a store-op node's write carries both: {record}"
        );
    }

    insta::assert_snapshot!(document(&run));
}

/// A model call served by its second route member (`docs/trace.md` §7).
///
/// PRD 5.9's own sentence — "served by `model.fast`, fallback #1" — as the
/// record that carries it: the `model.*` the composition named, the member that
/// answered, its ordinal, and the condition the member it replaced refused with.
#[test]
fn a_failed_over_model_calls_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "42" }))),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    insta::assert_snapshot!(document(&run));
}

/// A run that produced no answer (`docs/trace.md` §9).
///
/// Both members of the route refuse, so the node fails and the run ends. The
/// document is the one a reader opens a trace for: `status: "failed"` with the
/// error that stopped it, and a final entry carrying no `writes`, no `routing`,
/// and the whole spent ladder in `models`.
#[test]
fn a_failed_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::rate_limit()),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.failed();
    insta::assert_snapshot!(document(&run));
}

/// A run that ended holding a pause (`docs/trace.md` §3.4, §9).
///
/// The third thing a document's `status` can say, and the record version `2`
/// added, in the one run that produces both without a resume surface: an
/// `agent-compose run` that reaches a `human` node stops there (grammar §8.7),
/// so the document is `status: "interrupted"` and the node's entry carries a
/// `human` record with a `pausedAt` and no settlement — the one case §3.4's
/// presence column says `settled`/`settledAt` are absent in, which is a promise
/// nothing else here holds.
#[test]
fn an_interrupted_runs_trace_document_keeps_its_shape() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(run) = harness::run(
        "http-trigger",
        "flow.assisted",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.failed();
    insta::assert_snapshot!(document(&run));
}

/// A pause is recorded on the node that held it and on no node above it
/// (`docs/trace.md` §3).
///
/// The presence rule §3's `human` row ends on — "a node that is not a `human`
/// node never carries it" — read at the three places a pause is reached from
/// *under* another node, which is where it can be broken without any snapshot
/// here changing. `flow.assisted`, which
/// [`an_interrupted_runs_trace_document_keeps_its_shape`] pins, pauses at the
/// top level of the flow it triggered, so its document has no enclosing entry
/// for a wait to leak onto; these three have one each. `flow.patient` wraps the
/// pause in a `flow:` node, `flow.batch` dispatches it from a `map`, and
/// `flow.decide` reaches it through a flow a **model** called (grammar §5.4),
/// and the error that carries the interrupt out of the run passes through each
/// on its way — which is the reason a node that held no wait can end up
/// reporting one.
///
/// What a leak would look like to a reader is why this is a `must` rather than
/// tidiness: one wait would be reported by N+1 entries, and the extra copies
/// carry no `settledAt`, so each reads as §3.4's one settlement-free case — a
/// wait the run ended holding — on a node that never waited for anything.
#[test]
fn a_pause_is_recorded_on_the_node_that_held_it_and_on_no_node_above_it() {
    let provider = MockProvider::start().expect("a loopback port");

    // One construct out: a `flow:` node whose subflow pauses.
    let Some(wrapped) = harness::run(
        "http-trigger",
        "flow.patient",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    wrapped.failed();
    let held = wrapped.trace_document();
    every_human_record_is_on_the_node_that_paused(&held);
    assert_eq!(
        nodes_holding_a_pause(&held),
        [("flow.sign_off".to_string(), "sign".to_string())],
        "`wrap` ran the instance that paused and held no wait of its own: {held}"
    );

    // …and the other: a `map` dispatching a flow that pauses. One item, because
    // what is asserted is which entries carry the record rather than how many
    // instances a fan-out gets as far as parking before the first interrupt ends
    // the node.
    let Some(dispatched) = harness::run(
        "http-trigger",
        "flow.batch",
        &[("questions", r#"["what is it?"]"#)],
        &provider,
    ) else {
        return;
    };
    dispatched.failed();
    let held = dispatched.trace_document();
    every_human_record_is_on_the_node_that_paused(&held);
    assert_eq!(
        nodes_holding_a_pause(&held),
        [("flow.sign_off".to_string(), "sign".to_string())],
        "`fan` dispatched the instance that paused and held no wait of its own: \
         {held}"
    );

    // …and the third: an `agent:` node whose model called a flow that pauses.
    // The enclosing entry here is the one that also carries the loop's own
    // account — the dispatch record for the instance holding the question, and
    // the model call whose `toolCalls` names it (PRD §9.20) — so it is the entry
    // with the most to leak and the one a reader would most readily believe.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "sign",
            json!({ "draft": "a drafted answer" }),
        )]),
    ));
    let Some(called) = harness::run(
        "flow-as-tool",
        "flow.decide",
        &[("question", "ship it?")],
        &provider,
    ) else {
        return;
    };
    called.failed();
    let held = called.trace_document();
    every_human_record_is_on_the_node_that_paused(&held);
    assert_eq!(
        nodes_holding_a_pause(&held),
        [("flow.sign".to_string(), "approve".to_string())],
        "`draft` ran the loop that started the instance that paused, and held no \
         wait of its own: {held}"
    );

    // The rule is an absence, so it is worth one positive beside it: the entry
    // that *does* carry the record is the one the run stopped at, and it carries
    // the settlement-free shape §3.4 describes.
    let paused = entries(&held)
        .into_iter()
        .find(|entry| entry.get("human").is_some())
        .unwrap_or_else(|| panic!("the run ended holding a pause: {held}"));
    assert!(
        paused["human"]["pausedAt"].is_string()
            && paused["human"]["settled"].is_null()
            && paused["human"]["settledAt"].is_null(),
        "{paused}"
    );
}

/// Both halves of a routing record are in **declaration** order, including the
/// one edge shape whose declaration order a router cannot decide in
/// (`docs/trace.md` §4).
///
/// `flow.branch_order` declares three edges out of one node: a guarded one that
/// answers `false`, then an `else:`, then an unconditional one. An `else:` edge
/// is not decidable until every guarded sibling has been (grammar §7.3 rule 4),
/// so a record built as the router decides puts the `else:` edge last — after
/// the unconditional edge it is declared before. §4 promises `edges` **and**
/// `targets` in the order the file declares them, and this is the composition
/// that can tell the difference.
#[test]
fn a_routing_records_edges_and_targets_are_both_in_declaration_order() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(run) = harness::run("activities", "flow.branch_order", &[], &provider) else {
        return;
    };
    run.succeeded();

    let fork = run.entries("fork");
    let routing = &fork[0]["routing"];
    let edges = routing["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("the entry carries its edge decisions: {}", fork[0]));
    let decided: Vec<(&str, bool)> = edges
        .iter()
        .map(|edge| {
            (
                edge["to"].as_str().expect("a target"),
                edge["taken"].as_bool().expect("a verdict"),
            )
        })
        .collect();
    assert_eq!(
        decided,
        [("guarded", false), ("spare", true), ("always", true)],
        "the decisions are in declaration order, not decision order: {routing}"
    );
    assert_eq!(
        routing["targets"],
        json!(["spare", "always"]),
        "…and so are the targets, which is the half a reader reconstructs branch \
         order from: {routing}"
    );
}

/// An edge decision carries `reason` for the three decisions `docs/trace.md`
/// §4.1 names, and for none of the others.
///
/// The **absences** are what needs a fixture of its own. The five snapshot runs
/// above reach all three presences — `"unconditional"`, the budget-spent reason
/// and the else-suppressed one — and not one of them holds a guarded edge that
/// answered `false` or an `else:` edge that was taken, so a runtime that started
/// writing a `reason` on every untaken edge would leave every snapshot here
/// matching. §10.1 makes the presence column something a reader may rely on, and
/// a reader taking §4.1 at its word on an untaken edge reads `undefined` from a
/// field the table promised: the same class of break as a rename, which is what
/// this file exists to catch.
///
/// `flow.branch_order` declares all three shapes out of one node — a guarded
/// edge that answers `false`, an `else:` edge nothing suppresses, and an
/// unconditional one — so one run reaches both absences and the presence beside
/// them.
#[test]
fn an_edge_decision_carries_a_reason_only_where_the_document_names_one() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(run) = harness::run("activities", "flow.branch_order", &[], &provider) else {
        return;
    };
    run.succeeded();

    let held = run.trace_document();
    every_edge_decisions_reason_follows_its_rule(&held);

    let fork = run.entries("fork");
    let routing = &fork[0]["routing"];
    let edges = routing["edges"]
        .as_array()
        .unwrap_or_else(|| panic!("the entry carries its edge decisions: {}", fork[0]));
    // The shapes first, because the reasons below say nothing without them:
    // whichever way `reason` went, these are the decisions §4.1 speaks about.
    assert_eq!(
        edges[0]["value"],
        json!(false),
        "`guarded` is the guarded edge that answered `false`: {routing}"
    );
    assert_eq!(
        edges[1]["else"],
        json!(true),
        "`spare` is the `else:` catch-all, and no guarded sibling was taken: {routing}"
    );
    assert!(
        edges[2].get("when").is_none() && edges[2].get("else").is_none(),
        "`always` is the unconditional edge: {routing}"
    );

    let reasons: Vec<(&str, Option<&str>)> = edges
        .iter()
        .map(|edge| {
            (
                edge["to"].as_str().expect("a target"),
                edge.get("reason").and_then(Value::as_str),
            )
        })
        .collect();
    assert_eq!(
        reasons,
        [
            ("guarded", None),
            ("spare", None),
            ("always", Some("unconditional")),
        ],
        "only the decisions `docs/trace.md` §4.1 tabulates carry a `reason`; an \
         untaken edge is not by itself one of them: {routing}"
    );
}

/// A failed activity names the `${ENV}` reference its author wrote, never the
/// value it resolved to (`docs/trace.md` §11.1).
///
/// `tally` runs `${OPS_BIN}/false`, which exits 1 — outside the default
/// `expect_exit: [0]` — and its `on_error: skip` puts the message in a trace
/// entry a reader is handed. Grammar §4.3 class 2 makes the whole `exec:` block
/// interpolable, so a composition may perfectly well point a command or a URL at
/// a secret; §11.1 is the promise that the resolved value stays out of the
/// document, and this is that promise against a real run.
#[test]
fn a_failed_activity_names_its_env_reference_rather_than_the_resolved_value() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "fast", "note": "a note" })),
    ));

    let Some(run) = harness::run(
        "activities",
        "flow.pipeline",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let tally = run.entries("tally");
    let error = tally[0]["error"]
        .as_str()
        .unwrap_or_else(|| panic!("a skipped node's entry says what failed: {}", tally[0]));
    assert_eq!(
        error,
        "NodeFailure: flow.pipeline node `tally` failed: Error: `${OPS_BIN}/false` \
         exited 1, which is outside `expect_exit: [0]`",
        "the message quotes the command as the composition spells it, in the one \
         `<error name>: <message>` shape §3 gives the field on every entry that \
         carries it — a failure the node absorbed reads exactly as one that ended \
         the run does"
    );

    // The harness resolves `OPS_BIN` to `/bin`, so the command really ran; what
    // the document must not hold is that value.
    every_entrys_error_is_in_one_shape(&run.trace_document());
    let document = run.trace_document().to_string();
    assert!(
        !document.contains("/bin/false"),
        "no resolved `${{ENV}}` value is in the trace document: {document}"
    );
}

/// The two entries a failure the run **survived** lands on carry `error` in §3's
/// one shape, like the entry a failure ended the run on.
///
/// The five snapshot runs above reach `error` by one path only — the aborting
/// entry of a spent route — and [`document`] holds each of them to the rule. The
/// other two paths are here, because they are separate sites in the emitted
/// runtime and a rule kept at one of three is not a rule: `flow.pipeline`'s
/// `tally` has its `exec:` failure absorbed by `on_error: skip`, and
/// `flow.deadline`'s `slow` runs out of its `timeout:` and is routed around by
/// `on_error: { fallback: rescue }`. Both runs **succeed** — which is the point:
/// a reader reconstructing what happened inside a run that finished is reading
/// exactly this field.
#[test]
fn a_failure_a_run_survived_carries_its_class_like_one_that_ended_a_run() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "fast", "note": "a note" })),
    ));

    let Some(skipped) = harness::run(
        "activities",
        "flow.pipeline",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    skipped.succeeded();
    let held = skipped.trace_document();
    every_entrys_error_is_in_one_shape(&held);
    assert_eq!(
        entry_errors(&held)
            .iter()
            .map(|(node, _)| node.as_str())
            .collect::<Vec<_>>(),
        ["tally"],
        "the absorbed failure is the one entry of this run that carries the field: \
         {held}"
    );

    let Some(rescued) = harness::run("activities", "flow.deadline", &[], &provider) else {
        return;
    };
    rescued.succeeded();
    let held = rescued.trace_document();
    every_entrys_error_is_in_one_shape(&held);
    let errors = entry_errors(&held);
    let [(node, error)] = errors.as_slice() else {
        panic!("the node the `fallback:` routed around is the one that carries it: {held}");
    };
    assert_eq!(node, "slow");
    assert!(
        error.starts_with("NodeFailure: ") && error.contains("timed out"),
        "…and it names the class in front of the budget that ended it: {error:?}"
    );
}

/// The third delivery surface keeps §1's rule: `serve` reports the version
/// beside the trace, and neither before the run has stopped.
///
/// Snapshotting this one would pin the *app's* report rather than the format —
/// the same entries, wrapped in an execution's status — so what is asserted here
/// is the rule the other four cannot reach: that a surface with an optional
/// `trace` carries the version exactly when it carries a trace. The gate is the
/// trace's presence, not its length (§1.3): a run that failed inside the graph
/// having recorded nothing reports `trace: []` with the version beside it, and a
/// report carries neither key only where there is no trace at all.
///
/// **Both** halves, against reports the app really built. The negative half is
/// the one that is easy to fake: a `404` body is `unknownExecution`'s, which
/// never goes through the report builder at all, so its missing `trace_version`
/// would go on being missing however the builder was changed. So the assertion
/// is made against a `running` execution — the state §1 names — and the model's
/// answer is held back to put the run in it. `Outcome::after` is what makes that
/// a fact about the fixture rather than about scheduling luck: the status is
/// read while the provider is still holding the only answer the run is waiting
/// on.
#[test]
fn the_status_route_carries_the_version_beside_its_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    // Long enough that the status route below is answered while the run is still
    // waiting on it, and short enough not to dominate the suite. The flow has
    // one model call in it, so this is the whole of what it is waiting for.
    const HELD: Duration = Duration::from_secs(3);
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })).after(HELD),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let started = app
        .post_json("/direct-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // The negative half: a report the app built for an execution that has made
    // no trace yet carries neither key.
    let running = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        running["status"], "running",
        "the provider is still holding this run's only answer: {running}"
    );
    assert!(
        running["trace"].is_null(),
        "a running execution reports no trace at all — not an empty one \
         (`docs/trace.md` §1.3): {running}"
    );
    assert!(
        running["trace_version"].is_null(),
        "…and so no version, because the version travels with the trace: {running}"
    );

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert!(
        finished["trace"]
            .as_array()
            .is_some_and(|entries| !entries.is_empty()),
        "a settled execution reports the entries it made: {finished}"
    );
    assert_eq!(
        finished["trace_version"],
        json!(4),
        "…and the version that describes them, beside them (`docs/trace.md` §1): \
         {finished}"
    );

    // An id the app never started is answered by a different builder entirely —
    // a `404` with an `error` and nothing else — so this says nothing about the
    // rule above. It is here because a reader polling a status URL can be handed
    // it, and what it must not do is answer a version for a trace it has not
    // got.
    let unknown = app
        .get("/executions/exec_not_started")
        .expect("the status route answers");
    assert_eq!(unknown.status, 404, "{:?}", unknown.body);
    assert!(
        unknown.json()["trace_version"].is_null(),
        "a `404` names no trace version either: {:?}",
        unknown.body
    );
}
