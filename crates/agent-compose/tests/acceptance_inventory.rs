//! The completion inventory: every criterion PRD §7 promises of a *running*
//! compiled graph, mapped to the acceptance test that decides it and to whether
//! that test runs yet.
//!
//! M1 is the bulk of it and is what the shape below is built around. M3's first
//! bullet — durable execution — is here too, for the reason the file exists at
//! all: it is a promise about what a compiled graph does when it runs, and the
//! tests that decide it live in the same suite.
//!
//! `crates/compose-core/tests/static_check_inventory.rs` is the companion for M0, and this
//! file is the same idea one milestone on: the PRD's own sentences, transcribed
//! verbatim and made executable. The difference is what a row points at. M0's
//! rows point at diagnostic codes, because a static check is a thing the
//! compiler *says*; M1's point at acceptance tests, because codegen is a thing a
//! compiled graph *does* (CLAUDE.md: "each milestone defines its acceptance
//! criteria as runnable tests before implementation of that milestone begins").
//!
//! Four assertions keep it honest, and each covers a different way a plan rots:
//!
//! * the criteria are the PRD's own phrases, split from its own sentences, so a
//!   milestone that grows a promise fails here rather than shipping untested;
//! * every test a row names exists;
//! * every test in the suite is named by a row — the direction that catches a
//!   test drifting out of the plan rather than a plan drifting out of the suite;
//! * a row's status matches the test's `#[ignore]` attribute **exactly**, so
//!   un-ignoring a test is the only way to claim a criterion is met, and
//!   claiming it without the feature is a failure here.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// One acceptance criterion.
struct Criterion {
    /// Which of PRD §7 M1's bullets it comes from.
    bullet: Bullet,
    /// The PRD's own phrase, verbatim.
    phrase: &'static str,
    /// The tests in `tests/compiled_graph_acceptance.rs` that decide it, each with whether
    /// it runs yet.
    ///
    /// Status is per **test**, not per criterion: one criterion's tests can be
    /// unlocked by different features — "state models (incl. tagged unions via
    /// Zod)" needs the state model for one of its tests and Zod discriminated
    /// unions for the other — and a status that could not say so would either
    /// lie about one of them or force the reasons to be vague.
    tests: &'static [(&'static str, Status)],
}

/// Which sentence of PRD §7 M1 a criterion is transcribed from.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Bullet {
    /// "IR → deterministic LangGraph TypeScript: …"
    Codegen,
    /// "`agent-compose build`, `agent-compose run` (manual trigger), …"
    Commands,
    /// "Mock provider server + e2e harness: …"
    Harness,
    /// Not PRD §7 M1's own enumeration: CLAUDE.md's *Validation strategy*.
    Strategy,
    /// PRD §7 M3's first bullet: "**Durable execution**: …".
    Durability,
    /// PRD §7 M3's second bullet, its runtime half: "**Built-in tools** … and
    /// runtime bash/file tools opted into per agent node, bounded by root +
    /// timeout (resolved q31)".
    ///
    /// The *other* half of that bullet — provider-executed server tools — is
    /// filed under [`Self::Harness`], because what it changed was what a
    /// scripted answer has to be able to carry. This half changed what a
    /// compiled graph *does*, so it gets a heading of its own.
    BuiltinTools,
    /// PRD §7 M3's fourth bullet: "**HTTP-native events** (resolved q32–q36):
    /// …".
    HttpEvents,
}

/// Whether a criterion's tests run today.
enum Status {
    /// They run. This PR delivers what they test.
    Live,
    /// They are `#[ignore]`d, with this reason — verbatim, so the attribute and
    /// the inventory cannot drift apart.
    ///
    /// **No row is pending right now**, which is what the `expect` says: every
    /// test this file inventories runs. The arm stays because it is half of what
    /// `a_pending_criterion_names_what_must_land_first` decides — a row claimed
    /// live whose test is ignored, and a row claimed pending whose test is not —
    /// and deleting it would delete that half of the check along with the
    /// vocabulary the next criterion written ahead of its feature needs.
    /// `expect` rather than `allow` so that the day a pending row returns, the
    /// unfulfilled expectation is what removes this attribute, instead of a
    /// suppression outliving its reason.
    #[expect(dead_code, reason = "no acceptance test is ignored today")]
    Pending(&'static str),
}

/// PRD §7 M1's first bullet, decomposed into the ten items its own sentence
/// lists, in order.
const CODEGEN: &[Criterion] = &[
    // Four tests, because this criterion splits three ways and only the first
    // two are decidable now.
    //
    // The **emission** half has landed and is claimed here: `build` writes the
    // state model and the Zod discriminated unions, and the two live rows decide
    // that through the real command. The **execution** half — that those channel
    // specs reduce the way their policies say, and that the union refuses a tag
    // it does not declare — is decided on every `cargo test` by `compose-core`'s
    // `tests/generated_code_gates.rs`, which invokes the compiled graph under the
    // pinned LangGraph and runs the schema corpus through both columns of grammar
    // 3.8's table. It is not a row here because this file maps
    // `tests/compiled_graph_acceptance.rs`, and pointing a row at another crate's
    // test would make the status column mean two different things.
    //
    // What is left pending is the half that needs a *flow*: the declared defaults
    // read back as a flow's outputs, and a bad tag failing a run by name. Both
    // wait on commands this milestone has not built, which is what their reasons
    // say.
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "state models (incl. tagged unions via Zod)",
        tests: &[
            (
                "the_state_model_carries_every_channels_type_default_and_reduce_policy",
                Status::Live,
            ),
            (
                "a_tagged_union_output_is_emitted_as_a_discriminated_union_narrowed_per_variant",
                Status::Live,
            ),
            // What a reducer is *called in*: concurrent writers of one step land
            // in ascending node-id order, which grammar 7.6.4 clause 1 fixes and
            // the scheduler supplies.
            (
                "concurrent_writers_append_in_node_id_order_not_completion_order",
                Status::Live,
            ),
            (
                "state_channels_carry_their_declared_types_and_defaults",
                Status::Live,
            ),
            (
                "a_tagged_union_output_is_narrowed_per_variant_and_a_bad_tag_is_rejected",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "node fns",
        tests: &[
            (
                "an_agent_node_sends_its_prompt_input_and_output_schema",
                Status::Live,
            ),
            // The same criterion on the other HTTP surface. Three of grammar
            // 12.1's six provider kinds reach Chat Completions, so a node fn
            // that only works on Messages is half a node fn.
            (
                "an_agent_node_sends_its_prompt_input_and_output_schema_on_chat_completions",
                Status::Live,
            ),
            // What that surface's structured output cannot promise, and what it
            // still does: an `optional:` property costs `strict`, and the parse
            // is what holds the answer to the contract instead
            // (`codegen::runtime`'s divergence ledger).
            (
                "a_nested_optional_property_costs_the_strict_decoder_and_not_the_parse",
                Status::Live,
            ),
            // The other two of those three kinds. `azure_openai` and `openai`
            // differ from `openai_compatible` only in the request — auth header,
            // route, `api-version` query, `openai-organization` — so a node fn
            // that compiles them wrong is invisible to every assertion about
            // what a flow produced.
            (
                "each_chat_completions_kind_authenticates_and_routes_the_way_its_row_says",
                Status::Live,
            ),
            // The conditional half of the same row (grammar 12.1, Decision
            // D120): a provider that names a `base_url:` may hold no key, and
            // then the node fn must send no auth header rather than an empty
            // one. Only the transcript can tell those apart, and the mock does
            // not enforce it — a keyless request is a legal wire shape — so an
            // emitter that dropped the header for every provider would pass
            // every other test here, which is what the keyed third run closes.
            (
                "a_provider_with_no_key_sends_no_authentication_header_on_either_wire",
                Status::Live,
            ),
            // The other thing that surface can answer with: a refusal, which
            // carries its reason and is otherwise indistinguishable from a
            // `max_tokens` cut.
            (
                "a_refused_answer_carries_the_reason_the_model_gave",
                Status::Live,
            ),
            (
                "an_agent_node_bounds_its_tool_loop_at_max_tool_iterations",
                Status::Live,
            ),
            // What the loop does with a call the tool's own contract refuses
            // (Decision D119, PRD §9.22): it goes back to the model as an error
            // tool result, on both wire shapes, while a tool whose *execution*
            // failed still ends the node. The second of the two is what stops
            // the bound from becoming decorative — a model that never corrects
            // spends it and fails holding the last refusal.
            (
                "a_tool_definitions_input_refuses_on_the_chat_completions_wire_and_an_exit_code_does_not",
                Status::Live,
            ),
            (
                "a_tool_loop_that_never_corrects_spends_its_budget_and_fails_holding_the_last_refusal",
                Status::Live,
            ),
            // …and its twin, which is what keeps that message honest: the same
            // bound spent by a loop whose last call *worked* claims no refusal.
            (
                "a_tool_loop_whose_last_call_worked_spends_its_budget_claiming_no_refusal",
                Status::Live,
            ),
            // One answer, two calls, the first refused: every call of the answer
            // comes back to the model, because a request leaving either
            // unanswered is one both surfaces refuse (D119, `WIRE-NOTES` (18)).
            // The last two are that answer with the refused call naming a tool
            // the agent never offered — the one place the two wires give the
            // model different shapes for one answer, so each wire gets its own
            // test. Chat Completions splits it in two, a `tool` message for the
            // call the request may name and a `user` turn for the one it may
            // not; the Messages API carries both as `tool_result` blocks of one
            // turn. Either way it takes a *mixed* answer to observe the ordering.
            (
                "a_refused_call_does_not_stop_the_calls_beside_it_in_one_answer",
                Status::Live,
            ),
            // Grammar 6.1's fourth implementation binding, executed (PRD
            // resolved q48). Three rows because the claim is **parity** and
            // parity is three separate things: the node surface, the tool-loop
            // surface — where the comparison against an `exec:` twin is the
            // assertion — and the policy chain that governs both. A module
            // dispatched through a seam of its own could pass any one of them
            // alone while behaving unlike a tool at the other two.
            //
            // A fourth is about the one thing a module has that the other three
            // bindings cannot have wrong, because they run somewhere else: its
            // declared `env:` is the *call's*, and running in the graph's own
            // process is exactly what makes "and nothing else in this process
            // sees it" a claim worth executing (PRD resolved q49).
            (
                "a_module_tool_runs_at_a_function_node_and_reads_the_environment_it_declared",
                Status::Live,
            ),
            (
                "a_module_tools_declared_environment_reaches_the_call_and_no_subprocess_after_it",
                Status::Live,
            ),
            (
                "a_module_tool_in_a_loop_records_what_an_exec_tool_records_and_bounces_the_same_refusal",
                Status::Live,
            ),
            (
                "a_module_tool_is_retried_bounded_and_routed_around_like_every_other_binding",
                Status::Live,
            ),
            (
                "an_answer_mixing_an_unoffered_call_with_an_offered_one_is_answered_in_both_chat_completions_shapes",
                Status::Live,
            ),
            (
                "an_answer_mixing_an_unoffered_call_with_an_offered_one_is_answered_in_one_messages_turn",
                Status::Live,
            ),
            // What an agent node leaves behind for the next one: the implicit
            // `messages` channel of grammar 10.4 (PRD 5.7 tier 3), which is
            // state no composition declares and only the wire makes visible.
            (
                "an_agent_nodes_exchange_reaches_the_next_agent_nodes_request",
                Status::Live,
            ),
            // What the loop puts *back* on the wire: the model's own content
            // blocks, and — for an answer that carried none — a node error about
            // the answer rather than a request no surface accepts.
            (
                "a_loop_answer_is_replayed_verbatim_and_an_empty_one_stops_the_node",
                Status::Live,
            ),
            // …and how it hands over to the pinned call (PRD §9 resolved q52):
            // with one fixed user turn, because a forced `tool_choice` over a
            // history ending on the assistant is prefill against a forced call
            // and strict gateways refuse it. The turn is one request's shape, so
            // the second half of the row is that the next node never sees it.
            (
                "the_pinned_call_after_a_tool_loop_ends_on_the_turn_that_closes_it",
                Status::Live,
            ),
            // The other three kinds this milestone executes, which are not the
            // model's: an inline `exec:`, an inline `http:`, and a `function:`
            // over a `tool.*` (grammar 8.2, 8.3, 8.4).
            (
                "the_deterministic_node_kinds_run_and_decode_their_results",
                Status::Live,
            ),
            // The escape hatch of grammar 6.1, and the registration it costs.
            (
                "a_host_registered_function_runs_and_an_unregistered_one_says_so",
                Status::Live,
            ),
            // …and what a `tool.*`'s own `input:` does at that surface: a value
            // constraint grammar 8.4's arity-and-types check does not cover
            // fails the node, as a mismatch rather than as a refusal — the half
            // of Decision D119's split with no model on it.
            (
                "a_function_nodes_unfit_argument_fails_the_node_as_a_mismatch",
                Status::Live,
            ),
            // How those two inline kinds are *parameterised*: `input:` bindings
            // as the child's environment, a scalar one on its stdin, and the two
            // widened accepted-outcome lists that make a failure routable data
            // (grammar 8.2, 8.3, Decisions D84, D88).
            (
                "an_inline_nodes_parameters_reach_the_process_and_the_wire",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "routers with embedded CEL",
        tests: &[
            (
                "an_edge_guard_routes_on_the_source_nodes_structured_output",
                Status::Live,
            ),
            // The two rules of grammar 7.3 an enum guard alone does not reach:
            // multicast with a per-step join, and what a `skip` changes about
            // the routing algorithm (rule 6, Decision D97).
            (
                "a_multicast_fork_fires_every_true_edge_and_the_convergence_runs_once",
                Status::Live,
            ),
            (
                "a_skipped_node_routes_through_its_else_edge_and_writes_nothing",
                Status::Live,
            ),
            // A guard on an edge leaving `start`, which grammar 7.2 admits and
            // which has no node to be evaluated at.
            ("a_guarded_start_edge_decides_the_first_step", Status::Live),
            // *When* a guard is evaluated, which decides what it can see: its own
            // node's writes and not a concurrent sibling's (grammar 7.6 P1, read
            // per node — see `codegen::graph`'s ledger row).
            (
                "a_guard_sees_its_own_writes_and_not_a_concurrent_siblings",
                Status::Live,
            ),
            // Where a routing decision *goes*: the trace PRD 5.3 asks for,
            // including on the runs that fail — and the router's own failure,
            // grammar 7.3 rule 7, which no static check can reach.
            (
                "a_failed_runs_trace_holds_what_landed_and_the_node_it_stopped_at",
                Status::Live,
            ),
            // The other end of the same record: a run that quiesced, so its
            // trace is complete, and then could not materialize an output
            // (grammar 7.6.3, 10.1). The trace has to survive that too.
            (
                "a_run_that_quiesces_without_an_output_fails_carrying_its_whole_trace",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "bounded cycles",
        tests: &[
            (
                "a_bounded_cycle_leaves_through_its_escape_edge_when_the_budget_is_spent",
                Status::Live,
            ),
            // The same cycle in the *documented* project rather than a fixture,
            // with an agent carrying `tools:` inside the loop and a `model.*`
            // route bound to its first member — the two things `bounded-cycle`
            // cannot reach.
            (
                "the_review_loop_example_runs_its_cycle_against_the_mock_provider",
                Status::Live,
            ),
            // Grammar 7.4's *other* clause, which PRD 5.4 accepts and which is
            // not a static termination proof: a cycle whose only bound is a CEL
            // exit condition runs as long as its guard says, and the superstep
            // ceiling is what catches the one whose guard never goes false.
            (
                "a_cel_bounded_cycle_runs_the_passes_its_guard_asks_for",
                Status::Live,
            ),
            (
                "a_run_that_reaches_the_superstep_ceiling_says_which_bound_was_missing",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "homogeneous + discriminator-routed `map`→`Send` with index-tagged reducers",
        tests: &[
            (
                "a_homogeneous_map_dispatches_one_instance_per_item",
                Status::Live,
            ),
            (
                "a_routed_map_sends_each_variant_to_its_own_route",
                Status::Live,
            ),
            (
                "appended_results_are_ordered_by_source_item_index",
                Status::Live,
            ),
            // The three rules of grammar 8.6 the two forms above do not reach:
            // the `default:` catch-all over the unrouted variants, a sink route
            // waited on like any other beside a `detach: true` one resolved at
            // dispatch, and `on_item_error` dropping a failed item.
            (
                "a_sink_route_is_joined_and_a_detached_one_is_resolved_at_dispatch",
                Status::Live,
            ),
            // …and what "resolved at dispatch" leaves in the record: the join
            // never observed the delivery, so nothing it went on to do is on
            // the map node's entry either. The sibling above decides that the
            // run does not wait; this decides that the trace does not report a
            // wait it did not make (`docs/trace.md` §5.1, PRD 5.3).
            (
                "a_detached_deliverys_effects_stay_off_the_map_nodes_entry",
                Status::Live,
            ),
            // Rule 6's other end: a dispatch of zero instances is a completion,
            // not a stall — its outgoing edge fires as if every instance had
            // finished.
            (
                "an_empty_fan_out_completes_and_its_downstream_edge_still_fires",
                Status::Live,
            ),
            // A map whose target is a `flow.*` that itself fans out: the
            // flattened instance path of grammar 9.4, the innermost
            // `execution.item_index`, and channel values that stay inside their
            // instance (grammar 10.1).
            (
                "a_nested_fan_out_keys_and_isolates_each_instance_by_its_whole_path",
                Status::Live,
            ),
            // …and where the trace puts a dispatched instance: under its own
            // record, once. The sibling above decides that a nested instance is
            // reported; this decides that the map node's entry is not a second
            // place it is reported from (`docs/trace.md` §3, §8, PRD 5.3).
            (
                "a_dispatched_instances_trace_stays_under_its_own_record",
                Status::Live,
            ),
            // …and the same boundary read the other way round, at the other
            // module boundary: a `flow:` node whose child's fan-out failed
            // carries the child's whole trace and none of its dispatch records,
            // because only a `map` node has that key (`docs/trace.md` §3).
            (
                "a_child_instances_fan_out_stays_inside_the_flow_nodes_inner_trace",
                Status::Live,
            ),
            // The same criterion over the *documented* project rather than a
            // fixture: `examples/triage-fanout` is what PRD 5.6 is written
            // about, and it reaches further than any fixture — a subgraph, a
            // `function:` node, three routes including two `tool.*` sinks, a
            // concurrent branch converging at equal depth, and an inline
            // `exec:` node after the join.
            (
                "the_triage_fanout_example_routes_every_finding_and_joins_them_in_source_order",
                Status::Live,
            ),
            // Rule 5's other two policies. `append` is the one every flow above
            // writes; a `merge` channel and a `last_wins` channel with no
            // `default:` are the shapes where "index-tagged" is decided by the
            // reducer's *interface* rather than by its order alone — an unset
            // channel never calls its reducer for the first write.
            (
                "a_merge_and_an_undefaulted_last_wins_channel_take_the_highest_indexed_write",
                Status::Live,
            ),
            // The traversal ordinal of grammar 9.4, in a run rather than in a
            // derivation: one map node inside a bounded cycle, and a detached
            // sink that reports which key each of its two deliveries carried.
            (
                "each_traversal_of_a_map_delivers_its_detached_sink_a_key_of_its_own",
                Status::Live,
            ),
            // Rule 10's `fail` half, which is the path a fan-out's record is
            // easiest to lose on: the map node produced no answer, and the items
            // that already ran — one write, one delivery to a sink — are
            // accounted for only if the record survives the failure.
            (
                "a_fan_out_absorbed_by_its_own_on_error_still_records_every_dispatch",
                Status::Live,
            ),
            // …and the same two obligations under the other way a map node
            // fails, which is the harder one: its own `timeout:` (rule 9,
            // grammar 9.2). The deadline is *raced*, so the map's promise is
            // abandoned and carries nothing out — and a detached sink queued
            // behind the instance the budget cut short still has to be
            // delivered, on a clock that is not the node's.
            (
                "a_fan_out_cut_short_by_its_own_timeout_still_delivers_and_records_its_sink",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "subgraphs",
        tests: &[
            (
                "a_subgraph_runs_with_explicit_bindings_and_isolated_history",
                Status::Live,
            ),
            // The two keys the default instantiation does not exercise:
            // `context: inherit`, which shares the caller's history in both
            // directions, and `policy:`, which is grammar 9.3's level 1 for
            // every node inside the instance.
            (
                "a_subflow_inherits_the_callers_history_and_takes_its_instantiation_policy",
                Status::Live,
            ),
            // An instantiation under a `retry:` is more than one instance, and
            // each is a run with effects and a trace of its own — so what the
            // boundary reports is a claim about the subgraph rather than about
            // the policy above it, and it belongs to this criterion.
            (
                "a_retried_subflow_reports_the_instance_of_every_attempt",
                Status::Live,
            ),
            // The subgraph's *other* call site. PRD 5.1's flow-as-tool
            // equivalence makes an agent's `tools:` entry a second way to reach
            // the same module, and grammar 7.7 clause 4 analyses it as one — so
            // what the provider is offered, and what a call to it instantiates,
            // belong to this criterion rather than to the agent's. Four tests,
            // because a call has four outcomes worth pinning apart: the
            // instance ran, its arguments never fit, it failed inside, and the
            // name the model used was never on the wire — the last being the
            // one call whose record carries no `target` (`docs/trace.md` §7.3).
            // Two of the four are **refusals** and go back to the model
            // (Decision D119); the one between them is the tool's execution
            // failing, which still ends the node — that split is the point.
            (
                "a_flow_attached_as_a_tool_runs_one_instance_per_call_under_its_own_frame",
                Status::Live,
            ),
            (
                "arguments_a_flow_tools_inputs_refuses_come_back_to_the_model_as_a_tool_error",
                Status::Live,
            ),
            (
                "a_flow_tool_call_whose_instance_failed_fails_the_agent_node_carrying_its_trace",
                Status::Live,
            ),
            (
                "a_call_to_a_tool_the_agent_does_not_offer_comes_back_with_the_names_it_has",
                Status::Live,
            ),
            // …and the same correction on the other wire, where the invented
            // name may not be replayed at all: Chat Completions re-validates an
            // assistant turn's `tool_calls` against the request's `tools`
            // (`WIRE-NOTES` (18)), so the turn is rewritten rather than sent.
            (
                "a_call_to_a_tool_the_agent_does_not_offer_is_corrected_on_the_chat_completions_wire",
                Status::Live,
            ),
            // …and five more over the *frame* a call derives (grammar 9.4, PRD
            // resolved q19), which is the half of this call site that no other
            // construct has: the ordinal counts invocations rather than answers,
            // a node `retry:` restarts it, a refusal spends none of it even
            // across a retry, a `map` puts an item frame above it, and a raced
            // deadline leaves a call with no record at all.
            (
                "two_flow_tool_calls_in_one_model_answer_get_distinct_ordinals",
                Status::Live,
            ),
            (
                "an_agent_node_retry_restarts_the_flow_tool_call_ordinals",
                Status::Live,
            ),
            (
                "a_refusal_does_not_move_the_key_a_retried_attempt_re_derives",
                Status::Live,
            ),
            (
                "a_map_dispatched_agents_flow_tool_calls_nest_under_the_items_frame",
                Status::Live,
            ),
            (
                "a_node_deadline_that_abandons_a_tool_call_records_neither_half_of_it",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "retry/timeout policy",
        tests: &[
            (
                "a_node_retries_its_model_call_per_its_declared_policy",
                Status::Live,
            ),
            (
                "a_node_timeout_fires_and_its_error_policy_takes_over",
                Status::Live,
            ),
            // The same budget over a child process, and the third `on_error`
            // strategy: a fallback replaces the node's own edges (D21).
            (
                "a_node_timeout_fires_over_a_child_process_and_its_fallback_takes_over",
                Status::Live,
            ),
            // …and over the one activity the runtime does not control: a
            // grammar 6.1 host function that never looks at `context.signal`.
            // The other two observe the abort because the runtime spawns and
            // fetches for them, so only this one decides that the deadline is
            // raced rather than merely signalled.
            (
                "a_node_timeout_fires_over_a_host_function_that_ignores_its_signal",
                Status::Live,
            ),
            // The criterion's boundary: which errors a policy governs at all.
            // An unset-channel read fails the *execution* (grammar 10.1, D78),
            // and neither `skip` nor a `fallback:` absorbs it.
            (
                "reading_an_unset_channel_fails_the_run_whatever_the_error_policy_says",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "store-op nodes + synthesized store tools with SQLite/local-disk backends",
        tests: &[
            (
                "a_store_op_node_reads_and_writes_the_local_backend",
                Status::Live,
            ),
            // The other two kinds, which the `kv` round trip above does not
            // reach: a `vector` store — whose ops embed through a real provider
            // connection (grammar 11.2) — and a `blob` store on local disk.
            (
                "a_vector_and_a_blob_store_round_trip_through_the_local_backends",
                Status::Live,
            ),
            // The lifetime that outlives an execution, which is the whole of
            // PRD 5.8's cross-session memory story — and the run-start refusal
            // that keeps it honest when no session identity was supplied.
            (
                "a_session_scoped_store_outlives_the_execution_that_wrote_it",
                Status::Live,
            ),
            (
                "an_attached_store_synthesizes_its_tool_surface_in_the_model_request",
                Status::Live,
            ),
            ("agent_access_read_withholds_the_write_tool", Status::Live),
            // The two rows above decide the tool *list*. This one decides the
            // surface: a model that calls one of those tools reaches the same
            // backend the store-op nodes reach, carrying the arguments it sent —
            // which a list cannot show, and which is the half of "synthesized
            // store tools" a composition actually depends on.
            (
                "an_agent_calling_a_synthesized_store_tool_reaches_the_backend_with_its_arguments",
                Status::Live,
            ),
            // …and what happens when it calls one *wrong*. Decision D119 splits
            // this surface the way it splits the other two: arguments grammar
            // 11.4's row does not admit go back to the model, and a backend that
            // could not answer still ends the node.
            (
                "a_store_tool_refuses_arguments_to_the_model_and_still_fails_the_node_on_a_backend_error",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "model routing with trace-recorded failover",
        tests: &[
            (
                "a_route_fails_over_to_its_next_member_and_the_trace_records_it",
                Status::Live,
            ),
            (
                "a_condition_outside_route_on_fails_the_node_instead_of_failing_over",
                Status::Live,
            ),
            // The three conditions the first row does not stage, each failing
            // over through a route that declares it. `route_on:` is a declared
            // list, so a criterion met for one of its members and not the others
            // would be met by an accident of which status the fixture happened
            // to script — and with this row grammar 12.2's whole enumeration is
            // staged **positively**, so a classifier that stopped recognising a
            // condition fails here instead of quietly turning the row above into
            // the only place that condition appears.
            (
                "every_declared_condition_is_recognized_from_the_shape_the_provider_answers_with",
                Status::Live,
            ),
            // `timeout` twice, because it is two conditions wearing one name: a
            // provider that *reports* a failure to answer, which the row above
            // stages as a dropped connection, and a provider that reports
            // nothing at all, which only a budget can end.
            (
                "a_route_member_that_answers_nothing_fails_over_inside_the_nodes_budget",
                Status::Live,
            ),
            // "recorded" is the load-bearing half of this phrase, and a record
            // that survived only a successful run would not be one — nor one
            // that survived only the *winning attempt* of a retried node, which
            // is the other half and the one the answer alone cannot carry.
            (
                "an_exhausted_route_records_every_member_it_spent_in_the_failed_nodes_trace",
                Status::Live,
            ),
            (
                "a_node_that_succeeded_on_a_retry_records_what_its_earlier_attempt_called",
                Status::Live,
            ),
        ],
    },
    // The emission has landed: the built project reads its `${ENV}` references
    // when it is loaded and refuses to start without them, and `compose-core`'s
    // `the_generated_project_checks_its_environment_when_it_is_loaded` decides
    // that on every `cargo test` against a real `bun src/index.ts`. The row is
    // still pending because this file's test asserts it of a *run*, and the
    // reason says the command it waits on rather than the feature it has.
    Criterion {
        bullet: Bullet::Codegen,
        phrase: "env-ref presence checks at process start",
        tests: &[
            (
                "a_missing_env_ref_fails_at_process_start_naming_the_variable",
                Status::Live,
            ),
            // PRD §9.15's other half: `run`/`serve` fail fast *before* invoking
            // the graph, which is a different check with a different message —
            // the compiler names every reference and the site each is written
            // at, without a runtime having been started at all.
            (
                "run_refuses_before_it_launches_when_a_variable_is_missing",
                Status::Live,
            ),
        ],
    },
];

/// PRD §7 M1's second bullet, decomposed into the four items it lists.
const COMMANDS: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose build`",
        tests: &[(
            "build_writes_a_typescript_project_for_the_target",
            Status::Live,
        )],
    },
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose run` (manual trigger)",
        tests: &[
            (
                "run_executes_a_manual_trigger_and_prints_the_flow_outputs",
                Status::Live,
            ),
            // What the other report format answers with: the whole record on
            // stdout, which is the shape a caller that parses one stream reads.
            (
                "run_reports_its_whole_record_under_the_json_format",
                Status::Live,
            ),
            // …and the precondition of *starting* something that has nothing to
            // do with the composition: the pinned dependency set.
            (
                "run_says_what_is_missing_when_the_dependency_set_is_not_installed",
                Status::Live,
            ),
            // The one key a *declared* manual trigger adds to the entry that
            // exists without it (grammar 13.2): the `session_key:` remap of
            // `--session`, which decides the partition a session-scoped store
            // op addresses.
            (
                "a_declared_manual_trigger_remaps_the_session_the_cli_was_given",
                Status::Live,
            ),
            // And the outcome a one-shot CLI run cannot carry to the end
            // *headless*: a `human` pause, whose answer then arrives through
            // `serve`'s resume route (grammar 8.7, PRD 5.11). What `run` owes
            // there is an exit path of its own rather than one of the two it
            // already has.
            (
                "run_reports_the_pause_it_cannot_answer_and_exits_on_its_own_code",
                Status::Live,
            ),
            // …which is the *other* half of a rule whose first half is a run at a
            // terminal. A pause is a question, and a `run` with somebody to ask
            // asks it: the emitted CLI renders what the human is shown, reads one
            // line of JSON, holds it to the node's `output:` exactly as the
            // resume route does, and carries on in the same process (grammar 8.7,
            // PRD §9.21). Five tests, because the surface has five separable
            // claims — the loop itself, the order and refusals of several
            // questions, a budget that runs out mid-question, the answers running
            // out, and the switch that decides which path a run takes.
            (
                "a_run_at_a_terminal_asks_the_pause_it_reaches_and_finishes_the_flow",
                Status::Live,
            ),
            (
                "a_terminal_asks_one_question_per_pause_and_a_refused_answer_asks_again",
                Status::Live,
            ),
            (
                "a_wait_that_expires_while_the_terminal_is_asking_withdraws_the_question",
                Status::Live,
            ),
            (
                "a_terminal_that_runs_out_of_answers_leaves_the_run_interrupted",
                Status::Live,
            ),
            (
                "a_run_told_not_to_ask_reports_the_pause_instead_of_reading_the_answer",
                Status::Live,
            ),
            // …and the same exit taken from under an **agent** node, which is
            // the one place the interrupt path has a trace entry to lose: a
            // pause inside a flow the model called leaves the loop's dispatch
            // records and the model calls that name them on one entry
            // (PRD §9.20, `docs/trace.md` §9).
            (
                "a_pause_a_run_cannot_answer_leaves_the_loops_calls_on_the_agent_nodes_entry",
                Status::Live,
            ),
        ],
    },
    // The criterion's three verbs, plus what the app does with a request and
    // what the command does when there is no app to serve. `resume` is the verb
    // that takes the most tests, because it is the one whose refusals carry
    // information: which pause a request means, and whether the wait it names is
    // still there to answer (grammar 8.7, PRD 5.11).
    Criterion {
        bullet: Bullet::Commands,
        phrase: "`agent-compose serve` (generated Fastify app for http triggers: start/resume/status)",
        tests: &[
            (
                "serve_exposes_start_and_status_for_an_http_trigger",
                Status::Live,
            ),
            (
                "serve_answers_a_sync_trigger_and_upgrades_when_its_timeout_expires",
                Status::Live,
            ),
            (
                "resume_tells_an_unknown_execution_from_one_that_is_not_waiting",
                Status::Live,
            ),
            // What the app does with a request is as much a part of "generated
            // Fastify app" as which routes it mounts: the body rule of
            // Decision D117, and the completion webhook `start` fires.
            (
                "a_request_with_an_empty_body_starts_an_execution_and_a_non_object_one_does_not",
                Status::Live,
            ),
            (
                "a_completion_webhook_fires_with_the_runs_report_and_only_when_a_url_was_given",
                Status::Live,
            ),
            // …and what it answers a caller whose request the trigger's own
            // bindings could not read, which is the first place a CEL
            // diagnostic is read by somebody outside the composition (PRD G3).
            (
                "a_trigger_that_cannot_read_a_request_names_the_key_it_looked_for",
                Status::Live,
            ),
            // And what the *command* answers when there is no app to serve.
            (
                "serve_answers_two_with_a_sentence_when_it_cannot_take_the_port",
                Status::Live,
            ),
            // …and what it answers for the one collision the compiler cannot
            // decide, which is a route rather than an address.
            (
                "serve_names_the_route_collision_the_compiler_could_not_see",
                Status::Live,
            ),
            // …and what it does when there is one and it is asked to stop. The
            // command is not the server — it launches the emitted project and
            // waits on it — so "the app is served by this command" is only true
            // if ending the command ends the app.
            ("stopping_serve_stops_the_app_it_started", Status::Live),
            // The third verb, end to end: the execution really stops at the
            // pause, the status route publishes what the human is shown and
            // what their answer has to fit, and the payload is held to that
            // schema before anything is delivered.
            (
                "serve_resumes_an_interrupted_execution_against_the_human_nodes_schema",
                Status::Live,
            ),
            // …and the two other ways a pause ends or is addressed: a budget
            // that runs out and routes elsewhere, and an execution holding more
            // than one pause at a time.
            (
                "an_expired_wait_takes_its_route_and_refuses_the_answer_that_arrives_after_it",
                Status::Live,
            ),
            // …and the other target `on_timeout:` takes, which is not a node at
            // all: `end` retires the branch the wait was holding, so nothing is
            // scheduled after the budget spends and the execution nonetheless
            // completes (grammar 8.7, 7.6.3).
            (
                "a_wait_that_expires_into_end_retires_its_branch_and_completes_the_execution",
                Status::Live,
            ),
            (
                "two_pauses_in_one_execution_are_addressed_by_their_instance_paths",
                Status::Live,
            ),
            // …and the other axis a wait id is built on, which a fan-out cannot
            // reach: one `human` node inside a bounded cycle, whose two pauses
            // are the same site at two traversals and are told apart by grammar
            // 9.4's ordinal alone.
            (
                "a_second_traversals_pause_is_addressed_apart_from_the_first",
                Status::Live,
            ),
            // …and the other half of "several waits at once": two executions,
            // each holding a pause of its own at the same instance path, which
            // is what makes a wait belong to an execution rather than to the
            // process.
            (
                "two_interrupted_executions_hold_their_pauses_and_answers_apart",
                Status::Live,
            ),
            // …and the promise Decision D102 makes about a wait, held one
            // construct further out than the decision's own text reaches: the
            // node that *dispatched* the pause has a budget, and it does not run
            // while the pause is open.
            (
                "an_enclosing_nodes_budget_does_not_run_while_a_pause_below_it_is_open",
                Status::Live,
            ),
            // …and the third construct a pause can sit under, which grammar 8.7
            // names and which arrived with the flow-as-tool runtime: a flow a
            // **model** called (grammar 5.4, 7.7 clause 4). The wait id is the
            // child instance's path with the node's frame on the end, so the
            // status and resume routes address it with nothing new.
            (
                "a_pause_inside_a_flow_a_model_called_is_published_and_answered_like_any_other",
                Status::Live,
            ),
            // …and D102's hold at that third construct, which is the one where
            // the node holding the timer is also the node dividing the budget
            // across a model route (grammar 9.2, 9.3): an `agent:` node's own
            // `timeout:` outlived by the wait under its tool call.
            (
                "an_agent_nodes_budget_does_not_run_while_a_pause_below_its_tool_call_is_open",
                Status::Live,
            ),
        ],
    },
    // The committed half of this criterion — "goldens are reviewed in PRs like
    // any other code" — lives in `compose-core`'s
    // `tests/generated_project_goldens.rs`, over `tests/goldens/`. The row points
    // at the acceptance test because that is what this file maps, and because the
    // determinism it asserts through the real command is what makes a committed
    // golden mean anything.
    Criterion {
        bullet: Bullet::Commands,
        phrase: "golden-file codegen tests",
        tests: &[(
            "build_is_byte_identical_for_byte_identical_input",
            Status::Live,
        )],
    },
];

/// PRD §7 M1's third bullet — what this PR delivers, so its rows are the live
/// ones. It is one criterion rather than a split list because its sentence is
/// one claim ("compiled graphs execute end-to-end in CI …") with a trailing
/// condition, not an enumeration of deliverables.
const HARNESS: &[Criterion] = &[Criterion {
    bullet: Bullet::Harness,
    phrase: "Mock provider server + e2e harness: compiled graphs execute end-to-end in CI with scripted model responses, no API keys.",
    tests: &[
        ("the_acceptance_fixtures_validate_clean", Status::Live),
        (
            "a_generated_process_is_handed_a_sealed_environment",
            Status::Live,
        ),
        (
            "the_harness_serves_both_provider_surfaces_without_api_keys",
            Status::Live,
        ),
        (
            "a_scripted_structured_output_is_what_an_agent_node_will_receive",
            Status::Live,
        ),
        (
            "a_scripted_failover_condition_is_what_a_model_route_will_see",
            Status::Live,
        ),
        (
            "every_model_call_is_recorded_in_order_with_its_parsed_body",
            Status::Live,
        ),
        (
            "a_request_no_codegen_should_send_is_refused_and_recorded",
            Status::Live,
        ),
        (
            "an_unscripted_model_call_fails_the_run_loudly",
            Status::Live,
        ),
        (
            "a_scripted_delay_makes_completion_order_differ_from_item_order",
            Status::Live,
        ),
        // The fixture list's own guard, which grew a second arm when the first
        // fixture that validates with a *warning* arrived (Decision D122).
        ("every_fixture_with_a_declared_warning_exists", Status::Live),
        // Server tools (grammar 12.1, Decision D122, resolved q30), which are
        // this bullet's criterion on both counts: they are what a scripted model
        // response now has to be able to *carry*, and the Responses wire they
        // move an `openai` provider onto is a surface the harness had to learn.
        // Each is one claim a transcript decides.
        (
            "a_providers_server_tools_reach_the_messages_wire_verbatim",
            Status::Live,
        ),
        (
            "a_server_tool_block_is_neither_dispatched_nor_refused_by_the_loop",
            Status::Live,
        ),
        (
            "an_openai_provider_with_server_tools_runs_its_loop_on_the_responses_wire",
            Status::Live,
        ),
        // …and the turn shape only that wire can produce: a preamble `message`
        // before the shaped one, which is what a server tool running mid-turn
        // leaves behind.
        (
            "a_pinned_responses_turn_is_read_at_the_message_the_format_shaped",
            Status::Live,
        ),
        (
            "a_pinned_responses_turn_carrying_no_object_is_reported_as_no_structured_output",
            Status::Live,
        ),
        // …and the third route the key reaches, which is on neither of those
        // two wires: a gateway keeps Chat Completions and its suite rides that
        // request's own `tools` array.
        (
            "a_gateways_server_tools_ride_its_chat_completions_tools_array",
            Status::Live,
        ),
        (
            "a_server_tool_outside_the_table_is_warned_about_and_still_reaches_the_wire",
            Status::Live,
        ),
        // …and the same tier one level in: a key the row of a *known* tool does
        // not name, which a compiler that refused it would block every author of
        // that tool over until a new binary shipped.
        (
            "a_server_tool_field_outside_the_table_is_warned_about_and_still_reaches_the_wire",
            Status::Live,
        ),
        ("a_failover_that_crosses_two_wires_composes", Status::Live),
        // …and the half of that seam a first-call failover cannot reach: a
        // *replayed* assistant turn, which each wire will only take back in its
        // own vocabulary.
        (
            "a_failover_off_the_responses_wire_rewrites_the_turn_for_the_messages_one",
            Status::Live,
        ),
        (
            "a_failover_off_the_messages_wire_rewrites_the_turn_for_the_responses_one",
            Status::Live,
        ),
        // …and the turn that rewriting has *nothing* to write: a Responses
        // answer that is all server-tool items, which the rebuild must drop
        // rather than send as the empty message the Messages API refuses.
        (
            "a_responses_turn_with_nothing_the_messages_wire_can_spell_is_not_replayed_empty",
            Status::Live,
        ),
        (
            "server_tools_on_a_kind_whose_wire_has_none_is_refused_by_name",
            Status::Live,
        ),
    ],
}];

/// The M1 requirements CLAUDE.md's *Validation strategy* adds that PRD §7 M1's
/// own sentences do not name individually. They are not extras — "CI green has
/// to mean actually done" is the reason this file exists — but they come from
/// the working agreement rather than from the PRD's enumeration, so they are
/// listed apart to keep the transcription honest. This mirrors `GRAMMAR` in
/// `static_check_inventory.rs`.
const STRATEGY: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Strategy,
        phrase: "every golden fixture must type-check (`tsc`) and construct its graph under the pinned LangGraph version",
        // `build` and the pinned toolchain both landed, and `compose-core`'s
        // `tests/generated_code_gates.rs` already runs this criterion's two
        // checks — `tsc --noEmit` and construction under the pinned LangGraph —
        // over the committed golden corpus on every `cargo test`. The row stays
        // pending because its *subject* has not landed: `src/graph.ts` builds no
        // topology yet, so "constructs its graph" is not yet a claim to make.
        tests: &[(
            "every_generated_project_type_checks_and_constructs_its_graph",
            Status::Live,
        )],
    },
    Criterion {
        bullet: Bullet::Strategy,
        phrase: "shared fixtures (expression + input + expected result) executed against both the Rust validator's interpreter and the JS evaluator embedded in generated code",
        tests: &[(
            "the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus",
            Status::Live,
        )],
    },
];

/// PRD §7 M3's first bullet, whose four clauses are the four claims durability
/// makes: the record, what a resume does with it, the two recovery surfaces, and
/// the failure that is not a re-execution.
///
/// The bullet is one sentence rather than an enumeration, so — like `HARNESS` —
/// its phrases are held to it by containment rather than by splitting on `, `.
/// The resolved questions behind it (q26–q29) are quoted in each test.
const DURABILITY: &[Criterion] = &[
    Criterion {
        bullet: Bullet::Durability,
        phrase: "a deploy-target-bound journal (SQLite locally, Postgres when distributed) records every effect",
        tests: &[
            // The two effect kinds a repeat is *visible* in, which is what makes
            // "records every effect" a claim a test can decide rather than an
            // inventory of call sites.
            (
                "a_replayed_prefix_re_issues_neither_its_store_write_nor_its_subprocess",
                Status::Live,
            ),
            // …and the half a write cannot decide: a **read** is recorded too,
            // and what it answers has to be what the generation that recorded it
            // went on with, or the two generations compose different requests
            // out of one composition (`docs/durability.md` §11.1).
            (
                "a_replayed_store_read_is_the_row_the_recording_generation_went_on_with",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Durability,
        phrase: "a resumed execution replays that record read-only up to the frontier",
        tests: &[
            // The core assertion of resolved q29, decided by the provider's own
            // request log: the recorded calls do not reach it a second time.
            (
                "a_resumed_run_consumes_its_recorded_model_answers_instead_of_asking_again",
                Status::Live,
            ),
            // …and the failure that is *not* a re-execution: a journal that no
            // longer describes this composition stops the resume naming the step
            // it disagrees at.
            (
                "a_resume_whose_journal_no_longer_describes_the_run_names_the_divergent_step",
                Status::Live,
            ),
            // …and the one composition shape that could have made replay a
            // different question and does not: a provider that runs **server
            // tools** (Decision D122). The use happens inside the recorded model
            // call, so it gets no record and no key of its own, and the resumed
            // generation replays the whole turn — search and all.
            (
                "a_resumed_run_replays_a_server_tools_answer_without_asking_again",
                Status::Live,
            ),
            // …and the same claim on the second block-carrying wire, whose
            // service-minted item ids travel inside the recorded answer and so
            // inside the next call's request identity (Decision D122).
            (
                "a_resumed_run_replays_a_responses_wire_answer_without_asking_again",
                Status::Live,
            ),
            // …and the promise `docs/durability.md` §7 makes about the other
            // direction, which the two above cannot decide because a matching
            // suite replays whether or not the identity keys on it: a resume
            // whose provider *lost* a server tool diverges at the first model
            // call rather than replaying an answer produced under another tool
            // surface (Decision D122, resolved q29).
            (
                "a_resume_whose_provider_lost_its_server_tools_diverges_at_the_first_model_call",
                Status::Live,
            ),
            // …and the arm that buys, which is why `JOURNAL_VERSION` did not
            // move for any of it: a composition declaring no suite omits the key
            // entirely and derives the identity it always derived, so a journal
            // written before the key existed still replays.
            (
                "a_composition_with_no_server_tools_keeps_the_identity_it_always_derived",
                Status::Live,
            ),
            // …and the same argument on the second key `JOURNAL_VERSION` did not
            // move for: a Messages-wire turn is replayed untagged, so a tool loop
            // on that wire puts no `wire` key in the identity either
            // (`docs/durability.md` §3.1).
            (
                "a_messages_wire_tool_loop_keeps_the_untagged_turns_it_always_derived",
                Status::Live,
            ),
            // …the other divergence resolved q29 names, which is not a request
            // that moved but an answer this composition no longer accepts.
            (
                "a_recorded_answer_that_fails_the_current_contract_is_a_divergence_not_a_retry",
                Status::Live,
            ),
            // …and its other side, which is what keeps that from breaking a
            // composition nobody touched: a mismatch the recording generation's
            // own ladder already absorbed is replayed, not re-decided.
            (
                "a_recorded_answer_the_original_retried_past_is_retried_past_again",
                Status::Live,
            ),
            // …and what tells those two apart, at the site that has more than
            // one effect of a kind. Nothing about the records *around* the
            // answer can: two different tools at one site defeat counting, and
            // one tool called twice with the same arguments defeats counting
            // plus request identity, so the record says which it was.
            (
                "a_contract_that_moved_on_one_of_two_tools_at_a_site_is_not_read_as_a_retry",
                Status::Live,
            ),
            (
                "a_contract_that_moved_on_a_repeated_call_at_a_site_is_not_read_as_a_retry",
                Status::Live,
            ),
            // …and the one world a live effect past the frontier cannot assume
            // the record left behind: a store whose rows died with the process.
            (
                "a_store_that_died_with_the_process_refuses_the_resume_it_cannot_answer",
                Status::Live,
            ),
            // …and its opposite, which is the premise the refusal is carved out
            // of: a store that *does* outlive the process has to be there, so a
            // run that only parked leaves its own `blob` partition alone.
            (
                "a_parked_runs_own_blob_store_is_there_for_the_generation_that_resumes",
                Status::Live,
            ),
            // …and the second divergence at the one record kind that reaches no
            // result parse: an answer a person gave, under an `output:` the
            // composition has since narrowed.
            (
                "a_recorded_human_answer_the_composition_no_longer_admits_is_a_divergence",
                Status::Live,
            ),
            // …and the three nesting depths a divergence may not be absorbed at:
            // a dispatched subflow under `on_item_error: skip`, the item retry
            // the other form of that key gives, and a delivery nothing waits for.
            (
                "a_divergence_inside_a_dispatched_subflow_is_not_absorbed_by_on_item_error",
                Status::Live,
            ),
            (
                "a_divergence_inside_a_dispatched_item_is_not_retried_by_its_item_policy",
                Status::Live,
            ),
            (
                "a_divergence_in_a_detached_delivery_fails_the_resume_it_cannot_be_thrown_out_of",
                Status::Live,
            ),
            // …and the other half of that depth, which is the one a delivery's
            // *own* record has to carry: nothing joins it, so none of the
            // `ModelCall` entries a joined call files are filed at all, and a
            // replay that inferred the answering member from them would report a
            // composition nobody touched as divergent — permanently, because a
            // divergence never closes the row.
            (
                "a_detached_delivery_that_called_a_model_is_replayed_rather_than_reported_as_divergent",
                Status::Live,
            ),
            // …and the two refusals that come *before* a replay: an id the
            // journal does not hold, and a project with no journal at all.
            (
                "a_resume_that_cannot_find_its_execution_says_what_the_journal_holds",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::Durability,
        phrase: "executions survive a process restart",
        tests: &[
            // The `run` half (resolved q28: "`run` journals but does not
            // auto-resume"), including the wait id a second generation re-parks
            // under.
            (
                "a_pause_killed_with_its_process_is_asked_again_under_the_same_wait_id",
                Status::Live,
            ),
            // …and its other half, which is the promise a person can see: a
            // pause somebody **answered** is replayed rather than put to them
            // twice, dated by the generation that held it.
            (
                "an_answered_pause_is_replayed_rather_than_put_to_the_person_twice",
                Status::Live,
            ),
            // …and the `serve` half, which recovers on its own.
            (
                "a_restarted_serve_recovers_its_open_executions_and_their_waits",
                Status::Live,
            ),
            // …including the window recovering without waiting opens: an answer
            // that arrives before the replay is back at its pause is told to
            // send it again, rather than that there is nothing waiting for it.
            (
                "a_recovered_execution_still_catching_up_tells_a_resume_to_send_it_again",
                Status::Live,
            ),
            // …and that the window is per **pause**: an execution holding two
            // whose branches are not back at the same moment must not hand the
            // second one's client the refusal the first one's arrival cleared.
            (
                "a_second_pause_still_being_replayed_to_is_told_to_send_its_answer_again",
                Status::Live,
            ),
            // …including what an `async` caller was promised: the completion
            // webhook is fired by the process that *finishes* the run, which is
            // not the one that answered its `202`.
            (
                "a_recovered_execution_delivers_the_completion_webhook_its_caller_waits_for",
                Status::Live,
            ),
            // …and the half of that promise a divergence would break twice
            // over: a recovery that leaves the row open pushes nothing, so one
            // execution is one completion whatever it took to reach it.
            (
                "a_diverged_recovery_delivers_no_webhook_and_the_repair_delivers_one",
                Status::Live,
            ),
            // …and the same promise where the failure is not a divergence at
            // all: a recovery refused before the execution is opened leaves the
            // row open while carrying nothing the class of the error could say
            // so, so the push is decided by reading the row.
            (
                "a_recovery_that_cannot_take_the_recorded_inputs_delivers_no_webhook",
                Status::Live,
            ),
            // …and what a run that stops on its own must not leave behind: a
            // detached delivery made with no record is one the resume makes
            // again.
            (
                "a_detached_delivery_in_flight_when_a_run_parks_is_not_delivered_twice",
                Status::Live,
            ),
            // …and what a **resumed** run must not decide while one is still in
            // flight: a delivery is the one place a divergence has nothing to be
            // thrown to, so the predicate that keeps the row open is still being
            // decided until the deliveries are done.
            (
                "a_resumed_generation_does_not_end_with_a_detached_delivery_still_in_flight",
                Status::Live,
            ),
            // …and what "survives" has to mean when a build disagrees with the
            // record: an execution a divergence stopped is still open, because a
            // resume against a `failed` row is refused by name and `serve`
            // replays every open execution at every start.
            (
                "a_diverged_resume_leaves_the_execution_open_for_the_composition_that_fits_it",
                Status::Live,
            ),
            // …and what a crash must not be able to do to the one artifact
            // recovery reads: seal it. A writer killed inside a write leaves the
            // driver's lock directory behind, and nothing else removes it.
            (
                "a_lock_a_killed_writer_left_behind_does_not_seal_the_journal",
                Status::Live,
            ),
        ],
    },
];

/// PRD §7 M3's second bullet, its runtime half: the built-ins, what bounds
/// them, and what a bound being crossed does.
///
/// One sentence rather than an enumeration, so — like `HARNESS` and
/// `DURABILITY` — its phrases are held to it by containment. Three claims: that
/// the tools exist and run, that the workspace bounds them, and that the timeout
/// does. The failure and refusal rules are not separate phrases of the bullet —
/// resolved q31 states them as "the standing rules" — so they hang off the two
/// bounds they are reached through.
///
/// PRD resolved q54 replaced the four-tool set this bullet was written against
/// with `builtin.bash` and `builtin.files`, and both halves are named below. The
/// file tool's rows are its round trip — `create`, `str_replace`, `insert` and
/// `view` under the workspace, in the provider-defined editor's own operations —
/// and the two crossings that workspace has to refuse: a path that *resolves*
/// outside it, and a second **name** for a file outside it, which no resolution
/// can catch because a hard link has no target to follow.
///
/// What no row here reaches is the inside of one call, which is a limit of this
/// corpus rather than a gap in it: an acceptance case drives a model loop, and
/// what a loop shows is the wire, the trace and the bounce. The rest — every
/// other way out of a workspace, the read bound, the session held across two
/// activities, the scrubbed child — is asserted against a generated project's
/// own runtime by
/// `the_built_in_tools_are_bounded_by_their_workspace_session_and_deadline` in
/// `compose-core`'s `generated_code_gates.rs`.
const BUILTINS: &[Criterion] = &[
    Criterion {
        bullet: Bullet::BuiltinTools,
        phrase: "runtime bash/file tools",
        tests: &[
            // The shell, in one turn, asserted on things only real calls could
            // produce — including the one whose effect outlives the process, and
            // the program the trace carries of each (resolved q54 ruling c).
            (
                "the_builtin_shell_runs_inside_its_workspace_and_answers_the_model",
                Status::Live,
            ),
            // …and what makes it a *session* rather than a fork per command: a
            // `cd` in one call is still in effect in the next, which is the
            // difference between a tool a model can drive and one it has to
            // re-explain itself to (resolved q54).
            (
                "a_shells_state_carries_across_the_calls_of_one_node_activity",
                Status::Live,
            ),
            // …and the cost that session has: the shell reads its script from
            // the same standard input the commands are typed into, so a command
            // that reads standard input would read this runtime's own protocol
            // — silently, as a command with no exit status or one that spends
            // its whole bound answering with marker text.
            (
                "a_command_that_reads_standard_input_does_not_eat_the_marker_protocol",
                Status::Live,
            ),
            // …and the file tool, round-tripped: create, edit, view, with the
            // provider-defined text editor's own operations and parameter names.
            (
                "the_file_tool_creates_edits_and_views_inside_its_workspace",
                Status::Live,
            ),
            // …and the half of the loop that is not the tool running: a call the
            // contract refuses is the model's to make again, which is what keeps
            // a built-in's contract the same contract every other tool surface
            // has (Decision D119).
            (
                "arguments_a_builtin_refuses_bounce_back_to_the_model",
                Status::Live,
            ),
            // …including the argument the *vendor's* tool carries and this one
            // does not: the Messages wire declares the provider-defined text
            // editor, whose `view` takes a `view_range`, so the refusal has to
            // name the key it refused for the loop to be a cost rather than a
            // dead end.
            (
                "an_argument_outside_a_builtins_schema_is_refused_naming_the_key",
                Status::Live,
            ),
            // …and the same rule where a *file* path is what the model got
            // wrong, which is the one refusal that is also a bound.
            (
                "a_file_path_that_leaves_the_workspace_bounces_back_to_the_model",
                Status::Live,
            ),
            // …and the crossing a *resolution* cannot catch either, which is why
            // it is refused at the write: a hard link has no target, so a second
            // name for a file outside the workspace resolves to the path inside
            // it and an edit through that name lands outside.
            (
                "a_write_to_a_file_with_a_second_name_is_refused_to_the_model",
                Status::Live,
            ),
            // …and what a built-in buys over the `exec:` tool an author could
            // have hand-rolled: the journal holds its answer, so a resumed
            // execution consumes it rather than running the command again.
            (
                "a_resumed_run_consumes_a_recorded_builtin_instead_of_running_it_again",
                Status::Live,
            ),
            // …and the one thing q54 keeps from q31 about *where* the shell
            // comes from: `PATH`, at the call, with a host that has none failing
            // as an execution failure that names the requirement.
            (
                "a_host_with_no_bash_on_path_fails_the_call_naming_the_requirement",
                Status::Live,
            ),
            // …and what the *wire* carries, which resolved q54 ruling d makes a
            // requirement rather than an implementation detail: on the Messages
            // wire the two go out as the dated provider-defined tool types, so
            // the behaviour the model was trained into engages.
            (
                "the_builtins_go_out_as_provider_defined_tools_on_the_messages_wire",
                Status::Live,
            ),
            // …and the wire the other half of that ruling is about: on Chat
            // Completions the same two tools are function tools carrying this
            // compiler's own schemas, because that surface has no
            // provider-defined types.
            (
                "the_builtins_go_out_as_function_tools_on_the_openai_wire",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::BuiltinTools,
        phrase: "opted into per agent node, bounded by root",
        tests: &[
            // The two ways a bound can fail to be a bound at all: a workspace
            // that names no directory, and one the environment answered with
            // nothing — which would leave the tool bounded to wherever the
            // runtime was started, the ambient capability D135 refuses.
            (
                "a_workspace_that_names_no_directory_fails_the_call",
                Status::Live,
            ),
            (
                "a_workspace_that_resolves_to_nothing_fails_the_call",
                Status::Live,
            ),
            // …and the bound a composition did *not* write, which resolved q54
            // gives a lifetime rather than leaving to the process's own working
            // directory: one directory per execution, shared by every built-in
            // of that execution that took the default, gone when the run
            // settles.
            (
                "the_default_workspace_is_the_executions_own_and_goes_when_the_run_settles",
                Status::Live,
            ),
            // …and the bound q54 ruling b adds beside the workspace: the child
            // environment, scrubbed to what the binding declared, with an
            // explicit opt-in for the machines where inheriting is the point.
            (
                "a_shells_environment_holds_what_its_binding_declared_and_nothing_else",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::BuiltinTools,
        phrase: "bounded by root + timeout",
        tests: &[
            // The deadline, reached — and reached at the *binding's* value,
            // since the fixture binds the same built-in at two. What it answers
            // is the **model**: resolved q54 makes a spent deadline a tool
            // result, so the loop goes on and the run completes.
            (
                "a_command_that_outruns_its_timeout_comes_back_as_a_timeout_result",
                Status::Live,
            ),
            // …and the other thing a command can do that is not a failure of the
            // tool: exit nonzero, which for a program the *model* wrote is a
            // fact it asked for rather than a contract of the composition's.
            (
                "a_command_that_exits_nonzero_comes_back_to_the_model_with_its_status",
                Status::Live,
            ),
            // …and what the deadline has to end besides the shell: the work the
            // command *forked*, which is where a shell's work almost always is
            // — a kill aimed at the shell alone leaves it writing inside `root:`
            // after the node has already failed (D124).
            (
                "a_killed_command_takes_the_work_it_forked_with_it",
                Status::Live,
            ),
            // …and the same bound reached the other way, where the *run* is what
            // ends: a command detached far enough for the deadline to reach it is
            // detached out of the terminal's reach, so stopping the run has to
            // take it along — and has to still stop the run.
            (
                "a_run_asked_to_stop_takes_its_command_with_it",
                Status::Live,
            ),
            // …and what is left when the group kill cannot reach it: the pipes an
            // escapee holds, which a runtime that kept them would go on reading —
            // and be held open by — long after the call failed.
            (
                "a_killed_commands_grandchild_does_not_hold_the_runtime_open",
                Status::Live,
            ),
        ],
    },
];

/// PRD §7 M3's HTTP-native events bullet, whose five clauses are the five
/// claims that pass makes: who may call, which routes the answer covers, what a
/// delivery says it is and where it may go, when one fires, and what makes it
/// survive a restart.
///
/// The bullet is one sentence rather than an enumeration, so — like `HARNESS`
/// and `DURABILITY` — its phrases are held to it by containment rather than by
/// splitting on `, `. The resolved questions behind it (q32–q35) are quoted in
/// each test.
const EVENTS: &[Criterion] = &[
    Criterion {
        bullet: Bullet::HttpEvents,
        phrase: "declarative auth on `http` triggers — inbound `bearer`/`hmac`",
        tests: &[
            // Both schemes, every way each can be wrong, and the half a status
            // code cannot say: the counting shim is what makes "no execution
            // started" an assertion rather than an inference.
            (
                "an_authenticated_start_admits_the_credential_it_declares_and_refuses_every_other",
                Status::Live,
            ),
            // …and the credential that is *present* and verifies nothing, which
            // §4.3's presence check counts as set: an empty token is equal to
            // the empty token an anonymous caller sends, so the app refuses to
            // start rather than serving a route that looks guarded.
            (
                "a_credential_set_to_nothing_refuses_the_app_at_launch",
                Status::Live,
            ),
            // …and the request carrying *two*, which is the case a parsed
            // header map cannot report: the runtimes a generated project runs
            // under disagree about which of two values under one name they
            // keep, so a route deciding on one of them admits a request whose
            // credential depends on the launch.
            (
                "a_credential_a_request_carried_twice_verifies_nothing",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::HttpEvents,
        phrase: "an execution's resume/status routes enforcing the auth of the trigger that started it",
        tests: &[
            (
                "an_executions_status_and_resume_enforce_the_auth_of_the_trigger_that_started_it",
                Status::Live,
            ),
            // …and the scheme for which "the same auth" is a different
            // credential on every request: an `hmac` signature is over the body
            // that arrived, so the status route signs the empty one a `GET`
            // carries and the resume route signs the answer exactly as sent.
            (
                "a_signed_executions_status_and_resume_verify_over_each_requests_own_body",
                Status::Live,
            ),
            // …and the half one process cannot decide: the trigger is on the
            // journal's lifecycle row, so a `serve` restarted while somebody was
            // thinking goes on refusing the same callers.
            (
                "a_delivery_a_restart_interrupted_completes_under_the_same_id",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::HttpEvents,
        phrase: "signed and allowlisted callbacks",
        tests: &[
            (
                "a_delivery_carries_the_identity_its_trigger_declared_over_the_bytes_it_sent",
                Status::Live,
            ),
            // …and what the allowlist does with a URL it admits nowhere, which
            // is a recorded refusal rather than anybody's failure.
            (
                "a_callback_url_the_allowlist_admits_nowhere_is_refused_and_the_run_settles",
                Status::Live,
            ),
            // …and the URL whose text the list admits and whose *host* it never
            // named: everything before an `@` is userinfo, so an entry with a
            // wildcard where the port goes matches a string that resolves
            // somewhere else entirely.
            (
                "a_callback_url_that_hides_its_host_behind_userinfo_is_refused",
                Status::Live,
            ),
            // The list is matched **whenever** the URL is read, which includes
            // the reading a restart does of a row an earlier build wrote: a
            // deployment that narrows its allowlist does not deliver what the
            // narrower list admits nowhere.
            (
                "a_pending_delivery_the_allowlist_no_longer_admits_is_refused_rather_than_sent",
                Status::Live,
            ),
            // …and the cost of matching it, which the URL's author chooses: the
            // callback URL comes out of the request payload, so an entry matched
            // by anything that backtracks hands a caller the one thread that
            // answers every route.
            (
                "a_long_callback_url_is_refused_without_wedging_the_app",
                Status::Live,
            ),
            // …and the hop after the one the list matched: an admitted receiver
            // answering `3xx` would otherwise carry the report, its signature
            // and its token wherever its `Location:` named.
            (
                "a_receiver_that_redirects_a_delivery_sends_it_nowhere_else",
                Status::Live,
            ),
            // The check behind the check: every signing assertion above compares
            // against a digest this harness computed, which is worth something
            // only if this side is right.
            (
                "the_harnesss_own_hmac_answers_the_published_vector",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::HttpEvents,
        phrase: "lifecycle webhooks (parkings and settle)",
        tests: &[
            (
                "a_parking_and_a_settle_reach_the_callback_in_ascending_ordinals",
                Status::Live,
            ),
            // The other half of "every quiescence that opened **new** pauses":
            // a recovered execution re-parks under the ids its predecessor
            // published and announces nothing, and a replay that diverges
            // announces nothing either.
            (
                "a_recovered_execution_delivers_the_completion_webhook_its_caller_waits_for",
                Status::Live,
            ),
            (
                "a_diverged_recovery_delivers_no_webhook_and_the_repair_delivers_one",
                Status::Live,
            ),
            // …and the claim no single-pause flow can make: **one** webhook per
            // quiescence, listing every pause it opened, however far apart in
            // time the branches reached them (resolved q34).
            (
                "one_quiescence_that_opened_many_pauses_is_one_parked_delivery",
                Status::Live,
            ),
            // …and the work a quiescence is **not** about: a detached dispatch
            // runs its sink's nodes under this execution's id, and grammar 8.6
            // rule 7 says nothing it does may delay the enclosing instance —
            // the webhook included.
            (
                "a_parking_is_delivered_while_a_detached_dispatch_is_still_running",
                Status::Live,
            ),
        ],
    },
    Criterion {
        bullet: Bullet::HttpEvents,
        phrase: "journal-backed at-least-once callback delivery",
        tests: &[
            (
                "a_delivery_two_refusals_could_not_stop_lands_on_the_third_attempt",
                Status::Live,
            ),
            // …and the bound on it: exhaustion is recorded and is never the
            // execution's failure.
            (
                "a_delivery_no_attempt_lands_is_recorded_exhausted_and_leaves_the_run_alone",
                Status::Live,
            ),
            // The same bound in wall-clock time, which the row above cannot
            // reach: its receiver refuses promptly, and a schedule of five
            // prompt refusals is bounded however long one attempt may take.
            (
                "a_receiver_that_never_answers_does_not_hold_a_delivery_open",
                Status::Live,
            ),
            // The schedule itself is a setting somebody has to be able to trust:
            // one that could not be read is refused at launch rather than
            // ignored (Decision D50).
            (
                "a_callback_retry_schedule_that_is_not_one_refuses_the_app_at_launch",
                Status::Live,
            ),
            // …and the ledger the deliveries live in is a *compatible* change to
            // the journal, which is only true if a journal written before it
            // still opens (`docs/durability.md` §11.2).
            (
                "a_journal_written_before_the_delivery_ledger_opens_and_serves_under_this_build",
                Status::Live,
            ),
            // The event a build cannot deliver is journaled rather than dropped,
            // which is what makes "the row stays `pending` for a build that
            // declares the trigger" a promise about something that exists.
            (
                "a_delivery_journaled_without_its_trigger_is_finished_by_a_build_that_declares_it",
                Status::Live,
            ),
            // `serve` is not the only process that closes a lifecycle row, and a
            // settle journaled after one closes is a settle no start can find:
            // the hand resume records the intent before the row goes, and the
            // next start delivers it.
            (
                "an_execution_finished_by_a_hand_resume_still_journals_the_settle_it_owes",
                Status::Live,
            ),
            // …and the other end a delivery can reach without an attempt: a row
            // whose schedule has no offset it has not already tried is exhausted
            // rather than left owed for the life of the journal.
            (
                "a_pending_delivery_a_shorter_schedule_leaves_no_attempt_for_is_exhausted",
                Status::Live,
            ),
            // …and the bound on what picking one up may cost: a webhook is a
            // courtesy the status route backstops, so a journal read that fails
            // while the owed ones are enumerated leaves one delivery owed rather
            // than a deployment that never binds its port.
            (
                "a_delivery_whose_execution_cannot_be_read_leaves_the_app_serving",
                Status::Live,
            ),
            // …and the row a start has nothing to read the trigger off: a
            // delivery names its own, so the settle journaled for a run that
            // failed before it was journaled at all is still one a later start
            // can finish rather than one it skips for ever.
            (
                "a_settle_journaled_for_an_execution_the_journal_never_held_is_still_delivered",
                Status::Live,
            ),
        ],
    },
];

/// The http-native events rows are phrases of PRD §7 M3's fourth bullet.
///
/// Containment and normalization, for the reason the two siblings above give:
/// the bullet is one sentence about five things, and the PRD wraps its prose, so
/// a clause a reader hears as one spans two lines in the file.
#[test]
fn the_http_events_rows_transcribe_the_prd_m3_bullet() {
    let bullet = distribution_bullet("**HTTP-native events**");
    let flattened = bullet.split_whitespace().collect::<Vec<_>>().join(" ");
    for criterion in EVENTS {
        assert!(
            flattened.contains(criterion.phrase),
            "`{}` is not a phrase of PRD §7 M3's http-native events bullet: {flattened}",
            criterion.phrase
        );
    }
}

fn rows() -> impl Iterator<Item = &'static Criterion> {
    CODEGEN
        .iter()
        .chain(COMMANDS)
        .chain(HARNESS)
        .chain(STRATEGY)
        .chain(DURABILITY)
        .chain(BUILTINS)
        .chain(EVENTS)
}

fn repository() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("the manifest directory has a grandparent")
        .to_path_buf()
}

/// The text of PRD §7 M1 — the three bullets between its heading and M2's.
fn milestone() -> Vec<String> {
    let prd = fs::read_to_string(repository().join("prd.md")).expect("the PRD is readable");
    prd.lines()
        .skip_while(|line| !line.starts_with("**M1 —"))
        .take_while(|line| !line.starts_with("**M2 —"))
        .map(str::to_string)
        .collect()
}

/// One bullet of PRD §7 M1, by the phrase that opens it.
fn bullet(opening: &str) -> String {
    milestone()
        .into_iter()
        .find(|line| line.starts_with(&format!("- {opening}")))
        .unwrap_or_else(|| panic!("PRD §7 M1 has a bullet opening `{opening}`"))
}

/// The text of PRD §7 M3 — everything between its heading and the next section.
fn distribution() -> Vec<String> {
    let prd = fs::read_to_string(repository().join("prd.md")).expect("the PRD is readable");
    prd.lines()
        .skip_while(|line| !line.starts_with("**M3 —"))
        .take_while(|line| !line.starts_with("## "))
        .map(str::to_string)
        .collect()
}

/// One bullet of PRD §7 M3, **whole**.
///
/// M1's bullets each fit on a line; M3's do not, and a reader of this file
/// should not have to know which. So a bullet is the line that opens it plus
/// every wrapped continuation under it — the lines up to the next one that
/// starts a bullet of its own — joined back into the sentence the PRD wrote.
fn distribution_bullet(opening: &str) -> String {
    let lines = distribution();
    let at = lines
        .iter()
        .position(|line| line.starts_with(&format!("- {opening}")))
        .unwrap_or_else(|| panic!("PRD §7 M3 has a bullet opening `{opening}`"));
    let mut held = vec![lines[at].clone()];
    for line in &lines[at + 1..] {
        if line.starts_with("- ") || line.trim().is_empty() {
            break;
        }
        held.push(line.clone());
    }
    held.join(" ")
}

/// A bullet's list of items: everything after the first `: ` (where the bullet
/// has a lead-in), split on `, `.
fn items(bullet: &str, lead_in: Option<&str>) -> Vec<String> {
    let list = match lead_in {
        Some(lead_in) => {
            bullet
                .split_once(lead_in)
                .unwrap_or_else(|| panic!("the bullet opens with `{lead_in}`"))
                .1
        }
        None => bullet.trim_start_matches("- "),
    };
    list.trim()
        .trim_end_matches('.')
        .split(", ")
        .map(|item| item.trim().to_string())
        .collect()
}

/// One test in `tests/compiled_graph_acceptance.rs`: its name, and the `#[ignore]` it
/// carries if it carries one.
struct Test {
    name: String,
    ignore: Option<Ignore>,
}

impl Test {
    /// The reason its `#[ignore]` gives, if it carries one and it gives one.
    fn reason(&self) -> Option<&str> {
        match &self.ignore {
            Some(Ignore::Because(reason)) => Some(reason),
            Some(Ignore::Bare) | None => None,
        }
    }

    /// How its attribute reads, for a failure message.
    fn attribute(&self) -> String {
        match &self.ignore {
            Some(Ignore::Bare) => "#[ignore]".to_string(),
            Some(Ignore::Because(reason)) => format!("#[ignore = \"{reason}\"]"),
            None => "no `#[ignore]`".to_string(),
        }
    }
}

/// An `#[ignore]` attribute, in either of its two spellings.
///
/// A **bare** `#[ignore]` is kept apart from `#[ignore = "…"]` rather than read
/// as "not ignored": both stop the test from running, so a parser that saw only
/// the spelling with a reason would let a `Live` row pass while its test never
/// ran — the one thing this file exists to make impossible. It fails both
/// branches of [`a_pending_criterion_names_what_must_land_first`]: `Live`
/// because the test is ignored, `Pending` because it names no feature.
enum Ignore {
    Bare,
    Because(String),
}

/// Every `#[test]` in the acceptance suite, read from its source.
///
/// Reading the source rather than asking the harness is what lets this file
/// assert about `#[ignore]` **reasons**, which no test-runner API exposes — and
/// the reason is exactly what tells a later PR which feature un-ignores which
/// test.
fn suite() -> Vec<Test> {
    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compiled_graph_acceptance.rs"),
    )
    .expect("the acceptance suite is readable");
    parse(&source)
}

/// The parser [`suite`] reads with, over any source text — which is how the
/// two `#[ignore]` spellings are themselves tested.
fn parse(source: &str) -> Vec<Test> {
    let mut tests = Vec::new();
    let mut is_test = false;
    let mut ignore = None;
    for line in source.lines() {
        let line = line.trim();
        if line == "#[test]" {
            is_test = true;
        } else if line == "#[ignore]" {
            ignore = Some(Ignore::Bare);
        } else if let Some(reason) = line
            .strip_prefix("#[ignore = \"")
            .and_then(|rest| rest.strip_suffix("\"]"))
        {
            ignore = Some(Ignore::Because(reason.to_string()));
        } else if let Some(rest) = line.strip_prefix("fn ") {
            if is_test {
                let name = rest.split('(').next().expect("a function name").to_string();
                tests.push(Test { name, ignore });
            }
            is_test = false;
            ignore = None;
        }
    }
    tests
}

/// The criteria are PRD §7 M1's own phrases, split from its own sentences.
///
/// The two enumerated bullets are compared item by item, so a criterion this
/// file paraphrases — or one the PRD adds — is a failure rather than a silent
/// gap. The third bullet is one claim rather than a list, so it is checked by
/// containment.
#[test]
fn the_inventory_transcribes_the_prd_m1_bullets() {
    let codegen = items(
        &bullet("IR → deterministic LangGraph TypeScript"),
        Some("LangGraph TypeScript: "),
    );
    assert_eq!(
        codegen,
        CODEGEN
            .iter()
            .map(|criterion| criterion.phrase.to_string())
            .collect::<Vec<_>>(),
        "the codegen bullet and the inventory's transcription of it have diverged"
    );

    let commands = items(&bullet("`agent-compose build`"), None);
    assert_eq!(
        commands,
        COMMANDS
            .iter()
            .map(|criterion| criterion.phrase.to_string())
            .collect::<Vec<_>>(),
        "the commands bullet and the inventory's transcription of it have diverged"
    );

    let harness = bullet("Mock provider server");
    assert_eq!(
        harness.trim_start_matches("- "),
        HARNESS[0].phrase,
        "the harness bullet and the inventory's transcription of it have diverged"
    );
}

/// M3's durability rows are the PRD's own phrases too.
///
/// Containment rather than a split, for `HARNESS`'s reason: the bullet is one
/// sentence and not a list. It is normalized first because the PRD wraps its
/// prose, so a phrase that reads as one clause spans two lines in the file —
/// which is a fact about the margin rather than about the promise.
#[test]
fn the_durability_rows_transcribe_the_prd_m3_bullet() {
    let bullet = distribution_bullet("**Durable execution**");
    let flattened = bullet.split_whitespace().collect::<Vec<_>>().join(" ");
    for criterion in DURABILITY {
        assert!(
            flattened.contains(criterion.phrase),
            "`{}` is not a phrase of PRD §7 M3's durable-execution bullet: {flattened}",
            criterion.phrase
        );
    }
}

/// The built-in rows are phrases of PRD §7 M3's second bullet.
///
/// Containment and normalization, for the reason the sibling above gives: the
/// bullet is one sentence about two things — provider-executed server tools and
/// runtime ones — and the PRD wraps its prose, so a clause a reader hears as one
/// spans two lines in the file.
#[test]
fn the_builtin_rows_transcribe_the_prd_m3_bullet() {
    let bullet = distribution_bullet("**Built-in tools**");
    let flattened = bullet.split_whitespace().collect::<Vec<_>>().join(" ");
    for criterion in BUILTINS {
        assert!(
            flattened.contains(criterion.phrase),
            "`{}` is not a phrase of PRD §7 M3's built-in-tools bullet: {flattened}",
            criterion.phrase
        );
    }
}

/// The rows CLAUDE.md contributes are its own phrases too.
#[test]
fn the_strategy_rows_transcribe_claude_md() {
    let agreement =
        fs::read_to_string(repository().join("CLAUDE.md")).expect("CLAUDE.md is readable");
    for criterion in STRATEGY {
        assert!(
            agreement.contains(criterion.phrase),
            "`{}` is not a phrase of CLAUDE.md's validation strategy",
            criterion.phrase
        );
    }
}

/// Every test a row names exists in the acceptance suite.
#[test]
fn every_named_test_exists() {
    let tests = suite();
    let suite: BTreeSet<&str> = tests.iter().map(|test| test.name.as_str()).collect();
    for criterion in rows() {
        assert!(
            !criterion.tests.is_empty(),
            "`{}` names no test, so nothing decides it",
            criterion.phrase
        );
        for (test, _) in criterion.tests {
            assert!(
                suite.contains(test),
                "`{}` names `{test}`, which is not a test in `tests/compiled_graph_acceptance.rs`",
                criterion.phrase
            );
        }
    }
}

/// Every test in the acceptance suite is named by a row.
///
/// The direction that keeps the plan honest: a test written outside the
/// inventory is a criterion nobody wrote down, and a criterion nobody wrote down
/// is one the next milestone review cannot see.
#[test]
fn the_inventory_accounts_for_every_acceptance_test() {
    let inventoried: BTreeSet<&str> = rows()
        .flat_map(|criterion| criterion.tests.iter().map(|(test, _)| *test))
        .collect();
    let orphans: Vec<String> = suite()
        .into_iter()
        .filter(|test| !inventoried.contains(test.name.as_str()))
        .map(|test| test.name)
        .collect();
    assert!(
        orphans.is_empty(),
        "these acceptance tests belong to no M1 criterion: {orphans:?}"
    );
}

/// A row's status is the test's `#[ignore]` attribute, verbatim.
///
/// This is what makes un-ignoring the definition of done: a `Live` row whose
/// test is still ignored is a criterion claimed and not met, and a `Pending` row
/// whose test has been un-ignored is one met and not claimed. Comparing the
/// reason **verbatim** also means the inventory says which feature unlocks which
/// test, in the same words the source does.
#[test]
fn a_pending_criterion_names_what_must_land_first() {
    let suite = suite();
    for criterion in rows() {
        for (name, status) in criterion.tests {
            let test = suite
                .iter()
                .find(|test| test.name == *name)
                .unwrap_or_else(|| panic!("`{name}` is a test"));
            match status {
                Status::Live => assert!(
                    test.ignore.is_none(),
                    "`{name}` is inventoried as live but carries `{}`",
                    test.attribute()
                ),
                Status::Pending(reason) => {
                    assert_eq!(
                        test.reason(),
                        Some(*reason),
                        "`{name}` is inventoried as pending `{reason}`, and its \
                         attribute says otherwise (`{}`)",
                        test.attribute()
                    );
                    assert!(
                        reason.starts_with("pending: "),
                        "a pending reason names the milestone and the feature: `{reason}`"
                    );
                }
            }
        }
    }
}

/// The parser behind every assertion above reads **both** `#[ignore]`
/// spellings.
///
/// The bare one is the hole worth closing by hand: `#[ignore]` with no reason
/// stops a test from running exactly as the other spelling does, so a parser
/// that missed it would report a `Live` row as met while its test never ran —
/// and this file's whole job is making that impossible. It is neither
/// not-ignored (so a `Live` row fails) nor a named reason (so a `Pending` row
/// fails), which is what the last two cases assert.
#[test]
fn both_ignore_spellings_are_read_and_a_bare_one_claims_nothing() {
    let parsed = parse(
        r#"
        #[test]
        fn it_runs() {}

        #[test]
        #[ignore = "pending: something must land first"]
        fn it_is_pending() {}

        #[test]
        #[ignore]
        fn it_is_bare() {}

        fn not_a_test() {}
        "#,
    );

    let names: Vec<&str> = parsed.iter().map(|test| test.name.as_str()).collect();
    assert_eq!(names, ["it_runs", "it_is_pending", "it_is_bare"]);

    assert!(parsed[0].ignore.is_none());
    assert_eq!(parsed[0].reason(), None);

    assert_eq!(
        parsed[1].reason(),
        Some("pending: something must land first")
    );
    assert_eq!(
        parsed[1].attribute(),
        "#[ignore = \"pending: something must land first\"]"
    );

    assert!(
        parsed[2].ignore.is_some(),
        "a bare `#[ignore]` is ignored, so a `Live` row naming it fails"
    );
    assert_eq!(
        parsed[2].reason(),
        None,
        "…and it names no feature, so a `Pending` row naming it fails too"
    );
    assert_eq!(parsed[2].attribute(), "#[ignore]");
}

/// Every criterion is decided by a test, and every bullet contributes criteria —
/// a count worth pinning, because a milestone that loses a bullet loses it
/// quietly.
#[test]
fn every_bullet_contributes_criteria() {
    for (bullet, expected, what) in [
        (Bullet::Codegen, 10, "the codegen bullet's ten items"),
        (Bullet::Commands, 4, "the commands bullet's four items"),
        (Bullet::Harness, 1, "the harness bullet's single claim"),
        (Bullet::Strategy, 2, "CLAUDE.md's two generated-code gates"),
        (
            Bullet::Durability,
            3,
            "the durable-execution bullet's three claims",
        ),
        (
            Bullet::BuiltinTools,
            3,
            "the built-in tools bullet's runtime half: the tools, the root, the timeout",
        ),
        (
            Bullet::HttpEvents,
            5,
            "the http-native events bullet's five claims",
        ),
    ] {
        let count = rows()
            .filter(|criterion| criterion.bullet == bullet)
            .count();
        assert_eq!(count, expected, "{what}");
    }
}

/// Every acceptance fixture the suite names is a project on disk.
#[test]
fn every_acceptance_fixture_exists() {
    let projects = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/projects/execution");
    let mut found: Vec<String> = fs::read_dir(&projects)
        .expect("the acceptance fixture corpus exists")
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.path().is_dir())
        .map(|entry| entry.file_name().to_string_lossy().into_owned())
        .collect();
    found.sort();

    let source = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/compiled_graph_acceptance/harness.rs"),
    )
    .expect("the harness is readable");
    for name in &found {
        assert!(
            source.contains(&format!("\"{name}\"")),
            "the fixture project `{name}` is not in the harness's `FIXTURES` list, \
             so nothing validates it"
        );
        assert!(
            projects.join(name).join("main.yml").is_file(),
            "the fixture project `{name}` has no entrypoint"
        );
    }
    assert!(!found.is_empty(), "the acceptance suite has fixtures");
}
