//! The M1 completion inventory: every criterion PRD §7 M1 promises, mapped to
//! the acceptance test that decides it and to whether that test runs yet.
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
}

/// Whether a criterion's tests run today.
enum Status {
    /// They run. This PR delivers what they test.
    Live,
    /// They are `#[ignore]`d, with this reason — verbatim, so the attribute and
    /// the inventory cannot drift apart.
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
            // what the provider is offered, and what a call to it does in a
            // release that does not instantiate from there, belong to this
            // criterion rather than to the agent's.
            (
                "a_flow_attached_as_a_tool_reaches_the_model_and_refuses_the_call",
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
            // And the one outcome a one-shot CLI run cannot carry to the end: a
            // `human` pause, whose answer arrives through `serve`'s resume route
            // (grammar 8.7, PRD 5.11). What `run` owes there is an exit path of
            // its own rather than one of the two it already has.
            (
                "run_reports_the_pause_it_cannot_answer_and_exits_on_its_own_code",
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
            (
                "two_pauses_in_one_execution_are_addressed_by_their_instance_paths",
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

fn rows() -> impl Iterator<Item = &'static Criterion> {
    CODEGEN
        .iter()
        .chain(COMMANDS)
        .chain(HARNESS)
        .chain(STRATEGY)
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
