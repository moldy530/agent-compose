//! The M1 acceptance suite: PRD §7 M1, decomposed into observable behaviours and
//! made runnable.
//!
//! CLAUDE.md's validation strategy asks for a milestone's acceptance criteria to
//! be **defined as runnable tests before implementation of that milestone
//! begins**. This file is that definition. Each test names one thing PRD §7 M1
//! promises, in terms of what a compiled graph does when it runs — what it sends
//! to the provider, what it writes to state, what it prints — rather than in
//! terms of the TypeScript it is made of. `tests/acceptance_inventory.rs` is the map from
//! the PRD's own phrases to these test names, and it fails if either side moves.
//!
//! # Live and pending
//!
//! Two kinds of test live here, and the difference is visible in one attribute:
//!
//! * **Live** — the harness itself: the mock provider, the fixture projects, the
//!   scripting and recording every other test will lean on. These run today, for
//!   real, because PRD §7 M1's third bullet ("Mock provider server + e2e
//!   harness") is what this PR delivers.
//! * **Pending** — everything whose subject is codegen. Each carries
//!   `#[ignore = "pending: …"]` naming what must land first, and each has a **real
//!   body** written against the harness: build the fixture, run it against the
//!   mock, assert the transcript and the output. Nothing is stubbed, so
//!   **un-ignoring is the definition of done** — a codegen PR removes the
//!   attribute and either the test passes or the feature is not finished.
//!
//! A pending test that cannot even be written would be a gap in the plan; that
//! is why they are written now, when the plan is what is being reviewed.
//!
//! # No API keys, no network
//!
//! Every fixture's providers declare `base_url: ${MOCK_BASE_URL}`, and
//! [`harness::environment`] resolves it to a `MockProvider` on loopback. The
//! composition is identical to one that talks to a real provider (PRD 5.9: env
//! refs survive unresolved into the IR), which is the point — the harness tests
//! the graph that would ship.
//!
//! # How many model calls an agent node makes
//!
//! A pending test scripts answers *before* the run, so it has to know how many
//! calls the node will make and what each one pins. One rule fixes both, and it
//! is forced rather than chosen: **structured output is asked for by pinning the
//! output tool by name** (PRD 5.2, `WIRE-NOTES.md` (1)), and a pinned tool choice
//! is a promise that the pinned tool is what gets called. So:
//!
//! * an agent with **no tools and no stores** makes **one** call — its output
//!   schema offered as the single tool, pinned — and `Outcome::structured` is
//!   what answers it, rendered under whichever name codegen chose;
//! * an agent **with** tools or attached stores runs its loop first, on calls
//!   that offer those tools and pin **nothing** (a pinned choice would make the
//!   loop unreachable), and the pinned output call is what ends it. Its first
//!   recorded call is therefore a loop call, answered with `Outcome::text` or
//!   `Outcome::tool_calls`, and `Outcome::structured` answers the last one.
//!
//! The mock enforces exactly this: a `structured` reply to a request that pinned
//! nothing, a `text` reply to one that pinned a tool, and a tool call beside the
//! pinned one are each refused as a `script-mismatch` rather than answered — as
//! is a scripted stop reason the surface does not send or the reply's own body
//! cannot carry, which is the field a tool loop branches on (WIRE-NOTES (17)). A
//! test that scripts the wrong number of calls fails loudly instead of passing
//! against a transcript no provider could produce — and the refusal is recorded
//! as one, so `snapshot().is_drained()` is false for a run that hit it even
//! though the 422 went to the generated process rather than to the test.

// A test target is a crate root, so its submodules resolve against `tests/`
// rather than against a directory named after the file. The path keeps the
// harness beside the suite it serves instead of loose in `tests/`, where cargo
// would not build it as a target but a reader would have to guess what it is
// for.
#[path = "compiled_graph_acceptance/harness.rs"]
mod harness;

use std::collections::BTreeSet;
use std::path::Path;
use std::process::Command;
use std::time::Duration;

use mock_provider::{
    Client, HARNESS_HEADER, MockProvider, Outcome, REFUSED_INVALID, REFUSED_UNSCRIPTED,
    RecordedRequest, Request, Script, StructuredOutput, Surface, ToolCall,
};
use serde_json::{Value, json};

/// The model ids the fixtures bind, which are the keys their scripts use.
const SONNET: &str = "claude-sonnet-4-6";
const HAIKU: &str = "claude-haiku-4-5";
const LOCAL: &str = "qwen3-coder-30b";

// ---------------------------------------------------------------------------
// PRD §7 M1, bullet 3 — "Mock provider server + e2e harness". Live.
// ---------------------------------------------------------------------------

/// Every fixture the suite is written against is a valid composition **today**,
/// under the real `validate`.
///
/// This is what keeps the pending tests honest: an ignored test whose project
/// had quietly stopped validating would be waiting on nothing. It is also the
/// suite's own regression net — a grammar change that invalidates a fixture
/// fails here rather than in a codegen PR three milestones later.
#[test]
fn the_acceptance_fixtures_validate_clean() {
    for name in harness::FIXTURES {
        let output = harness::validate(name, "local");
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert_eq!(
            output.status.code(),
            Some(0),
            "the acceptance fixture `{name}` does not validate:\n{stderr}"
        );
        assert!(
            stderr.ends_with("is valid (target `local`)\n"),
            "unexpected verdict for `{name}`: {stderr}"
        );
    }
}

/// A generated process is handed the environment the harness names, and nothing
/// else — whether it was started by `run` or by `serve`.
///
/// Both commands start a process that resolves a fixture's env refs at process
/// **start** (PRD 5.9), so both need the same sealed environment, and for the
/// same two reasons: the presence check is only testable if a variable left out
/// is really absent, and a developer machine that exports one a fixture
/// references must not make a run behave one way there and another in CI. The
/// two halves are asserted to agree here rather than read as agreeing, because
/// they once did not.
#[test]
fn a_generated_process_is_handed_a_sealed_environment() {
    // Cargo exports this into every test binary it runs, which makes it a
    // variable that really is in the ambient environment on every machine this
    // test runs on — the shape of the problem, without setting one.
    const AMBIENT: &str = "CARGO_PKG_NAME";
    assert!(
        std::env::var(AMBIENT).is_ok(),
        "cargo exports `{AMBIENT}` into its test binaries"
    );

    let mut command = Command::new("env");
    harness::seal(
        &mut command,
        &[(harness::API_KEY.to_string(), "supplied".to_string())],
    );
    let printed = command.output().expect("`env` prints an environment");
    let printed = String::from_utf8_lossy(&printed.stdout);
    let carried: BTreeSet<&str> = printed
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name)
        .collect();

    let mut named: BTreeSet<&str> = ["NO_COLOR", harness::API_KEY].into_iter().collect();
    named.extend(
        harness::MACHINE
            .iter()
            .copied()
            .filter(|name| std::env::var_os(name).is_some()),
    );
    assert_eq!(
        carried, named,
        "a sealed environment carries what the harness named and nothing else"
    );
}

/// Both provider surfaces answer, with no API key anywhere and nothing but
/// loopback.
///
/// The two HTTP protocols grammar 12.1's six kinds funnel into: `anthropic` on
/// Messages, and `openai`/`openai_compatible`/`azure_openai` on Chat
/// Completions. A fixture exists for each (`agent-anthropic`, `agent-openai`),
/// and this is the harness-level statement that both are served.
#[test]
fn the_harness_serves_both_provider_surfaces_without_api_keys() {
    let provider = MockProvider::start().expect("the harness binds a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::text("answered on the Messages API")),
        Script::new(LOCAL, Outcome::text("answered on Chat Completions")),
    ]);
    let client = provider.client();

    let messages = client
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": SONNET,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the Messages surface answers");
    assert_eq!(messages.status, 200);

    let completions = client
        .send(
            Request::post("/v1/chat/completions")
                .openai_auth()
                .json(&json!({
                    "model": LOCAL,
                    "messages": [{ "role": "user", "content": "go" }],
                })),
        )
        .expect("the Chat Completions surface answers");
    assert_eq!(completions.status, 200);

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.surface)
            .collect::<Vec<_>>(),
        [Surface::Anthropic, Surface::OpenAi]
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert_eq!(
        harness::environment(&provider)
            .iter()
            .find(|(name, _)| name == harness::API_KEY)
            .map(|(_, value)| value.as_str()),
        Some("mock-provider-key"),
        "the only credential in play is a placeholder"
    );
}

/// A scripted structured output is what an agent node's model call receives, on
/// both surfaces and through all three mechanisms.
///
/// PRD 5.2 is the load-bearing surface: every agent declares an output schema,
/// so every agent's model call asks for structured output, and the harness has
/// to be able to answer that ask in the provider's own idiom — a forced tool on
/// Anthropic, `response_format` or a forced function on OpenAI. A test writes
/// the object the agent should produce and never writes a wire shape.
#[test]
fn a_scripted_structured_output_is_what_an_agent_node_will_receive() {
    let provider = MockProvider::start().expect("a loopback port");
    let client = provider.client();
    let produced = json!({ "verdict": "approve", "feedback": "" });

    provider.enqueue(Script::new(SONNET, Outcome::structured(produced.clone())));
    let answer = client
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .json(&anthropic_agent_request()),
        )
        .expect("the Messages surface answers")
        .json();
    assert_eq!(answer["stop_reason"], "tool_use");
    assert_eq!(answer["content"][0]["input"], produced);

    provider.enqueue(Script::new(LOCAL, Outcome::structured(produced.clone())));
    let answer = client
        .send(
            Request::post("/v1/chat/completions")
                .openai_auth()
                .json(&openai_agent_request()),
        )
        .expect("the Chat Completions surface answers")
        .json();
    let content = answer["choices"][0]["message"]["content"]
        .as_str()
        .expect("the structured object is serialized into the content");
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("it parses"),
        produced
    );

    let recorded = provider.requests();
    assert!(matches!(
        recorded[0].structured_output,
        Some(StructuredOutput::ForcedTool { .. })
    ));
    assert!(matches!(
        recorded[1].structured_output,
        Some(StructuredOutput::JsonSchema { .. })
    ));
}

/// Every failover condition PRD 5.9 names is scriptable, in the shape generated
/// code will classify it from.
///
/// This is the harness half of "model routing with trace-recorded failover": the
/// sequence below — the route's first member refusing, its second answering — is
/// exactly what a compiled `model.default` will produce, and the transcript is
/// how a test sees which member served.
#[test]
fn a_scripted_failover_condition_is_what_a_model_route_will_see() {
    let provider = MockProvider::start().expect("a loopback port");
    let client = provider.client().with_timeout(Duration::from_millis(500));
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "42" }))),
    ]);

    let refused = client
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .json(&structured_request(SONNET, "answerer_output")),
        )
        .expect("the first member answers");
    assert_eq!(
        refused.status, 429,
        "the condition generated code routes on"
    );
    assert_eq!(refused.header("retry-after"), Some("1"));

    let served = client
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .json(&structured_request(HAIKU, "answerer_output")),
        )
        .expect("the fallback answers");
    assert_eq!(served.status, 200);
    assert_eq!(served.json()["content"][0]["input"]["answer"], "42");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.model.as_str())
            .collect::<Vec<_>>(),
        [SONNET, HAIKU],
        "the transcript is what a failover assertion reads"
    );
    assert!(provider.snapshot().is_drained());
}

/// Every model call is recorded, in arrival order, with its parsed body — the
/// transcript an acceptance assertion is written against.
#[test]
fn every_model_call_is_recorded_in_order_with_its_parsed_body() {
    let provider = MockProvider::start().expect("a loopback port");
    let client = provider.client();
    provider.enqueue(Script::new(SONNET, Outcome::text("ok")).times(3));

    for pass in 0..3 {
        let response = client
            .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
                "model": SONNET,
                "max_tokens": 4096,
                "system": "You are a meticulous technical reviewer.",
                "messages": [{ "role": "user", "content": format!("pass {pass}") }],
            })))
            .expect("the surface answers");
        assert_eq!(response.status, 200);
    }

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 3);
    for (index, request) in recorded.iter().enumerate() {
        assert_eq!(request.sequence, index as u64 + 1);
        assert_eq!(request.model, SONNET);
        assert_eq!(request.path, "/v1/messages");
        assert_eq!(
            request.body()["messages"][0]["content"],
            format!("pass {index}")
        );
        assert_eq!(
            request.body()["system"],
            "You are a meticulous technical reviewer.",
            "the agent's literal prompt reaches the provider as the system turn"
        );
    }
    assert!(provider.snapshot().is_drained());
}

/// A request no codegen should send is refused in the provider's own shape and
/// recorded with the field that is wrong — and consumes no scripted outcome.
///
/// This is the harness property the whole suite depends on: the mock is strict,
/// so a graph that builds a malformed tool loop fails the acceptance run instead
/// of passing it. The malformed request below is the ordering mistake a tool
/// loop makes when it appends results after the next user turn.
#[test]
fn a_request_no_codegen_should_send_is_refused_and_recorded() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(SONNET, Outcome::text("never served")));

    let refused = provider
        .client()
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": SONNET,
            "max_tokens": 4096,
            "messages": [
                { "role": "user", "content": "review it" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": {} },
                ]},
                { "role": "user", "content": [
                    { "type": "text", "text": "here you go" },
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "x" },
                ]},
            ],
            "tools": [{ "name": "lookup", "input_schema": { "type": "object" } }],
        })))
        .expect("the surface answers");

    assert_eq!(refused.status, 400);
    assert_eq!(refused.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let recorded = provider.requests();
    assert!(!recorded[0].is_valid());
    assert_eq!(recorded[0].failures()[0].pointer, "messages.2.content.1");
    assert_eq!(
        provider.snapshot().queues[SONNET],
        1,
        "one bad call must not eat the answer meant for the next one"
    );
}

/// An unscripted model call is refused loudly, naming the model — never answered
/// with a default, and never retried past.
///
/// Determinism is the reason: a default answer is how an acceptance test starts
/// passing for a reason nobody chose. The status has to survive two mechanisms
/// that would otherwise hide it — PRD 5.9's failover, and the client SDKs' own
/// default retry set — because a refusal that is retried twice arrives as three
/// transcript entries and a failure whose shape does not match its cause.
#[test]
fn an_unscripted_model_call_fails_the_run_loudly() {
    let provider = MockProvider::start().expect("a loopback port");
    let refused = provider
        .client()
        .send(Request::post("/v1/messages").anthropic_auth().json(&json!({
            "model": SONNET,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the surface answers");

    assert_eq!(refused.status, mock_provider::HARNESS_STATUS);
    assert_eq!(refused.header(HARNESS_HEADER), Some(REFUSED_UNSCRIPTED));
    let message = refused.json()["error"]["message"]
        .as_str()
        .expect("a message")
        .to_string();
    assert!(message.contains(SONNET), "{message}");
    // 408, 409, 429 and 5xx are what `@anthropic-ai/sdk` and `openai` retry by
    // default, and 429/5xx are what PRD 5.9 fails over on. A harness refusal
    // must be none of them.
    let status = refused.status;
    assert!(
        !matches!(status, 408 | 409 | 429) && !(500..600).contains(&status),
        "a harness refusal must be neither a failover condition nor an SDK retry: {status}"
    );
    assert_eq!(provider.snapshot().unscripted, 1);
}

/// The harness can force completion order to differ from item order, which is
/// the precondition for asserting anything about index-tagged reducers.
///
/// A fan-out's instances share one model id and reach the server in scheduler
/// order, so the outcomes are narrowed by the item they answer; the delayed one
/// still gets its own answer, and finishes last.
#[test]
fn a_scripted_delay_makes_completion_order_differ_from_item_order() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "result": "first item" }))
                .after(Duration::from_millis(250)),
        )
        .matching("item-0"),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "result": "second item" })),
        )
        .matching("item-1"),
    ]);

    let client = provider.client();
    let started = std::time::Instant::now();
    let finished = std::thread::scope(|scope| {
        let handles: Vec<_> = ["item-0", "item-1"]
            .into_iter()
            .map(|item| {
                let client = client.clone();
                scope.spawn(move || {
                    let answer = client
                        .send(
                            Request::post("/v1/messages")
                                .anthropic_auth()
                                .json(&item_request(item)),
                        )
                        .expect("the surface answers")
                        .json();
                    (
                        answer["content"][0]["input"]["result"]
                            .as_str()
                            .expect("a structured result")
                            .to_string(),
                        started.elapsed(),
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|handle| handle.join().expect("the call finished"))
            .collect::<Vec<_>>()
    });

    assert_eq!(finished[0].0, "first item", "each item got its own answer");
    assert_eq!(finished[1].0, "second item");
    assert!(
        finished[1].1 < finished[0].1,
        "item 1 finished first: {:?} vs {:?}",
        finished[1].1,
        finished[0].1
    );
}

// ---------------------------------------------------------------------------
// PRD §7 M1, bullet 1 — "IR → deterministic LangGraph TypeScript".
// The state model has landed; everything below it is pending.
// ---------------------------------------------------------------------------

/// Every `state:` channel reaches the emitted state model with its declared
/// type, its `default:` as an initial value, and its `reduce:` policy as a
/// reducer (grammar 10.1, 10.2, 7.6.4).
///
/// This is the emission half of "state models", decided through the real
/// command. The other half — that those channel specs *behave* — is
/// `compose-core`'s `tests/generated_code_gates.rs`, which invokes the compiled
/// graph under the pinned LangGraph and compares the state at quiescence against
/// what the policies say (`the_emitted_state_model_reduces_the_way_its_policies_say`).
/// What neither can reach from here is the third half, which
/// `state_channels_carry_their_declared_types_and_defaults` names: a *flow*
/// returning them, which needs `agent-compose run`.
#[test]
fn the_state_model_carries_every_channels_type_default_and_reduce_policy() {
    let built = harness::build("agent-anthropic", "local");
    built.succeeded();
    let state = built.read("src/state.ts");
    let schemas = built.read("src/schemas.ts");

    // Two defaulted channels of different types: the typed default is the
    // literal the spec wrote, not its string spelling.
    assert!(
        state.contains("default: () => \"approve\","),
        "the enum channel's `default: approve` is its initial value:\n{state}"
    );
    assert!(
        state.contains("default: () => 1,"),
        "the integer channel's default is the number `1`, not `\"1\"`:\n{state}"
    );
    assert!(
        state.contains("default: () => \"none yet\","),
        "the string channel's default is its initial value:\n{state}"
    );

    // `reduce: append` is the one policy whose update type differs from the
    // channel type: a write supplies one element (grammar 10.2, D58), and the
    // channel starts at the policy's identity element rather than unset.
    assert!(
        state.contains(
            "notes: Annotation<z.infer<typeof stateNotes>, z.infer<typeof stateNotes>[number]>({"
        ),
        "an `append` channel takes one element per write:\n{state}"
    );
    assert!(
        state.contains("reducer: (left, right) => left.concat([right]),"),
        "and appends it in write order:\n{state}"
    );
    assert!(
        state.contains("default: () => [],"),
        "starting from the empty array:\n{state}"
    );

    // The declared type is the schema, not a restatement of it.
    assert!(
        schemas.contains("export const stateVerdict = z.enum([\"approve\", \"revise\"])"),
        "the channel's type is lowered per grammar 3.8:\n{schemas}"
    );
    assert!(
        schemas.contains(".default(\"approve\");"),
        "carrying its `default:` into the schema as well as into the channel:\n{schemas}"
    );
    assert!(
        schemas.contains("export const stateNotes = z.array(z.string()).max(8)"),
        "including the array's bound:\n{schemas}"
    );
}

/// A tagged union reaches the emitted schemas as a `z.discriminatedUnion` whose
/// variants are narrowed one by one and closed (PRD 5.2, grammar 3.7).
///
/// The emission half of the same criterion, over the shape the fan-out example
/// is built on. That the schema then *refuses* a tag it does not declare is
/// `compose-core`'s `the_emitted_zod_agrees_with_the_json_schema_lowering`, which
/// runs exactly that document through both columns of grammar 3.8's table; that
/// the refusal becomes a *run* failure naming the tag is what
/// `a_tagged_union_output_is_narrowed_per_variant_and_a_bad_tag_is_rejected`
/// waits on a node function for.
#[test]
fn a_tagged_union_output_is_emitted_as_a_discriminated_union_narrowed_per_variant() {
    let built = harness::build("fanout", "local");
    built.succeeded();
    let schemas = built.read("src/schemas.ts");

    assert!(
        schemas.contains("z.discriminatedUnion(\"kind\", ["),
        "the union is discriminated on its declared tag field:\n{schemas}"
    );
    for (tag, field) in [
        ("auto_fixable", "hint: z.string()"),
        ("needs_human", "severity: z.enum(["),
    ] {
        let variant = format!("kind: z.literal(\"{tag}\"),");
        assert!(
            schemas.contains(&variant),
            "the variant `{tag}` is pinned to its tag:\n{schemas}"
        );
        let payload = schemas
            .split(&variant)
            .nth(1)
            .expect("the variant was just found");
        let payload = payload
            .split("}).strict()")
            .next()
            .expect("a closed variant");
        assert!(
            payload.contains(field),
            "and carries its own payload `{field}`, not another variant's:\n{payload}"
        );
    }
}

/// State channels carry their declared types, defaults, and reduce policies into
/// the running graph.
///
/// The fixture's four channels split into two halves on purpose, because a test
/// whose every expected value is also the scripted value cannot tell a default
/// from a write. `verdict` and `feedback` are written by the agent, and the
/// script below gives both a value the declaration does *not* — `approve` and
/// `none yet` are the defaults. `round` and `notes` are written by nothing, so
/// the only thing that can produce them is the declaration: an `integer`
/// default, and an `append` channel's identity element (grammar 10.1).
///
/// `build` now emits that state model, and `compose-core`'s
/// `tests/generated_code_gates.rs` constructs it under the pinned LangGraph on
/// every `cargo test`. What this test adds is the half only a run can show — that
/// the declared defaults are what a flow *returns*.
#[test]
fn state_channels_carry_their_declared_types_and_defaults() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    // Written by name from the agent's output (grammar 8.0, 10.3), and read back
    // as the flow's outputs — neither value could have come from the default.
    let outputs = run.outputs();
    assert_eq!(outputs["verdict"], "revise", "the default is `approve`");
    assert_eq!(
        outputs["feedback"], "tighten it",
        "the default is `none yet`"
    );

    // Nothing writes these two, so they are the declaration itself. `round` is
    // the typed default — the integer `1`, not the string `\"1\"` — and a build
    // that never applied defaults would fail materialization instead (D78).
    assert_eq!(outputs["round"], json!(1));
    // `notes` declares `reduce: append` and no default, so it starts at the
    // policy's identity element rather than unset (grammar 10.1).
    assert_eq!(outputs["notes"], json!([]));
}

/// A tagged-union output is narrowed per variant, and a response carrying a tag
/// the schema does not declare is rejected before any edge is evaluated.
///
/// The `z.discriminatedUnion` this needs is emitted, and `compose-core`'s
/// `tests/generated_code_gates.rs` runs a corpus through it that includes this
/// very refusal — a tag the union does not declare, rejected by both the emitted
/// Zod and the JSON Schema the same lowering produces. An agent node fn now
/// parses its answer with the emitted schema, which is what turns that refusal
/// into a run failure; what is left is the *fixture's* first node, a `flow:`
/// instantiation of `flow.normalize`, so this run cannot reach `classify` until
/// subgraphs do.
#[test]
#[ignore = "pending: codegen must emit subgraphs"]
fn a_tagged_union_output_is_narrowed_per_variant_and_a_bad_tag_is_rejected() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a report" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [{ "kind": "unheard_of", "file": "a.rs" }],
            })),
        ),
    ]);

    let run = harness::run(
        "fanout",
        "flow.triage",
        &[("report", "a report")],
        &provider,
    );
    let failure = run.failed();
    assert!(
        failure.contains("unheard_of"),
        "a variant the union does not declare is refused by name: {failure}"
    );
    assert!(
        provider.requests().iter().all(RecordedRequest::is_valid),
        "the model call itself was well formed; what failed is the answer's shape"
    );
}

/// An agent node sends its literal prompt, its bound input, and its output
/// schema — and writes the structured output it gets back.
///
/// `agent.reviewer` carries no `tools:`, so this is the single-call shape the
/// module header's *How many model calls an agent node makes* fixes: the output
/// schema offered as a tool and pinned. What an agent's own `tools:` list looks
/// like on the wire belongs to the tool-loop test — a request cannot pin the
/// output tool and leave another one callable, so no single test can assert
/// both.
#[test]
fn an_agent_node_sends_its_prompt_input_and_output_schema() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 1);
    let call = &recorded[0];
    assert!(call.is_valid(), "{:?}", call.failures());
    assert_eq!(call.model, SONNET);
    assert_eq!(
        call.body()["system"],
        "You are a meticulous technical reviewer.\n\nApprove only when the draft fully satisfies the goal. Otherwise ask for a\nrevision and list every required change.\n",
        "the prompt is literal text, sent verbatim (D13)"
    );
    let turn = call.body()["messages"][0]["content"]
        .as_str()
        .expect("the bound input is serialized into the user turn (grammar 5.3)")
        .to_string();
    assert!(
        turn.contains("ship it") && turn.contains("a draft"),
        "{turn}"
    );
    let structured = call
        .structured_output
        .as_ref()
        .expect("an agent always asks for structured output (PRD 5.2)");
    assert_eq!(
        call.tools,
        [structured.name()],
        "a tool-less agent offers exactly one tool: the one carrying its output \
         schema, which `tool_choice` then pins ({:?})",
        call.tools
    );
    assert_eq!(
        structured.schema()["properties"]["verdict"]["enum"],
        json!(["approve", "revise"])
    );
    assert_eq!(run.outputs()["verdict"], "revise");
}

/// The same agent node, compiled against the other HTTP surface: prompt, bound
/// input, declared settings, and a structured output asked for in Chat
/// Completions' own idiom.
///
/// The twin exists because half the provider surface would otherwise never run a
/// compiled graph. `agent-openai` differs from `agent-anthropic` only in
/// `kind: openai_compatible` — which is PRD 5.9's whole claim about the
/// provider/model split — so a graph that works on one and not the other is a
/// codegen bug that no Messages-API test can see, and three of grammar 12.1's
/// six kinds reach this surface.
#[test]
fn an_agent_node_sends_its_prompt_input_and_output_schema_on_chat_completions() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 1);
    let call = &recorded[0];
    assert!(call.is_valid(), "{:?}", call.failures());
    assert_eq!(
        call.surface,
        Surface::OpenAi,
        "an `openai_compatible` provider speaks Chat Completions"
    );
    assert_eq!(call.model, LOCAL);
    assert_eq!(
        call.body()["messages"][0]["role"],
        "system",
        "the agent's prompt is the system turn on this surface too"
    );
    assert_eq!(
        call.body()["messages"][0]["content"],
        "You are a meticulous technical reviewer.\n\nApprove only when the draft fully satisfies the goal. Otherwise ask for a\nrevision and list every required change.\n",
    );
    let turn = call.body()["messages"][1]["content"]
        .as_str()
        .expect("the bound input is serialized into the user turn (grammar 5.3)")
        .to_string();
    assert!(
        turn.contains("ship it") && turn.contains("a draft"),
        "{turn}"
    );
    assert_eq!(
        call.body()["temperature"],
        0.2,
        "the model's declared `settings:` reach the wire (grammar 12.2)"
    );

    // Either idiom is a correct compilation of PRD 5.2 here — `response_format`
    // or a forced function — so what is asserted is that one of them was used
    // and that it carried the agent's own output schema.
    let schema = call
        .structured_output
        .as_ref()
        .expect("an agent always asks for structured output (PRD 5.2)")
        .schema()
        .clone();
    assert_eq!(
        schema["properties"]["verdict"]["enum"],
        json!(["approve", "revise"])
    );
    assert_eq!(run.outputs()["verdict"], "revise");
    assert!(provider.snapshot().is_drained());
}

/// The intra-agent tool loop runs the tool, feeds the result back, and stops at
/// `max_tool_iterations` rather than looping forever.
///
/// `max_tool_iterations` is **Decision D51 and PRD §10's second open question**:
/// the grammar accepts the key (M0), and whether the PRD keeps it is not settled.
/// What this test decides either way is that the loop is bounded — declining D51
/// makes the bound a runtime default rather than a declared one, and rewrites
/// this test's fixture and its first assertion, not its subject.
#[test]
fn an_agent_node_bounds_its_tool_loop_at_max_tool_iterations() {
    let provider = MockProvider::start().expect("a loopback port");
    // `agent.researcher` declares `max_tool_iterations: 2`, so a model that only
    // ever asks for the tool must be stopped after the second call.
    provider.enqueue(
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        )
        .times(8),
    );

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("max_tool_iterations") || failure.contains("tool"),
        "the bound is what stopped it: {failure}"
    );
    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        2,
        "two calls, not eight: the loop is bounded by the agent's own declaration"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert!(
        recorded[0].tools.contains(&"lookup".to_string()),
        "the agent's `tools:` list reaches the provider: {:?}",
        recorded[0].tools
    );
    assert_eq!(
        recorded[0].structured_output, None,
        "a loop call pins no tool — a pinned one is a promise that the *pinned* \
         tool is called, which would make the loop unreachable"
    );
    let second = recorded[1].body()["messages"].clone();
    assert_eq!(
        second[2]["content"][0]["type"], "tool_result",
        "the tool's result leads the turn that answers it"
    );
}

/// The three deterministic node kinds run, and each reads its result the way its
/// own surface says (grammar 8.2, 8.3, 8.4, 6.1).
///
/// One run over `flow.pipeline`, because the claim is about a graph rather than
/// about three constructs in isolation: an inline `http:` node whose envelope
/// `status` and decoded body field arrive together (Decision D56), an inline
/// `exec:` node whose result *is* the process envelope, and a `function:` node
/// over a `tool.*` whose single string-typed property takes raw stdout — the
/// `tool.*`-surface exception of grammar 6.1 that Decision D91 withholds from
/// inline nodes.
#[test]
fn the_deterministic_node_kinds_run_and_decode_their_results() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "fast", "note": "a note" })),
    ));

    let Some(run) = harness::invoke(
        "activities",
        "flow.pipeline",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    let outputs = run.outputs();

    // The `http:` node reached a real server — the mock's own control plane — and
    // its result carries both halves: `status` from the response envelope, and
    // `requests` decoded out of the JSON body.
    assert_eq!(outputs["status"], json!(200));
    // The `exec:` node's `stdout` is the envelope field, appended to a reduced
    // channel one element per write (grammar 10.2, Decision D58).
    assert_eq!(outputs["checks"], json!(["merged"]));
    // `publish` wrote last; `check`'s raw-stdout binding is what it overwrote,
    // and the run visited it.
    assert_eq!(outputs["report"], "published");
    assert!(
        run.visited().contains(&"check".to_string()),
        "the `function:` node ran: {:?}",
        run.visited()
    );
    assert_eq!(provider.requests().len(), 1, "one agent, one call");
}

/// The `function:` binding is the escape hatch, and what it costs is a
/// registration the host has to make (grammar 6.1, PRD 5.5).
///
/// Both halves, because either alone would be a claim about the other: an
/// unregistered function fails the run by name and says how to fix it, and a
/// registered one is called with arguments already parsed against the tool's own
/// declared `input:` — the checked signature grammar 8.4 asks for.
#[test]
fn a_host_registered_function_runs_and_an_unregistered_one_says_so() {
    let provider = MockProvider::start().expect("a loopback port");
    let environment = harness::environment(&provider);

    let Some(unregistered) =
        harness::invoke_with("activities", "flow.hosted", &json!({}), &environment)
    else {
        return;
    };
    let failure = unregistered.failed();
    assert!(
        failure.contains("rank_candidates"),
        "the failure names the registration that is missing: {failure}"
    );
    assert!(
        failure.contains("registerFunction"),
        "…and how to supply it: {failure}"
    );

    let registered = harness::invoke_hosted(
        "activities",
        "flow.hosted",
        &json!({}),
        &environment,
        Some(
            r#"import { registerFunction } from "./src/runtime.ts";
registerFunction("rank_candidates", (args) => ({ ranked: `ranked: ${args.text}` }));
"#,
        ),
    )
    .expect("the toolchain was there a moment ago");
    registered.succeeded();
    assert_eq!(
        registered.outputs()["report"],
        "ranked: candidates",
        "the host's answer reached the channel the node's `writes:` names"
    );
}

/// Two edges of one fork both fire, and the node they meet at runs **once**
/// (grammar 7.3 rule 6, 7.6 P1/P2).
#[test]
fn a_multicast_fork_fires_every_true_edge_and_the_convergence_runs_once() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "fast", "note": "a note" })),
    ));

    let Some(run) = harness::invoke(
        "activities",
        "flow.pipeline",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let plan = run.entries("plan");
    assert_eq!(
        plan[0]["routing"]["targets"],
        json!(["probe", "check"]),
        "both guards were true, so both edges fired (grammar 7.3 rule 6)"
    );
    assert_eq!(
        plan[0]["routing"]["edges"][2]["taken"],
        json!(false),
        "…and the `else:` edge did not, because a guarded sibling was taken"
    );

    // P2: `merge` is targeted by two edges taken in one step and runs once.
    let visited = run.visited();
    assert_eq!(
        visited.iter().filter(|node| *node == "merge").count(),
        1,
        "the convergence ran once: {visited:?}"
    );
    // …in the step after both of its predecessors, which is P1: a node's edges
    // are evaluated only after it has completed.
    let step = |node: &str| run.entries(node)[0]["step"].as_i64().expect("a step");
    assert_eq!(
        step("probe"),
        step("check"),
        "the two branches are one step"
    );
    assert_eq!(step("merge"), step("probe") + 1);
}

/// A guard on an edge leaving `start` decides the first step, over the roots
/// grammar 4.1 gives it (`start` has no output).
#[test]
fn a_guarded_start_edge_decides_the_first_step() {
    let provider = MockProvider::start().expect("a loopback port");
    for (pick, expected) in [(true, "left"), (false, "right")] {
        let Some(run) = harness::invoke_with(
            "activities",
            "flow.entry",
            &json!({ "pick": pick }),
            &harness::environment(&provider),
        ) else {
            return;
        };
        run.succeeded();
        assert_eq!(run.outputs()["report"], expected);
        assert_eq!(
            run.visited(),
            ["$start", expected],
            "the guards are evaluated at a node, because `start` is not one"
        );
    }
}

/// An inline node's parameters reach the process and the wire the way grammar
/// 8.2 and 8.3 say, including the two widenings that turn a failure into data.
///
/// Four claims one run decides, each of which a regression would ship green
/// because the shapes are all *probed* elsewhere and none was asserted:
///
/// 1. **`input:` → environment** (grammar 8.2). The bindings are the object
///    handed to the child as environment variables, upper-snake-cased on the
///    way in, and the block's own `env:` writes into that same environment. The
///    child prints both, so a wrong spelling is a wrong string rather than a
///    silent empty one.
/// 2. **scalar `input:` → stdin** (grammar 8.0's table, Decision D88). `cat`
///    prints what was written there and nothing else.
/// 3. **widened `expect_exit`** (Decision D84). `false` exits 1, which the
///    default `[0]` makes a node error; widened, it completes and its
///    `exit_code` is what an edge guard routes on.
/// 4. **widened `expect_status`** (D84 again, on the response side). The mock
///    404s an unrouted path, which the default (any 2xx) makes a node error;
///    widened, it is a status the node records.
#[test]
fn an_inline_nodes_parameters_reach_the_process_and_the_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke(
        "activities",
        "flow.plumbing",
        &[("goal", "ship-it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    let outputs = run.outputs();

    // 1: `goal:` arrived as `$GOAL`, and the block's `env:` as `$SUFFIX`.
    assert_eq!(
        outputs["report"], "ship-it/from-env",
        "an `input:` binding is upper-snake-cased into the child's environment, \
         beside the block's own `env:` (grammar 8.2)"
    );
    // 2: the scalar binding was the child's stdin, and nothing else was.
    assert_eq!(
        outputs["checks"],
        json!(["ship-it/from-stdin"]),
        "a scalar `input:` goes on stdin (Decision D88)"
    );
    // 3: the run reached `absent` at all, which only the widened `expect_exit`
    // allows — under the default, `tolerated` is a node error and `on_error:
    // fail` aborts before any of this.
    let tolerated = run.entries("tolerated");
    assert_eq!(tolerated[0]["outcome"], "completed");
    assert_eq!(
        tolerated[0]["routing"]["edges"][0]["value"],
        json!(true),
        "`exit_code == 1` is routable data because `expect_exit` was widened \
         (grammar 8.2, Decision D84)"
    );
    assert_eq!(tolerated[0]["routing"]["targets"], json!(["absent"]));
    // 4: a 404 recorded rather than raised.
    assert_eq!(
        outputs["status"],
        json!(404),
        "a status inside a widened `expect_status` completes the node (grammar 8.3)"
    );
    assert_eq!(run.entries("absent")[0]["outcome"], "completed");
}

/// A guard sees its own node's writes and **not** a concurrent sibling's.
///
/// Grammar 7.6 says both of these and they are in tension: P1 is per node ("a
/// node's outgoing edges are evaluated only after **that node** has completed"),
/// while the *Steps* paragraph above it evaluates a step's edges after every
/// node of the step has written. Routing here is part of the node's own task,
/// which is P1's reading, so `bravo`'s guard over `state.beacon` is `false`
/// while `alpha` — running in the same step — is writing `lit` to it.
///
/// This test **pins that reading** rather than endorsing it: it is a reachable
/// routing choice, and a document that does not say which answer is right is
/// what makes it a choice. See `codegen::graph`'s note beside the emitted
/// router; the clarification request goes to the grammar's owner.
///
/// The second guard is the half that has to keep working either way: a node's
/// own writes **are** visible to its own guards, through their channels' reduce
/// policies.
#[test]
fn a_guard_sees_its_own_writes_and_not_a_concurrent_siblings() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "activities",
        "flow.visibility",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    run.succeeded();

    // Both branches ran, in one step: that is what makes the question askable.
    let step = |node: &str| run.entries(node)[0]["step"].as_i64().expect("a step");
    assert_eq!(
        step("alpha"),
        step("bravo"),
        "the fork's branches are one step"
    );

    let bravo = run.entries("bravo");
    let edges = &bravo[0]["routing"]["edges"];
    assert_eq!(
        edges[0]["value"],
        json!(false),
        "`alpha` wrote `beacon` in this same step and `bravo`'s guard does not \
         see it (grammar 7.6 P1, read per node)"
    );
    assert_eq!(
        edges[1]["value"],
        json!(true),
        "`bravo`'s own write is visible to `bravo`'s own guard"
    );
    assert_eq!(bravo[0]["routing"]["targets"], json!(["mine"]));
    assert_eq!(
        run.outputs()["checks"],
        json!(["mine"]),
        "only the own-write branch ran"
    );
    // …and `alpha` really did write it, so the `false` above is about *when* the
    // write is visible rather than about whether it happened.
    assert_eq!(run.entries("alpha")[0]["writes"], json!(["beacon"]));
}

/// Concurrent writers of one `append` channel land in ascending node-id order,
/// whatever order they complete or are declared in (grammar 7.6.4 clause 1).
///
/// The ordering is inherited from the scheduler rather than imposed by anything
/// this compiler emits, which is exactly why it needs an assertion: a pinned
/// LangGraph bump that changed task ordering would silently reorder every
/// `append` channel a fork writes, with a green suite.
///
/// The fixture makes all three orders disagree. `alpha` sorts **first** by node
/// id, is declared **last** among the fork's edges, and finishes **last**
/// (it sleeps 300ms); `zulu` sorts last, is declared first, and finishes first.
/// Only clause 1 puts `alpha` at the head of the result.
#[test]
fn concurrent_writers_append_in_node_id_order_not_completion_order() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "activities",
        "flow.ordering",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    run.succeeded();
    let step = |node: &str| run.entries(node)[0]["step"].as_i64().expect("a step");
    assert_eq!(step("alpha"), step("zulu"), "both writers are one step");
    assert_eq!(
        run.outputs()["checks"],
        json!(["alpha", "zulu"]),
        "writers are ordered by ascending node id (grammar 7.6.4 clause 1), not \
         by the order they completed or were declared in"
    );
}

/// Reading a channel nothing has written fails the **execution**, and no
/// `on_error:` strategy absorbs it (grammar 10.1, Decisions D78, D97, D110).
///
/// The two outcomes are distinct and D97 says so outright: `skip` exists to
/// continue past a *node's* failure, and choosing `false` for a guard over a
/// skipped node's output is what keeps it from being a synonym for `fail`. An
/// unset-channel read is not a node's failure at all — no attempt was made, the
/// activity never ran — so a `skip` that swallowed it would turn "fails the
/// execution naming the channel and the reader" into a silent continue, and a
/// `fallback:` would turn it into a route.
///
/// Both spellings of the read, because they take different paths through the
/// emitted node function: `flow.unset` resolves the tool's argument **by name**
/// from the channel of the same name (grammar 8.0 step 2), and
/// `flow.unset_expression` writes the read out as CEL.
#[test]
fn reading_an_unset_channel_fails_the_run_whatever_the_error_policy_says() {
    let provider = MockProvider::start().expect("a loopback port");
    let environment = harness::environment(&provider);

    let Some(skipped) = harness::invoke_with("activities", "flow.unset", &json!({}), &environment)
    else {
        return;
    };
    let failure = skipped.failed();
    assert!(
        failure.contains("`pending`"),
        "the failure names the channel (grammar 10.1): {failure}"
    );
    assert!(
        failure.contains("is unset") && failure.contains("reads it"),
        "…and the reader: {failure}"
    );
    assert!(
        !skipped.visited().contains(&"after".to_string()),
        "`on_error: skip` did not continue past it: {:?}",
        skipped.visited()
    );

    let bound = harness::invoke_with(
        "activities",
        "flow.unset_expression",
        &json!({}),
        &environment,
    )
    .expect("the toolchain was there a moment ago");
    let failure = bound.failed();
    assert!(
        failure.contains("pending"),
        "the expression's failure names what it could not read: {failure}"
    );
    assert!(
        !bound.visited().contains(&"rescue".to_string()),
        "a `fallback:` did not route around it either: {:?}",
        bound.visited()
    );
}

/// A skipped node writes nothing, and the one thing that changes about its
/// routing is the value of a guard over its own output (grammar 9.2, D97).
#[test]
fn a_skipped_node_routes_through_its_else_edge_and_writes_nothing() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "fast", "note": "a note" })),
    ));

    let Some(run) = harness::invoke(
        "activities",
        "flow.pipeline",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    // `tally` runs `false`, which exits 1 — outside the default `expect_exit:
    // [0]` — so the node errors and its declared `on_error: skip` applies.
    let tally = run.entries("tally");
    assert_eq!(tally[0]["outcome"], "skipped");
    assert_eq!(
        tally[0]["writes"],
        json!([]),
        "a skipped node writes nothing (grammar 9.2)"
    );
    assert_eq!(
        tally[0]["routing"]["edges"][0]["value"],
        json!(false),
        "its guard reads its own output, which is not there, so it is `false` \
         rather than an error (Decision D97)"
    );
    assert_eq!(tally[0]["routing"]["targets"], json!(["publish"]));
    assert!(
        !run.visited().contains(&"rework".to_string()),
        "the guarded branch was not taken"
    );
}

/// A subgraph runs with explicit bindings in, name-based outputs back, and a
/// conversation history of its own.
#[test]
#[ignore = "pending: codegen must emit subgraphs"]
fn a_subgraph_runs_with_explicit_bindings_and_isolated_history() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a normalized report" })),
        )
        .matching("a raw report"),
        Script::new(SONNET, Outcome::structured(json!({ "findings": [] }))),
    ]);

    let run = harness::run(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    );
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    let inside = recorded[0].body()["messages"]
        .as_array()
        .expect("a message list")
        .len();
    assert_eq!(
        inside, 1,
        "the subgraph's agent sees its own history, not the caller's (PRD 5.7)"
    );
    let outside = recorded[1].body()["messages"][0]["content"]
        .as_str()
        .expect("the parent's turn");
    assert!(
        outside.contains("a normalized report"),
        "the subgraph's output crossed back by name: {outside}"
    );
}

/// An edge guard routes on the source node's structured output, deterministically
/// (PRD 5.3).
///
/// The transition is the compiled router's and the value is the model's, which is
/// PRD G4's whole claim: the same graph, the same guards, and the branch decided
/// by a field the model filled in. The trace is asserted beside the outputs
/// because PRD 5.3 asks for routing decisions to *be data* rather than to be
/// inferred from what happened next.
#[test]
fn an_edge_guard_routes_on_the_source_nodes_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "draft": "the first draft" })),
        )
        .matching("research writer"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        )
        .matching("meticulous technical reviewer"),
    ]);

    let Some(run) = harness::invoke(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    assert_eq!(
        provider.requests().len(),
        2,
        "`approve` takes the `else:` edge to `end`, so the writer never runs again"
    );
    assert_eq!(run.outputs()["draft"], "the first draft");
    assert_eq!(run.visited(), ["write", "review"]);

    // The decision, as data: the guarded back-edge was evaluated against the
    // model's own field and came out false, and the `else:` edge fired because no
    // guarded sibling was taken (grammar 7.3 rules 3 and 4).
    let review = run.entries("review");
    let routing = &review[0]["routing"];
    assert_eq!(routing["targets"], json!(["__end__"]));
    assert_eq!(
        routing["edges"][0]["when"],
        "review.output.verdict == 'revise'"
    );
    assert_eq!(routing["edges"][0]["value"], json!(false));
    assert_eq!(routing["edges"][0]["taken"], json!(false));
    assert_eq!(routing["edges"][1]["else"], json!(true));
    assert_eq!(routing["edges"][1]["taken"], json!(true));
}

/// The JS evaluator generated routers embed agrees with the Rust one the
/// validator links, over the shared conformance corpus.
///
/// CLAUDE.md makes this a CI gate: two interpreters is a semantic-drift risk, and
/// the corpus is the mitigation. The Rust side already runs in
/// `crates/compose-core/tests/cel_conformance.rs`; this is the other half.
/// The Rust side already runs in `crates/compose-core/tests/cel_conformance.rs`;
/// this is the other half, over the **same files**, run against the evaluator a
/// built project embeds (`src/cel.ts`). The driver is committed beside this suite
/// rather than written inline, because it carries a JSON reader that keeps `1`
/// and `1.0` apart — the distinction the corpus's int64 cases exist to pin, and
/// one `JSON.parse` throws away.
#[test]
fn the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus() {
    let built = harness::build("bounded-cycle", "local");
    built.succeeded();

    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("compose-core/tests/fixtures/cel-conformance");
    let driver = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/compiled_graph_acceptance/cel-conformance.mjs");

    let output = std::process::Command::new("node")
        .arg(&driver)
        .arg(built.root())
        .arg(&corpus)
        .output()
        .expect("node runs the generated evaluator");
    assert!(
        output.status.success(),
        "the conformance driver failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8_lossy(&output.stdout),
        "[]",
        "the two CEL implementations must not diverge (CLAUDE.md)"
    );
}

/// `examples/review-loop` — the documented project, compiled and **run**.
///
/// Every other test here runs a fixture, and a fixture is written for the test.
/// This one runs what a reader is shown: the same files `compose-core`'s golden
/// corpus is emitted from, so the bytes under test are the committed golden's.
/// Two things it carries that no fixture does:
///
/// * `agent.researcher` declares `tools:` **inside the cycle**, so every pass of
///   the loop is a tool-loop call followed by the pinned structured one — the
///   interaction between grammar 7.4's budget and grammar 5's intra-agent loop,
///   which the `bounded-cycle` fixture's tool-less agents cannot reach;
/// * it binds `model.default`, which is a `route:`. Failover is a later bullet
///   (PRD §7 M1), and codegen binds the route's **first member** statically with
///   a note saying so, which is what makes the example runnable today: the run
///   below reaches `model.smart`'s `claude-sonnet-4-6`, and the day failover
///   lands this is the test that says the binding still resolves.
///
/// **Why a preamble.** The example's `provider.anthropic` declares no
/// `base_url:` — it is written to talk to Anthropic, which is the point of an
/// example — so the harness redirects egress in the host module the driver
/// imports before the graph. Nothing about the composition or the emitted
/// project differs; a fixture parameterises its `base_url:` instead, which is
/// why the fixtures exist and why this is the one test that needs the preamble.
#[test]
fn the_review_loop_example_runs_its_cycle_against_the_mock_provider() {
    let provider = MockProvider::start().expect("a loopback port");

    // `revise`, `revise`, `approve` — three passes of the cycle, inside a
    // `max_iterations: 3` budget that therefore never runs out.
    //
    // Both agents bind `claude-sonnet-4-6` (the reviewer directly, the writer
    // through `model.default`'s first member), so one queue serves both and each
    // entry says which agent's prompt it answers. Within one agent the entries
    // are consumed in order, which is the alternation an agent *with* tools
    // makes: a loop call offering the tools and pinning nothing, answered with
    // text, then the pinned call answered with structured output.
    let writer = "a research writer";
    let reviewer = "meticulous technical reviewer";
    let mut scripts = Vec::new();
    for (pass, verdict) in [(1, "revise"), (2, "revise"), (3, "approve")] {
        scripts.push(Script::new(SONNET, Outcome::text("searching")).matching(writer));
        scripts.push(
            Script::new(
                SONNET,
                Outcome::structured(json!({ "draft": format!("# draft {pass}") })),
            )
            .matching(writer),
        );
        scripts.push(Script::new(SONNET, Outcome::text("reading")).matching(reviewer));
        scripts.push(
            Script::new(
                SONNET,
                Outcome::structured(json!({
                    "verdict": verdict,
                    "feedback": if verdict == "approve" { "" } else { "tighten it" },
                })),
            )
            .matching(reviewer),
        );
    }
    provider.enqueue_all(scripts);

    let mut environment = harness::environment(&provider);
    for (name, value) in [
        ("ANTHROPIC_API_KEY", "mock-provider-key"),
        ("SEARCH_API_KEY", "mock-provider-key"),
        ("SEARCH_HOST", "search.invalid"),
    ] {
        environment.push((name.to_string(), value.to_string()));
    }

    let Some(run) = harness::invoke_entrypoint(
        &harness::example("review-loop"),
        "examples/review-loop",
        "flow.review_loop",
        &json!({ "goal": "explain the compiler" }),
        &environment,
        Some(REDIRECT_TO_MOCK),
    ) else {
        return;
    };
    run.succeeded();

    assert_eq!(
        run.visited(),
        ["write", "review", "write", "review", "write", "review"],
        "two `revise` passes over the back edge, then the `approve` that leaves \
         through the `else:` escape"
    );
    assert_eq!(
        run.outputs()["draft"],
        "# draft 3",
        "the flow returns the `draft` channel as it stood at quiescence \
         (grammar 7.5)"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        12,
        "six agent invocations, each a tool-loop call and a pinned one"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert!(
        recorded
            .iter()
            .all(|call| call.tools.contains(&"web_search".to_string())),
        "`tool.web_search` is attached to both agents and reaches every call: {:?}",
        recorded.iter().map(|call| &call.tools).collect::<Vec<_>>()
    );
    assert_eq!(
        recorded[0].structured_output, None,
        "a tool loop's first call pins nothing"
    );
    assert!(
        recorded[1].structured_output.is_some(),
        "…and the call that ends it pins the output schema (PRD 5.2)"
    );
    assert!(
        recorded.iter().all(|call| call.model == SONNET),
        "`model.default` binds its route's first member, `model.smart` (PRD §7 M1)"
    );

    // The back edge's budget was spent twice and never exhausted: what ended the
    // loop was the model's `approve`, not `max_iterations: 3`. The third pass
    // records no budget at all — a guard that came out false spends nothing —
    // which is what tells this run apart from `bounded-cycle`'s, where the
    // budget is what stops it.
    let review: Vec<Value> = run
        .entries("review")
        .iter()
        .map(|entry| entry["routing"]["edges"][0].clone())
        .collect();
    assert_eq!(
        review
            .iter()
            .map(|edge| edge["value"].clone())
            .collect::<Vec<_>>(),
        [json!(true), json!(true), json!(false)],
        "two `revise` verdicts, then the `approve`"
    );
    assert_eq!(
        review
            .iter()
            .map(|edge| edge["budget"]["used"].clone())
            .collect::<Vec<_>>(),
        [json!(1), json!(2), Value::Null],
    );
    assert_eq!(
        run.entries("review")[2]["routing"]["targets"],
        json!(["__end__"]),
        "the `else:` escape is what leaves the cycle on `approve` (grammar 7.4)"
    );
    assert!(provider.snapshot().is_drained(), "every script was served");
}

/// The host preamble that points `examples/review-loop` at the mock.
///
/// The example names no `base_url:`, so the emitted client resolves
/// `https://api.anthropic.com` — see the test above for why that is right and
/// why the redirect belongs here rather than in the composition. It rewrites the
/// host and nothing else: the request a compiled graph sends is still the one
/// the mock validates, over a real socket.
const REDIRECT_TO_MOCK: &str = r#"import process from "node:process";

const upstream = "https://api.anthropic.com";
const mock = process.env["MOCK_BASE_URL"].replace(/\/+$/, "");
const inner = globalThis.fetch;
globalThis.fetch = (resource, init) => {
  const url =
    typeof resource === "string"
      ? resource
      : resource instanceof URL
        ? resource.href
        : resource.url;
  return inner(url.startsWith(upstream) ? mock + url.slice(upstream.length) : url, init);
};
"#;

/// A bounded cycle stops at its budget and leaves through the escape edge, rather
/// than looping (PRD 5.4).
#[test]
fn a_bounded_cycle_leaves_through_its_escape_edge_when_the_budget_is_spent() {
    let provider = MockProvider::start().expect("a loopback port");
    // A model that never approves: only `max_iterations: 3` can end this run.
    //
    // Both nodes bind `model.smart`, so both draw from one queue and each entry
    // has to say which of the two calls it answers — an entry that narrows
    // nothing accepts everything and would shadow the one behind it. The
    // discriminator is each agent's own prompt, which reaches the request as the
    // system turn verbatim (D13).
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

    let Some(run) = harness::invoke(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    // One write, then three (write, review) passes on the back-edge budget: the
    // fourth `revise` finds the budget spent and the `else:` edge fires.
    let calls = provider.requests().len();
    assert_eq!(
        calls, 8,
        "four writer calls and four reviewer calls, then the escape — not a loop"
    );
    assert_eq!(run.outputs()["draft"], "a draft");
    assert_eq!(
        run.visited(),
        [
            "write", "review", "write", "review", "write", "review", "write", "review"
        ]
    );

    // The counter is the observable: it spends one unit per traversal, stops at
    // the declared budget, and the pass that finds it spent takes the escape
    // instead (grammar 7.4, Decision D19).
    let review = run.entries("review");
    let spent: Vec<Value> = review
        .iter()
        .map(|entry| entry["routing"]["edges"][0]["budget"]["used"].clone())
        .collect();
    assert_eq!(spent, [json!(1), json!(2), json!(3), json!(3)]);
    for (index, entry) in review.iter().enumerate() {
        let last = index == 3;
        assert_eq!(
            entry["routing"]["edges"][0]["value"],
            json!(true),
            "the model said `revise` on every pass, including the last"
        );
        assert_eq!(entry["routing"]["edges"][0]["taken"], json!(!last));
        assert_eq!(
            entry["routing"]["targets"],
            if last {
                json!(["__end__"])
            } else {
                json!(["write"])
            }
        );
    }
    assert_eq!(
        review[3]["routing"]["edges"][0]["reason"],
        "the `max_iterations` budget is spent"
    );
}

/// A homogeneous map dispatches one instance per item, bounded by
/// `max_concurrency`, and joins before the downstream edge fires.
#[test]
#[ignore = "pending: codegen must emit `map` as LangGraph `Send`"]
fn a_homogeneous_map_dispatches_one_instance_per_item() {
    let provider = MockProvider::start().expect("a loopback port");
    let tasks: Vec<Value> = (0..3)
        .map(|index| json!({ "title": format!("task-{index}"), "body": "do it" }))
        .collect();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "tasks": tasks })),
    ));
    for index in 0..3 {
        provider.enqueue(
            Script::new(
                HAIKU,
                Outcome::structured(json!({ "result": format!("done-{index}") })),
            )
            .matching(format!("task-{index}")),
        );
    }

    let run = harness::run("fanout", "flow.spread", &[("goal", "ship it")], &provider);
    run.succeeded();

    assert_eq!(
        run.outputs()["drafts"],
        json!(["done-0", "done-1", "done-2"]),
        "one instance per item, joined before the edge to `end` fired"
    );
    assert_eq!(
        provider.requests().len(),
        4,
        "the planner plus three workers"
    );
    assert!(provider.snapshot().is_drained());
}

/// A discriminator-routed map sends each item to its own route, narrowed to that
/// variant's payload (PRD 5.6).
#[test]
#[ignore = "pending: codegen must emit discriminator-routed `map` dispatch"]
fn a_routed_map_sends_each_variant_to_its_own_route() {
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

    let run = harness::run(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    );
    run.succeeded();

    let outputs = run.outputs();
    assert_eq!(outputs["drafts"], json!(["a patch"]));
    assert_eq!(outputs["tickets"], json!(["a ticket"]));

    let recorded = provider.requests();
    let fixer = recorded
        .iter()
        .find(|request| request.body_text.contains("rename it"))
        .expect("the auto_fixable item reached its own route");
    assert!(
        !fixer.body_text.contains("severity"),
        "each route is narrowed to its variant's payload, not a common item type"
    );
    assert!(provider.snapshot().is_drained());
}

/// Appended results are ordered by source-item index, whatever order the
/// instances complete in (PRD 5.6's replay guarantee).
#[test]
#[ignore = "pending: codegen must emit index-tagged reducers"]
fn appended_results_are_ordered_by_source_item_index() {
    let provider = MockProvider::start().expect("a loopback port");
    let tasks: Vec<Value> = (0..3)
        .map(|index| json!({ "title": format!("task-{index}"), "body": "do it" }))
        .collect();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "tasks": tasks })),
    ));
    // The first item answers last: completion order is deliberately the reverse
    // of source order, which is the only way this assertion means anything.
    for (index, delay) in [(0, 300), (1, 150), (2, 0)] {
        provider.enqueue(
            Script::new(
                HAIKU,
                Outcome::structured(json!({ "result": format!("done-{index}") }))
                    .after(Duration::from_millis(delay)),
            )
            .matching(format!("task-{index}")),
        );
    }

    let run = harness::run("fanout", "flow.spread", &[("goal", "ship it")], &provider);
    run.succeeded();

    assert_eq!(
        run.outputs()["drafts"],
        json!(["done-0", "done-1", "done-2"]),
        "appended values are reordered by source-item index before the join"
    );
}

/// A node retries its model call per its declared policy, and the retries are
/// visible as repeated calls.
#[test]
fn a_node_retries_its_model_call_per_its_declared_policy() {
    let provider = MockProvider::start().expect("a loopback port");
    // `write` declares `retry: { max: 2, backoff: 1s }`: one attempt plus two
    // retries, and the third answer is the one that lands.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::server_error())
            .times(2)
            .matching("research writer"),
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" })))
            .matching("research writer"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        )
        .matching("meticulous technical reviewer"),
    ]);

    let Some(run) = harness::invoke(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    assert_eq!(
        provider.requests().len(),
        4,
        "three attempts at the writer, then the reviewer"
    );
    assert_eq!(run.outputs()["draft"], "a draft");
    // `write` declares `retry: { max: 2, … }` — one attempt plus two retries —
    // and the node it feeds took one (grammar 9.1). The trace counts them, so a
    // retry that silently stopped happening is visible as a number rather than as
    // a transcript length.
    assert_eq!(run.entries("write")[0]["attempts"], json!(3));
    assert_eq!(run.entries("review")[0]["attempts"], json!(1));
}

/// A node timeout fires on a provider that never answers, and the node's error
/// policy takes over from there.
#[test]
fn a_node_timeout_fires_and_its_error_policy_takes_over() {
    let provider = MockProvider::start().expect("a loopback port");
    // `review` declares `timeout: 10s`; the scripted provider never answers, and
    // the project's `defaults: { on_error: fail }` is what the run reports.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" })))
            .matching("research writer"),
        Script::new(SONNET, Outcome::timeout(Duration::from_secs(30)))
            .matching("meticulous technical reviewer"),
    ]);

    let Some(run) = harness::invoke(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("review"),
        "the failure names the node whose budget ran out: {failure}"
    );
    assert!(
        failure.contains("timeout") || failure.contains("timed out"),
        "…and what happened to it: {failure}"
    );
}

/// The same budget, spent on a child process rather than on a provider — and the
/// `on_error: { fallback: … }` that takes over when it does.
///
/// Three things this decides that the model-side twin above cannot. The fallback
/// **replaces** the node's own edges rather than running beside them (grammar
/// 9.2, Decision D21), so the target is what runs next and `slow`'s own
/// `to: end` is not evaluated; the budget is the *node's* rather than the
/// provider client's, which is why an `exec:` node with no network in it times
/// out at all; and the budget is spent across attempts rather than per attempt
/// (grammar 9.1), which the trace's `attempts` is what makes observable.
#[test]
fn a_node_timeout_fires_over_a_child_process_and_its_fallback_takes_over() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke("activities", "flow.deadline", &[], &provider) else {
        return;
    };
    run.succeeded();

    assert_eq!(run.outputs()["report"], "rescued");
    assert_eq!(run.visited(), ["slow", "rescue"]);
    let slow = run.entries("slow");
    assert_eq!(slow[0]["outcome"], "failed");
    assert_eq!(slow[0]["fallback"], "rescue");
    assert!(
        slow[0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("timed out")),
        "the trace records what happened to it: {}",
        slow[0]["error"]
    );
    assert!(
        slow[0]["routing"].is_null(),
        "a fallback is taken *instead of* the node's own edges: {}",
        slow[0]
    );
    // `slow` declares `retry: { max: 3 }` — four attempts allowed — and made
    // one: the first spent the whole 300ms budget, and grammar 9.1 spends that
    // budget across attempts rather than resetting it per attempt. A trace that
    // reported 4 here would be describing three sleeps that never ran.
    assert_eq!(slow[0]["attempts"], json!(1));
}

/// A store-op node reads and writes the local backend, with no infrastructure
/// (PRD 5.8's zero-infra guarantee under `--target local`).
///
/// The **write** is what needs care to observe. `store.prefs` is
/// `scope: execution`, so nothing after this run can see what `remember` did,
/// and an `op: set` that silently did nothing would be invisible to any
/// assertion made outside the execution. So the fixture reads the key back
/// inside the same flow (`reread`) and materializes both halves of that read as
/// flow outputs — the miss before the write and the hit after it.
#[test]
#[ignore = "pending: codegen must emit store-op nodes over the SQLite/local-disk backends"]
fn a_store_op_node_reads_and_writes_the_local_backend() {
    let provider = MockProvider::start().expect("a loopback port");
    // `agent.grounded` has stores attached, so its tools open a loop: the first
    // call offers them and pins nothing, and the pinned output call is what ends
    // it (see *How many model calls an agent node makes*). The model here
    // searches nothing and answers.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    let run = harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    );
    run.succeeded();
    let outputs = run.outputs();
    assert_eq!(outputs["answer"], "an answer");

    // The round trip: `load` missed, `remember` wrote, `reread` found it. Both
    // channels declare a default the write never produces (`false`, and a
    // `theme` of `nothing was written`), so a stubbed `set` cannot pass this.
    assert_eq!(
        outputs["prefs_found_after"],
        json!(true),
        "the second `get` found what `set` wrote; its channel's default is `false`"
    );
    assert_eq!(outputs["recorded_prefs"]["theme"], "default");
    assert_eq!(
        outputs["recorded_prefs"]["verbosity"], "low",
        "the value read back is the one `remember` wrote, field for field"
    );

    // `load` ran before the agent and found nothing — the store is
    // execution-scoped, so every run starts empty. What the agent saw is that
    // read's answer, in the first call it made.
    let call = &provider.requests()[0];
    let turn = call.body()["messages"][0]["content"]
        .as_str()
        .expect("the bound input");
    assert!(
        turn.contains("false"),
        "the `found: false` of a miss reached the agent's input: {turn}"
    );
}

/// An attached store synthesizes its LLM-facing tools into the model request
/// (grammar 11.5).
///
/// Read off the **first** call, which is a loop call: attached store tools are
/// tools, so they open the same loop an agent's `tools:` list does, and the
/// pinned output call that ends it is a different request.
#[test]
#[ignore = "pending: codegen must synthesize store tools"]
fn an_attached_store_synthesizes_its_tool_surface_in_the_model_request() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    )
    .succeeded();

    let call = &provider.requests()[0];
    assert!(call.is_valid(), "{:?}", call.failures());
    assert!(
        call.tools.contains(&"docs_search".to_string()),
        "a vector store synthesizes `<name>_search`: {:?}",
        call.tools
    );
    assert!(
        call.tools.contains(&"prefs_get".to_string())
            && call.tools.contains(&"prefs_set".to_string()),
        "a kv store under the `read_write` default synthesizes both: {:?}",
        call.tools
    );
}

/// `agent_access: read` withholds the write tool — the least-privilege knob
/// Decision D37 adds to a store attachment.
///
/// D37 is **PRD §10's first open question**: the grammar accepts the key (M0),
/// and whether the PRD keeps it is not settled, so this test is written against
/// the fixture as it stands rather than against a ratification. If §10.1 is
/// declined, the key leaves the `stores` fixture and this test goes with it;
/// what survives either way is the criterion the inventory names, which is that
/// an attached store synthesizes a tool surface at all.
#[test]
#[ignore = "pending: codegen must synthesize store tools"]
fn agent_access_read_withholds_the_write_tool() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    )
    .succeeded();

    let call = &provider.requests()[0];
    assert!(
        !call.tools.contains(&"docs_upsert".to_string()),
        "`store.docs` is `agent_access: read`, so the write tool is not on offer: {:?}",
        call.tools
    );
    assert!(
        call.tools.contains(&"docs_search".to_string()),
        "…while the read tool it does grant is: {:?}",
        call.tools
    );
}

/// A route fails over to its next member on a declared condition, and the trace
/// records that it did (PRD 5.9).
#[test]
#[ignore = "pending: codegen must emit model routing with trace-recorded failover"]
fn a_route_fails_over_to_its_next_member_and_the_trace_records_it() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "42" }))),
    ]);

    let run = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    );
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "42");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.model.as_str())
            .collect::<Vec<_>>(),
        [SONNET, HAIKU],
        "the route was tried in order"
    );
    let trace = run.stderr();
    assert!(
        trace.contains("model.fast"),
        "failover is trace data, not silent behaviour (PRD 5.9): {trace}"
    );
}

/// A failure condition outside `route_on:` fails the node instead of failing
/// over — otherwise the declaration would mean nothing.
#[test]
#[ignore = "pending: codegen must emit model routing with trace-recorded failover"]
fn a_condition_outside_route_on_fails_the_node_instead_of_failing_over() {
    let provider = MockProvider::start().expect("a loopback port");
    // `model.default` routes on rate_limit, overloaded, and timeout — not on
    // server_error.
    provider.enqueue(Script::new(SONNET, Outcome::server_error()));

    let run = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    );
    run.failed();

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        1,
        "the fallback was never tried: {:?}",
        recorded.iter().map(|call| &call.model).collect::<Vec<_>>()
    );
    assert_eq!(recorded[0].model, SONNET);
}

/// A missing env ref fails at process start, naming the variable — never at the
/// first model call, and never with a key baked into the generated code
/// (PRD 5.9).
///
/// The emitted `src/env.ts` already does this, and
/// `compose-core`'s `the_generated_project_checks_its_environment_when_it_is_loaded`
/// decides it on every `cargo test` by loading a built project with the
/// variables removed. What this one adds is the *run*: that the check is what a
/// started execution hits, before the first model call — which needs a command
/// that starts one.
#[test]
fn a_missing_env_ref_fails_at_process_start_naming_the_variable() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(SONNET, Outcome::text("never reached")));

    let environment: Vec<(String, String)> = harness::environment(&provider)
        .into_iter()
        .filter(|(name, _)| name != harness::API_KEY)
        .collect();
    let Some(run) = harness::invoke_with(
        "agent-anthropic",
        "flow.review",
        &json!({ "goal": "ship it", "draft": "a draft" }),
        &environment,
    ) else {
        return;
    };
    let failure = run.failed();

    assert!(
        failure.contains(harness::API_KEY),
        "the failure names the missing variable: {failure}"
    );
    assert!(
        provider.requests().is_empty(),
        "it failed at start, before any model call"
    );
}

// ---------------------------------------------------------------------------
// PRD §7 M1, bullet 2 — build, run, serve, golden files.
// `build` has landed, goldens with it; `run` and `serve` are pending.
// ---------------------------------------------------------------------------

/// `agent-compose build` writes a TypeScript project for the selected target.
#[test]
fn build_writes_a_typescript_project_for_the_target() {
    for name in harness::FIXTURES {
        let built = harness::build(name, "local");
        built.succeeded();
        let files = built.files();
        assert!(
            files.iter().any(|file| file.ends_with(".ts")),
            "`{name}` produced no TypeScript: {files:?}"
        );
        assert!(
            files.iter().any(|file| file == "package.json"),
            "`{name}` produced no package manifest, so nothing can run it: {files:?}"
        );
        let generated = built.read(
            files
                .iter()
                .find(|file| file.ends_with(".ts"))
                .expect("a TypeScript file"),
        );
        assert!(
            generated.contains("generated by agent-compose"),
            "generated code carries a header saying so (PRD §8: hand-edited \
             generated code forks the source of truth)"
        );
    }
}

/// Same DSL in, byte-identical TypeScript out (PRD 5.12).
///
/// This is the machine-checkable half of "golden-file codegen tests". The other
/// half — goldens committed to the repository and reviewed in PRs like any other
/// code (CLAUDE.md) — lives in `compose-core`'s
/// `tests/generated_project_goldens.rs`, where the emitted bytes for both worked
/// examples are committed under `tests/goldens/`. What makes those goldens *mean*
/// anything is the determinism asserted here, because a regeneration diff is only
/// signal if identical input regenerates identically.
#[test]
fn build_is_byte_identical_for_byte_identical_input() {
    for name in harness::FIXTURES {
        let first = harness::build(name, "local");
        let second = harness::build(name, "local");
        first.succeeded();
        second.succeeded();
        assert_eq!(
            first.files(),
            second.files(),
            "`{name}` emitted a different file set on the second build"
        );
        for file in first.files() {
            assert_eq!(
                first.read(&file),
                second.read(&file),
                "`{name}`'s `{file}` differs between two builds of the same input; \
                 codegen must be deterministic or regeneration diffs are noise"
            );
        }
    }
}

/// `agent-compose run` executes a manual trigger and prints the flow's outputs.
#[test]
#[ignore = "pending: `agent-compose run` must exist"]
fn run_executes_a_manual_trigger_and_prints_the_flow_outputs() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    // No `triggers:` section declares this flow: the implicit manual entry
    // exists for every flow (D64), which is what `run` invokes.
    let run = harness::run(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    );
    run.succeeded();
    assert_eq!(run.outputs()["verdict"], "approve");

    // An input the schema refuses is refused at run start, naming the field
    // (grammar 13.2) — the same check a declared trigger's bindings get at
    // compile time.
    let bad = harness::run(
        "agent-anthropic",
        "flow.review",
        &[("goal", ""), ("draft", "a draft")],
        &provider,
    );
    let failure = bad.failed();
    assert!(failure.contains("goal"), "{failure}");
}

/// `agent-compose serve` exposes start and status for an `http` trigger.
///
/// Two of the criterion's three verbs, over the fixture's interrupt-free flow
/// (`flow.direct`): a start that answers with an execution id and a status route
/// that reports the run's terminal state and outputs. Split from the resume half
/// below because this half is decidable with `serve` alone — the criterion is
/// PRD §7 M1's, while the `human` node runtime the other half needs is scheduled
/// for M2 (PRD §9, resolved question 4).
#[test]
#[ignore = "pending: `agent-compose serve` must exist"]
fn serve_exposes_start_and_status_for_an_http_trigger() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let served = harness::serve("http-trigger", &provider);
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // Start: `respond: async` answers with an execution id immediately.
    let started = app
        .post_json("/direct-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // Status: nothing interrupts this flow, so it reaches its outputs on its own
    // and the status route is how the caller reads them.
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["outputs"]["answer"], "an answer");

    // The mirror of the resume rule below — a resume payload is validated
    // against the `human` node's output schema (PRD 5.11), and a start payload
    // against the flow's declared `inputs:`. `question` is `min_length: 1`, so
    // an empty one is a 400 at the route and no execution at all.
    let refused = app
        .post_json("/direct-answers", &json!({ "question": "" }))
        .expect("the trigger's route answers");
    assert_eq!(
        refused.status, 400,
        "an empty question is not the flow's declared input"
    );

    assert!(provider.snapshot().is_drained(), "the run used its script");
}

/// The third verb: `resume` against the interrupting `human` node's schema.
///
/// Kept apart from start/status because it cannot be decided without the `human`
/// node runtime, which PRD §9's resolved question 4 puts in M2 — so this is the
/// one row of the M1 inventory whose blocker is not `serve` itself.
#[test]
#[ignore = "pending: `agent-compose serve` must exist, and the `human` node runtime with it"]
fn serve_resumes_an_interrupted_execution_against_the_human_nodes_schema() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let served = harness::serve("http-trigger", &provider);
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // Start: `respond: async` answers with an execution id immediately.
    let started = app
        .post_json("/answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // Status: the execution is interrupted at the `human` node.
    let status = harness::settled(&app, &execution);
    assert_eq!(status["status"], "interrupted");

    // Resume: the payload is validated against the `human` node's output schema.
    let refused = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "maybe" }),
        )
        .expect("the resume route answers");
    assert_eq!(
        refused.status, 400,
        "`maybe` is not one of the declared variants"
    );

    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "approve", "note": "looks right" }),
        )
        .expect("the resume route answers");
    assert!(resumed.status == 200 || resumed.status == 202);

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["outputs"]["answer"], "an answer");
    assert_eq!(finished["outputs"]["decision"], "approve");
}

/// Every generated project type-checks and constructs its graph under the pinned
/// LangGraph version (CLAUDE.md, *Generated-code checks*).
///
/// Two things about this test changed when `build` landed, and both are
/// interface assumptions the harness header says a codegen PR may fix here:
///
/// * the module is `src/graph.ts`, not `./graph.js`. The emitted project has **no
///   build step** — Node has stripped types natively since 22.18, so the
///   TypeScript in `src/` is what runs — and a `.js` at the root would have had
///   to come from a `tsc` emit `--noEmit` never performs. See the generated
///   `README.md`, which documents the layout.
/// * the reason names what is actually missing. `build` exists, the pinned
///   toolchain exists, and `compose-core`'s `tests/generated_code_gates.rs`
///   already runs *this* pair of checks — `tsc --noEmit` and construction under
///   the pinned LangGraph — over the committed golden corpus on every `cargo
///   test`. What is left is the subject: `src/graph.ts` builds no topology yet,
///   so "constructs its graph" is not a claim this milestone can make until the
///   flows are assembled into it.
#[test]
fn every_generated_project_type_checks_and_constructs_its_graph() {
    for name in harness::FIXTURES {
        let Some((project, built)) = harness::build_under_toolchain(name, "gate") else {
            return;
        };
        assert!(
            built.status.success(),
            "`{name}` did not build:\n{}",
            String::from_utf8_lossy(&built.stderr)
        );

        let typecheck = std::process::Command::new(
            project
                .parent()
                .and_then(Path::parent)
                .expect("the toolchain root")
                .join("node_modules/.bin/tsc"),
        )
        .args(["--noEmit", "-p", "."])
        .current_dir(&project)
        .output()
        .expect("tsc runs");
        assert!(
            typecheck.status.success(),
            "`{name}` does not type-check:\n{}",
            String::from_utf8_lossy(&typecheck.stdout)
        );

        // Constructing every flow's graph is a stronger check than compiling it:
        // an edge to a node that is not registered, or a node reachable only
        // through a control-transfer position that `ends` did not declare, is a
        // runtime error at construction and a green `tsc` either way.
        let construct = std::process::Command::new("node")
            .args([
                "--input-type=module",
                "-e",
                "const graph = await import('./src/graph.ts');\
                 graph.createBuilder();\
                 if (Object.keys(graph.flows).length === 0) process.exit(0);\
                 for (const flow of Object.values(graph.flows)) {\
                   if (typeof flow.invoke !== 'function') throw new Error(`${flow.address} has no graph`);\
                 }",
            ])
            .current_dir(&project)
            .output()
            .expect("node runs");
        assert!(
            construct.status.success(),
            "`{name}`'s graph does not construct: {}",
            String::from_utf8_lossy(&construct.stderr)
        );
    }
}

// ---------------------------------------------------------------------------
// Request shapes the live tests send, written the way codegen will write them.
// ---------------------------------------------------------------------------

/// The request `agent.reviewer` of the `agent-anthropic` fixture produces.
fn anthropic_agent_request() -> Value {
    json!({
        "model": SONNET,
        "max_tokens": 4096,
        "system": "You are a meticulous technical reviewer.",
        "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\",\"draft\":\"a draft\"}" }],
        "tools": [{
            "name": "reviewer_output",
            "description": "The structured output agent.reviewer must produce.",
            "input_schema": {
                "type": "object",
                "properties": {
                    "verdict": { "type": "string", "enum": ["approve", "revise"] },
                    "feedback": { "type": "string" },
                },
                "required": ["verdict", "feedback"],
            },
        }],
        "tool_choice": { "type": "tool", "name": "reviewer_output" },
    })
}

/// The same request on the Chat Completions surface, as the `agent-openai`
/// fixture's `openai_compatible` provider reaches it.
fn openai_agent_request() -> Value {
    json!({
        "model": LOCAL,
        "messages": [
            { "role": "system", "content": "You are a meticulous technical reviewer." },
            { "role": "user", "content": "{\"goal\":\"ship it\",\"draft\":\"a draft\"}" },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": {
                "name": "reviewer_output",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": {
                        "verdict": { "type": "string", "enum": ["approve", "revise"] },
                        "feedback": { "type": "string" },
                    },
                    "required": ["verdict", "feedback"],
                    "additionalProperties": false,
                },
            },
        },
    })
}

/// A structured-output request for one model, under one output-schema name.
fn structured_request(model: &str, output: &str) -> Value {
    json!({
        "model": model,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": "{\"question\":\"what is it?\"}" }],
        "tools": [{
            "name": output,
            "input_schema": { "type": "object", "properties": { "answer": { "type": "string" } } },
        }],
        "tool_choice": { "type": "tool", "name": output },
    })
}

/// One dispatched instance's request, carrying its item.
fn item_request(item: &str) -> Value {
    json!({
        "model": HAIKU,
        "max_tokens": 2000,
        "messages": [{ "role": "user", "content": format!("{{\"title\":\"{item}\"}}") }],
        "tools": [{
            "name": "worker_output",
            "input_schema": { "type": "object", "properties": { "result": { "type": "string" } } },
        }],
        "tool_choice": { "type": "tool", "name": "worker_output" },
    })
}
