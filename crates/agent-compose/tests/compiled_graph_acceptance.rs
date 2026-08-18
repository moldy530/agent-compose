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
/// The `provider-kinds` fixture's two models: one per Chat Completions kind that
/// is neither `openai_compatible` nor already bound elsewhere.
const AZURE_HOSTED: &str = "gpt-4o-mini";
const OPENAI_DIRECT: &str = "gpt-4o";

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
            "notes: Annotation<z.infer<typeof stateNotes>, runtime.Written<z.infer<typeof stateNotes>[number]>>({"
        ),
        "an `append` channel takes one element per write — or a `map` node's whole \
         ordered batch of them (grammar 7.6.4 clause 2):\n{state}"
    );
    assert!(
        state.contains("reducer: (left, right) => runtime.appendReduce(left, right),"),
        "and appends in write order:\n{state}"
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
/// Zod and the JSON Schema the same lowering produces. An agent node fn parses
/// its answer with the emitted schema, which is what turns that refusal into a
/// run failure — reached here through the fixture's first node, a `flow:`
/// instantiation of `flow.normalize`, so the subgraph is on the path too.
#[test]
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

    let Some(run) = harness::invoke(
        "fanout",
        "flow.triage",
        &[("report", "a report")],
        &provider,
    ) else {
        return;
    };
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

/// What an `optional:` property costs on this surface, and what it does not.
///
/// OpenAI's structured-output decoder closes a schema only when every object in
/// it lists every property in `required`, so an agent whose output nests an
/// object with `optional:` is sent `strict: false` — the model is *not*
/// constrained by the schema its answer is then parsed with, which is the one
/// property PRD 9.16 calls load-bearing. `codegen::runtime`'s
/// `strict-is-refused-by-an-optional-property` row is where that is signed off
/// on, with the three alternatives and why each is worse; this is the test the
/// row names, and it pins both halves of it.
///
/// The second half is what keeps the degradation bounded. An unconstrained model
/// can answer something the schema does not admit — here a `meta` carrying a
/// property nothing declared, which a closed decoder could not have produced —
/// and the emitted Zod is what refuses it. The contract moves from the decoder
/// to the parse; it does not stop being enforced.
#[test]
fn a_nested_optional_property_costs_the_strict_decoder_and_not_the_parse() {
    let provider = MockProvider::start().expect("a loopback port");
    // `owner` omitted — the property whose being `optional:` is what costs the
    // strict decoder, answered the way the author said it may be.
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::structured(json!({ "verdict": "revise", "meta": { "severity": "high" } })),
    ));

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.triage",
        &[("report", "the build is red")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["meta"], json!({ "severity": "high" }));

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 1);
    let call = &recorded[0];
    assert!(call.is_valid(), "{:?}", call.failures());
    let Some(StructuredOutput::JsonSchema { schema, strict, .. }) = call.structured_output.as_ref()
    else {
        panic!(
            "this surface asks with `response_format` (WIRE-NOTES (3)): {:?}",
            call.structured_output
        );
    };
    assert_eq!(
        schema["properties"]["meta"]["required"],
        json!(["severity"]),
        "the schema on the wire is the whole one, `optional:` included"
    );
    assert!(
        !*strict,
        "…and a schema with a property outside `required` is one OpenAI's decoder \
         cannot be asked to close (`strict-is-refused-by-an-optional-property`)"
    );

    // The other half of the row. The model was unconstrained, so it can answer
    // what the schema does not admit — and the emitted Zod is what refuses it,
    // before any edge is evaluated (PRD 5.2).
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::structured(json!({
            "verdict": "revise",
            "meta": { "severity": "high", "assignee": "nobody" },
        })),
    ));
    let unconstrained = harness::invoke(
        "agent-openai",
        "flow.triage",
        &[("report", "the build is red")],
        &provider,
    )
    .expect("the toolchain was there a moment ago");
    let failure = unconstrained.failed();
    assert!(
        failure.contains("triage"),
        "the node whose answer did not parse is named: {failure}"
    );
    assert!(
        failure.contains("assignee"),
        "…and so is the property the schema does not declare: {failure}"
    );
}

/// The two Chat Completions kinds the fixture above does not reach, each
/// authenticating and routing the way grammar 12.1's row for it says.
///
/// `openai_compatible` is one of three kinds that speak this surface, and the
/// other two differ from it *only* in the request — which is precisely the
/// difference an outputs assertion cannot see, and precisely what codegen can
/// get wrong. `azure_openai` sends the credential as `api-key:` (a request that
/// authenticates the direct way is answered 401 by the mock, as
/// `an_azure_request_without_a_subscription_key_is_refused` pins), reaches
/// `/openai/v1/chat/completions`, and carries `api_version:` in the query;
/// `kind: openai` sends a bearer token and the `organization:` of its own row as
/// `openai-organization:`. Four runtime branches, emitted-but-unrun until here.
///
/// This is the same argument
/// `an_agent_node_sends_its_prompt_input_and_output_schema_on_chat_completions`
/// makes for itself, one level down: a kind that never runs a compiled graph is
/// a kind whose regression ships green.
#[test]
fn each_chat_completions_kind_authenticates_and_routes_the_way_its_row_says() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            AZURE_HOSTED,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ),
        Script::new(
            OPENAI_DIRECT,
            Outcome::structured(json!({ "summary": "the reviewer asked for one change" })),
        ),
    ]);

    let Some(azure) = harness::invoke(
        "provider-kinds",
        "flow.azure",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    azure.succeeded();
    assert_eq!(azure.outputs()["verdict"], "revise");

    let direct = harness::invoke(
        "provider-kinds",
        "flow.direct",
        &[("notes", "tighten it")],
        &provider,
    )
    .expect("the toolchain was there a moment ago");
    direct.succeeded();
    assert_eq!(
        direct.outputs()["summary"],
        "the reviewer asked for one change"
    );

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2, "one agent node each");

    let hosted = &recorded[0];
    assert!(hosted.is_valid(), "{:?}", hosted.failures());
    assert_eq!(hosted.surface, Surface::AzureOpenAi);
    assert_eq!(hosted.model, AZURE_HOSTED);
    assert_eq!(
        hosted.path, "/openai/v1/chat/completions",
        "`azure_openai` has its own route (WIRE-NOTES §7)"
    );
    assert_eq!(
        hosted.query, "api-version=2024-10-21",
        "…and its `api_version:` rides the query string"
    );
    assert_eq!(
        hosted.headers["api-key"], "mock-provider-key",
        "…and the credential is a subscription key, not a bearer token"
    );
    assert!(
        !hosted.headers.contains_key("authorization"),
        "the direct spelling is not sent beside it: {:?}",
        hosted.headers
    );

    let plain = &recorded[1];
    assert!(plain.is_valid(), "{:?}", plain.failures());
    assert_eq!(plain.surface, Surface::OpenAi);
    assert_eq!(plain.model, OPENAI_DIRECT);
    assert_eq!(plain.path, "/v1/chat/completions");
    assert_eq!(plain.query, "", "`api-version` belongs to Azure alone");
    assert_eq!(plain.headers["authorization"], "Bearer mock-provider-key");
    assert_eq!(
        plain.headers["openai-organization"], "org-acceptance",
        "the optional `organization:` of grammar 12.1's `openai` row"
    );
    assert!(
        !plain.headers.contains_key("api-key"),
        "…and not the Azure spelling: {:?}",
        plain.headers
    );
    assert!(provider.snapshot().is_drained());
}

/// A model that declines to answer says why, and the node error repeats it.
///
/// Chat Completions states a refusal as `content: null` beside a `refusal`
/// string (`WIRE-NOTES.md` (3)). Every branch of the reader sees the same thing
/// as an answer cut short by `max_tokens` — no structured output — so a node
/// error built from the absence alone is true and useless: it cannot tell an
/// author whose model refused from one whose budget ran out, and those have
/// opposite fixes.
///
/// The refusal is served with [`Outcome::raw`] because it is exactly what the
/// scripted shapes will not compose: a reply the *generated code* must reject.
#[test]
fn a_refused_answer_carries_the_reason_the_model_gave() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::raw(
            200,
            json!({
                "id": "chatcmpl_refusal",
                "object": "chat.completion",
                "created": 1_735_689_600,
                "model": LOCAL,
                "choices": [{
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": null,
                        "refusal": "I can't help with that request.",
                    },
                    "finish_reason": "stop",
                }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 8, "total_tokens": 20 },
            }),
        ),
    ));

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("I can't help with that request."),
        "the node error repeats the reason the surface stated: {failure}"
    );
    assert!(
        failure.contains("agent.reviewer"),
        "…and names the agent that asked: {failure}"
    );
    assert_eq!(
        provider.requests().len(),
        1,
        "an agent with no tools makes exactly one call, and a refusal ends it"
    );
}

/// The conversation crosses from one agent node to the next: what the first was
/// asked, what it answered, then what the second was asked (grammar 10.4, PRD
/// 5.7 tier 3).
///
/// The implicit `messages` channel is state no composition declares and every
/// agent node writes, so the only place it is visible is the wire: the second
/// node's request is the first node's exchange plus its own turn. Strict
/// alternation is the property that makes it *sendable* — an assistant turn is
/// followed by a user turn, never by another assistant turn — and it is why the
/// intra-agent tool loop's own turns stay out of the channel: an assistant turn
/// carrying `tool_use` blocks is well formed only when the very next turn
/// answers every one of them, which a later node's input never does.
#[test]
fn an_agent_nodes_exchange_reaches_the_next_agent_nodes_request() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "tightened" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.pair",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert_eq!(
        recorded[0].body()["messages"].as_array().map(Vec::len),
        Some(1),
        "the first node opens the conversation: {}",
        recorded[0].body()["messages"]
    );

    let messages = recorded[1].body()["messages"].clone();
    assert_eq!(
        messages.as_array().map(Vec::len),
        Some(3),
        "the second node is called with the first node's exchange and its own \
         turn, and with nothing else: {messages}"
    );
    assert_eq!(messages[0]["role"], "user");
    assert_eq!(
        messages[0]["content"], "{\"goal\":\"ship it\",\"draft\":\"a draft\"}",
        "what the first node was asked, verbatim"
    );
    assert_eq!(messages[1]["role"], "assistant");
    assert_eq!(
        messages[1]["content"][0]["type"], "text",
        "and what it answered, as one text block: {}",
        messages[1]
    );
    let answered: Value = messages[1]["content"][0]["text"]
        .as_str()
        .and_then(|text| serde_json::from_str(text).ok())
        .unwrap_or_else(|| panic!("the assistant turn carries its answer: {}", messages[1]));
    assert_eq!(
        answered,
        json!({ "verdict": "revise", "feedback": "tighten it" }),
        "the structured answer is what crosses, not the tool call that carried \
         it: {}",
        messages[1]
    );
    assert_eq!(messages[2]["role"], "user");
    assert_eq!(
        messages[2]["content"], "{\"goal\":\"ship it\",\"draft\":\"tighten it\"}",
        "then the second node's own input, which read the channel the first \
         wrote: {messages}"
    );
    assert_eq!(run.outputs()["verdict"], "approve");
    assert!(provider.snapshot().is_drained());
}

/// A loop answer is replayed as the model sent it, and an answer that carried
/// nothing stops the node instead of becoming a turn no provider accepts.
///
/// Both halves are about the same line of the tool loop — the assistant turn it
/// pushes before deciding what to do next — and both are only observable one
/// call later, in the request that carries the replayed turn:
///
///   * a `thinking` block is content the Messages API sends whenever a model
///     declares `settings: { thinking: … }` and requires back **unaltered**
///     beside the `tool_use` blocks it preceded, so a loop that rebuilt the turn
///     from the text and the tool calls it read would drop it;
///   * an answer cut short at `max_tokens` can carry no content at all, and the
///     turn built from one is `{"role": "assistant", "content": []}` — which the
///     Messages API refuses, and which `crates/mock-provider` refuses in its
///     place here, so sending it would report the model's empty answer as a
///     provider 400 about the wrong request.
///
/// `Outcome::raw` is what scripts both: they are answers a *provider* sends and
/// the scripted reply shapes are the ones a test asks for by name, so the body
/// is written out here rather than derived.
#[test]
fn a_loop_answer_is_replayed_verbatim_and_an_empty_one_stops_the_node() {
    let thinking = json!({
        "type": "thinking",
        "thinking": "The goal needs one fact.",
        "signature": "c2lnbmF0dXJl",
    });
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::raw(
                200,
                json!({
                    "id": "msg_thinking",
                    "type": "message",
                    "role": "assistant",
                    "model": SONNET,
                    "content": [
                        thinking,
                        {
                            "type": "tool_use",
                            "id": "toolu_lookup",
                            "name": "lookup",
                            "input": { "query": "a fact" },
                        },
                    ],
                    "stop_reason": "tool_use",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 12, "output_tokens": 34 },
                }),
            ),
        ),
        Script::new(SONNET, Outcome::text("found it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 3, "two loop calls, then the pinned one");
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "every request is one the Messages API would accept: {:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );
    let replayed = recorded[1].body()["messages"][1].clone();
    assert_eq!(replayed["role"], "assistant");
    assert_eq!(
        replayed["content"][0], thinking,
        "the model's own blocks go back unaltered, thinking first: {replayed}"
    );
    assert_eq!(
        replayed["content"][1]["type"], "tool_use",
        "…with the call it ended on: {replayed}"
    );
    assert_eq!(
        replayed["content"].as_array().map(Vec::len),
        Some(2),
        "…and nothing invented beside them: {replayed}"
    );

    // The same node, answered with nothing at all.
    let empty = MockProvider::start().expect("a loopback port");
    empty.enqueue(Script::new(
        SONNET,
        Outcome::raw(
            200,
            json!({
                "id": "msg_empty",
                "type": "message",
                "role": "assistant",
                "model": SONNET,
                "content": [],
                "stop_reason": "max_tokens",
                "stop_sequence": null,
                "usage": { "input_tokens": 12, "output_tokens": 4096 },
            }),
        ),
    ));

    let cut = harness::invoke(
        "agent-anthropic",
        "flow.research",
        &[("goal", "ship it")],
        &empty,
    )
    .expect("the toolchain was there a moment ago");
    let failure = cut.failed();
    assert!(
        failure.contains("no content") && failure.contains("max_tokens"),
        "the node fails about the answer it got, naming why it was empty: \
         {failure}"
    );
    assert_eq!(
        empty.requests().len(),
        1,
        "and no second request was sent: a turn with nothing in it never \
         reached the wire"
    );
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
/// The fixture makes every other order disagree. `alpha` sorts **first** by node
/// id, is declared **last** among the flow's nodes *and* last among the fork's
/// edges, and finishes **last** (it sleeps 300ms); `zulu` sorts last, is
/// declared first in both positions, and finishes first. Only clause 1 puts
/// `alpha` at the head of the result.
///
/// The node-declaration axis is the one the emitted code actually rides on and
/// the one a fixture is likeliest to leave untested: `codegen::graph` calls
/// `.addNode` in declaration order, so a fixture declaring its writers in
/// ascending id order would pass whether the scheduler ordered writes by node id
/// or by registration. Declaring them in reverse is what tells the two apart.
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
///
/// Each half asserts on the trace rather than on the absence of one. A failed
/// run's trace holds the node it stopped at (PRD 5.3, see
/// `a_failed_runs_trace_holds_what_landed_and_the_node_it_stopped_at`), so
/// "`skip` did not swallow this" is the *presence* of a `failed` entry for the
/// reader — which a run that had continued past it could not produce, and which
/// an empty trace would not have distinguished from any other failure.
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
    let named = skipped.entries("named");
    assert_eq!(
        named.len(),
        1,
        "the reader is in the trace once: {:?}",
        skipped.trace()
    );
    assert_eq!(
        named[0]["outcome"], "failed",
        "`on_error: skip` did not turn the execution failure into a skip: {}",
        named[0]
    );
    assert!(
        named[0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("`pending`")),
        "…and the entry says what stopped it: {}",
        named[0]
    );
    assert!(
        !skipped.visited().contains(&"after".to_string()),
        "…so nothing continued past it: {:?}",
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
    let read = bound.entries("bound");
    assert_eq!(
        read.len(),
        1,
        "the reader is in the trace once: {:?}",
        bound.trace()
    );
    assert_eq!(read[0]["outcome"], "failed", "{}", read[0]);
    assert_eq!(
        read[0]["fallback"],
        Value::Null,
        "a `fallback:` did not route around it either — the entry names none, \
         and a fallback that had been taken would: {}",
        read[0]
    );
    assert!(
        !bound.visited().contains(&"rescue".to_string()),
        "…and the fallback target never ran: {:?}",
        bound.visited()
    );
}

/// A run that quiesces and then cannot answer fails like every other run that
/// cannot answer, carrying the trace it made (PRD 5.3, grammar 7.6.3, 10.1).
///
/// The same unset channel as the test above, read at the *other* place a run
/// reads state. There, the read builds a node's input and the graph dies
/// mid-run; here the graph runs to quiescence and the read is the one
/// materializing the flow's `outputs:` — so this is the failure whose routing
/// record is **complete**. Every step landed; the whole account of how the flow
/// got somewhere it could not answer from is in hand at the moment it fails.
///
/// Which is why the failure has to be the same shape as the others. Every caller
/// of `runFlow` — `agent-compose run`, the generated `serve` app, this suite's
/// own driver — reads `FlowFailure.trace`, and a bare `Error` raised past that
/// type reports an empty trace for exactly the run that has a full one.
#[test]
fn a_run_that_quiesces_without_an_output_fails_carrying_its_whole_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "activities",
        "flow.missing_output",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("FlowFailure"),
        "it fails as a `FlowFailure` — the one type a caller reads a trace off: \
         {failure}"
    );
    assert!(
        failure.contains("`pending`") && failure.contains("output field `pending`"),
        "…naming the channel and the reader (grammar 10.1, Decision D78): {failure}"
    );

    assert_eq!(
        run.visited(),
        ["pick", "other"],
        "the whole run is in the trace, because the whole run happened"
    );
    let pick = run.entries("pick");
    assert_eq!(pick[0]["outcome"], "completed");
    assert_eq!(
        pick[0]["routing"]["targets"],
        json!(["other"]),
        "the routing decision survives the failure, which is the point of \
         raising one that carries a trace: {}",
        pick[0]
    );
    assert_eq!(
        run.entries("other")[0]["writes"],
        json!(["checks"]),
        "…and so does what the branch it took wrote — to a channel that is not \
         the one the output field reads"
    );
}

/// A run that fails still reports every routing decision it made (PRD 5.3).
///
/// The trace is the record of what a run did, and the runs it is most wanted for
/// are the ones that did not finish. Nothing is checkpointed, so a graph
/// invocation that throws takes the state — and the trace inside it — with it;
/// what makes this assertable is that `runFlow` streams the run and keeps each
/// superstep, and that a node which throws hands its own entry to the failure.
///
/// `flow.dead_end` is the shape that decides both halves at once: `pre` runs,
/// writes and routes (the landed half), and `head` completes and then finds
/// every outgoing edge untaken — grammar 7.3 rule 7, which is a run-time failure
/// no static check can reach, and whose entry has to carry the guard values or
/// nothing says why the run stopped.
#[test]
fn a_failed_runs_trace_holds_what_landed_and_the_node_it_stopped_at() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "activities",
        "flow.dead_end",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("no viable route out of `head`"),
        "the run says which node had nowhere to go (grammar 7.3 rule 7): {failure}"
    );

    assert_eq!(
        run.visited(),
        ["pre", "head"],
        "both steps are in the trace: the one that landed, and the one that \
         stopped the run"
    );
    let pre = run.entries("pre");
    assert_eq!(pre[0]["outcome"], "completed");
    assert_eq!(
        pre[0]["writes"],
        json!(["checks"]),
        "what the landed step wrote survives the failure: {}",
        pre[0]
    );
    assert_eq!(pre[0]["routing"]["targets"], json!(["head"]));

    let head = run.entries("head");
    assert_eq!(head[0]["outcome"], "failed");
    assert_eq!(
        head[0]["routing"]["edges"][0]["value"],
        json!(false),
        "the guard that answered `false` is what explains the failure, so the \
         entry carries the decision rather than only the error: {}",
        head[0]
    );
    assert_eq!(
        head[0]["routing"]["targets"],
        json!([]),
        "…with no target, which is the failure itself: {}",
        head[0]
    );
    assert_eq!(
        head[0]["writes"],
        Value::Null,
        "the superstep a run dies in lands nothing, so its entry claims no \
         writes: {}",
        head[0]
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

    let Some(run) = harness::invoke(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    ) else {
        return;
    };
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
/// `crates/compose-core/tests/cel_conformance.rs`; this is the other half, over
/// the **same files**, run against the evaluator a
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

/// A cycle bounded only by a CEL exit condition runs the passes its guard asks
/// for (PRD 5.4, grammar 7.4 clause 2).
///
/// Grammar 7.4's two clauses are proofs of different strength and the superstep
/// ceiling a run takes has to be sized from both. Clause 1's `max_iterations` is
/// a number the composition declares, so a ceiling can be derived from it;
/// clause 2's exit condition declares nothing — grammar 7.4 says outright that
/// only clause 1 makes a loop provably finite — so a ceiling derived from
/// counting bounds alone gives a clause-2 cycle no budget at all and stops it at
/// `25 + |nodes|` supersteps.
///
/// `flow.long_loop` is that shape reduced to nothing else: one node, no
/// `max_iterations` anywhere, and a guard asking for 30 passes. `25 + 1` is 26,
/// so a run of it is the difference between the two sizings — it either
/// completes or dies at the net, and no assertion about the guard is needed to
/// tell which.
#[test]
fn a_cel_bounded_cycle_runs_the_passes_its_guard_asks_for() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "bounded-cycle",
        "flow.long_loop",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    run.succeeded();

    let passes = run.entries("step").len();
    assert_eq!(
        passes,
        30,
        "the guard's own count of passes ran, not the ceiling's: {:?}",
        run.visited()
    );
    assert!(
        passes > 26,
        "…and more than `25 + |nodes|`, which is what a ceiling sized from \
         counting bounds alone would have allowed"
    );
    assert_eq!(
        run.outputs()["log"].as_array().map(Vec::len),
        Some(30),
        "every pass appended: {}",
        run.outputs()["log"]
    );
}

/// A run stopped by the superstep ceiling says which bound the composition was
/// missing, rather than which knob LangGraph has (PRD 5.4, grammar 7.4).
///
/// Clause 2 is decided syntactically (Decision D98) and its guard is a runtime
/// value, so `validate` accepts a loop whose exit condition never comes true and
/// nothing inside the composition ends it. That is the case the ceiling exists
/// for, and the diagnostic is the whole of what a reader gets: LangGraph
/// announces reaching a `recursionLimit` with a link to its own troubleshooting
/// page, naming a config key no `agent-compose` surface spells. The restatement
/// has to name the composition's own missing bound instead, and keep LangGraph's
/// error as the cause so nothing is hidden.
#[test]
fn a_run_that_reaches_the_superstep_ceiling_says_which_bound_was_missing() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_with(
        "bounded-cycle",
        "flow.runaway",
        &json!({}),
        &harness::environment(&provider),
    ) else {
        return;
    };
    let failure = run.failed();

    // The trace is the run's own count of the supersteps it took, so the message
    // is checked against what happened rather than against a literal that would
    // have to be edited every time the sizing moves.
    let supersteps = run.trace().len();
    assert!(
        failure.contains(&format!("ceiling of {supersteps} supersteps")),
        "the message names the ceiling the run actually reached: {failure}"
    );
    assert!(
        failure.contains("grammar 7.4 clause 2") && failure.contains("max_iterations"),
        "…says which bound the composition was missing, and how to declare it: \
         {failure}"
    );
    assert!(
        failure.contains("recursionLimit"),
        "…and how to raise the net for one run instead: {failure}"
    );
    assert!(
        failure.contains("GraphRecursionError"),
        "LangGraph's own error is kept as the cause rather than replaced: \
         {failure}"
    );

    assert!(
        run.trace().iter().all(|entry| entry["node"] == "spin"),
        "the trace of a run the net caught is still a trace: every superstep it \
         took is in it (PRD 5.3)"
    );
}

/// A homogeneous map dispatches one instance per item, bounded by
/// `max_concurrency`, and joins before the downstream edge fires.
#[test]
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

    let Some(run) = harness::invoke("fanout", "flow.spread", &[("goal", "ship it")], &provider)
    else {
        return;
    };
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

    let Some(run) = harness::invoke(
        "fanout",
        "flow.triage",
        &[("report", "a raw report")],
        &provider,
    ) else {
        return;
    };
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

    let Some(run) = harness::invoke("fanout", "flow.spread", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.succeeded();

    assert_eq!(
        run.outputs()["drafts"],
        json!(["done-0", "done-1", "done-2"]),
        "appended values are reordered by source-item index before the join"
    );

    // The record of the fan-out itself: one entry per source item, in index
    // order, whatever order they finished in (PRD 5.6, 5.3).
    let dispatched = run.entries("work")[0]["dispatches"].clone();
    assert_eq!(
        dispatched
            .as_array()
            .expect("a `map` node's trace entry records its dispatches")
            .iter()
            .map(|record| (record["index"].clone(), record["outcome"].clone()))
            .collect::<Vec<_>>(),
        (0..3)
            .map(|index| (json!(index), json!("completed")))
            .collect::<Vec<_>>()
    );
}

/// The catch-all takes the variants nothing routes, a sink route is joined like
/// any other, a detached one is resolved at dispatch, and a failed item is
/// skipped (grammar 8.6 rules 4, 6, 7, 10).
///
/// Four items, one per rule, and the whole claim rests on the run **succeeding**:
/// the `duplicate` item is dispatched to an agent whose call is deliberately
/// *unscripted*, so a join that waited on it would fail the map node and take the
/// run with it. That it does not is what "resolved at dispatch" means (Decision
/// D94) — the map completed, its outgoing edge fired, and the delivery's outcome
/// was never observed.
#[test]
fn a_sink_route_is_joined_and_a_detached_one_is_resolved_at_dispatch() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "needs_human", "summary": "unclear", "severity": "high" },
                    { "kind": "duplicate", "of": "issue-7" },
                    { "kind": "auto_fixable", "file": "b.rs", "hint": "widen it" },
                ],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "patch-a" }))).matching("a.rs"),
        // `min_length: 1` on `agent.fixer`'s `patch` refuses this, so item 3
        // fails — and `on_item_error: skip` drops it rather than the fan-out.
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "" }))).matching("b.rs"),
    ]);

    let Some(run) = harness::invoke(
        "fanout",
        "flow.sort",
        &[("report", "the build is red")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let outputs = run.outputs();
    assert_eq!(
        outputs["drafts"],
        json!(["patch-a"]),
        "the skipped item contributed nothing, and the one that landed did"
    );
    assert_eq!(
        outputs["tickets"],
        json!(["queued"]),
        "the sink route ran and was waited on: its result is in state"
    );

    let dispatched = run.entries("route")[0]["dispatches"].clone();
    let records = dispatched
        .as_array()
        .expect("the map records what it dispatched");
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record["index"].as_u64().expect("an index"),
                record["route"].as_str().expect("a route").to_string(),
                record["outcome"].as_str().expect("an outcome").to_string(),
            ))
            .collect::<Vec<_>>(),
        [
            (0, "auto_fixable".to_string(), "completed".to_string()),
            (1, "needs_human".to_string(), "completed".to_string()),
            (2, "$default".to_string(), "detached".to_string()),
            (3, "auto_fixable".to_string(), "skipped".to_string()),
        ],
        "every item is accounted for, in source-item order: {dispatched}"
    );
    assert_eq!(
        records[2]["attempts"],
        json!(0),
        "a detached dispatch has no observed outcome, so `on_item_error` never \
         applied to it (grammar 8.6 rule 7)"
    );
    assert!(
        records[3]["error"]
            .as_str()
            .is_some_and(|error| error.contains("agent.fixer")),
        "the skipped item says what was wrong with it: {}",
        records[3]
    );

    // Grammar 9.4's form, for the one dispatch that delivers without observing
    // an outcome: the execution id, then this dispatch's flattened instance
    // path. `route` is at the root instance, so its frame is the whole path.
    let execution = run.trace()[0]["step"].clone();
    assert_eq!(execution, json!(1), "the trace opens at step 1");
    let key = records[2]["idempotencyKey"]
        .as_str()
        .expect("a detached dispatch carries its key")
        .to_string();
    assert!(
        key.ends_with("/route/0/2"),
        "the key is `<execution id>/<node>/<traversal>/<item index>`: {key}"
    );
    assert!(
        key.starts_with("exec_"),
        "…opening with the execution's own id: {key}"
    );
    for record in records {
        assert!(
            record["idempotencyKey"]
                .as_str()
                .is_some_and(|held| held.ends_with(&format!(
                    "/route/0/{}",
                    record["index"].as_u64().expect("an index")
                ))),
            "every dispatch derives its own key: {record}"
        );
    }
}

/// A map whose target is a `flow.*` that itself fans out: two frames of instance
/// path, an inner index of its own, and channel values that stay inside their
/// instance (grammar 8.6, 9.4, 10.1).
#[test]
fn a_nested_fan_out_keys_and_isolates_each_instance_by_its_whole_path() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "tasks": [{ "steps": ["alpha", "beta"] }, { "steps": ["gamma"] }],
            })),
        ),
        // Every outcome is narrowed by the one item it answers, because the six
        // calls share two model ids and reach the server in scheduler order.
        Script::new(HAIKU, Outcome::structured(json!({ "part": "made-alpha" }))).matching("alpha"),
        Script::new(HAIKU, Outcome::structured(json!({ "part": "made-beta" }))).matching("beta"),
        Script::new(HAIKU, Outcome::structured(json!({ "part": "made-gamma" }))).matching("gamma"),
        Script::new(HAIKU, Outcome::structured(json!({ "line": "line-0" }))).matching("made-alpha"),
        Script::new(HAIKU, Outcome::structured(json!({ "line": "line-1" }))).matching("made-gamma"),
    ]);

    let Some(run) = harness::invoke("fanout", "flow.nested", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["rolled"],
        json!(["line-0", "line-1"]),
        "each instance's `outputs:` crossed back in source-item order"
    );

    // Instance-local channel values (grammar 10.1): each `flow.subtask` instance
    // appended to *its own* `parts`, so the second one rolled up one part rather
    // than three. Nothing else in this run can produce that.
    let turns: Vec<String> = provider
        .requests()
        .iter()
        .filter_map(|request| {
            request.body()["messages"][0]["content"]
                .as_str()
                .map(str::to_string)
        })
        .collect();
    assert!(
        turns.contains(&"{\"parts\":[\"made-alpha\",\"made-beta\"]}".to_string()),
        "the first instance rolled up both of its own parts, in index order: {turns:?}"
    );
    assert!(
        turns.contains(&"{\"parts\":[\"made-gamma\"]}".to_string()),
        "…and the second saw only its own: {turns:?}"
    );

    // `execution.item_index` is the **innermost** map's (grammar 4.1): the
    // second step of the first task is at index 1, and the only step of the
    // second task is back at 0.
    assert!(
        turns.contains(&"{\"step\":\"beta\",\"at\":1}".to_string()),
        "the second step of the first task is at index 1: {turns:?}"
    );
    assert!(
        turns.contains(&"{\"step\":\"gamma\",\"at\":0}".to_string()),
        "…and the only step of the second task is back at 0, because the index \
         names the innermost dispatch and nothing above it: {turns:?}"
    );

    // Grammar 9.4's nesting, as the keys the two levels derive: the outer map's
    // frame, then the inner one's, joined from the root instance down.
    let outer = run.entries("work")[0]["dispatches"].clone();
    let outer = outer.as_array().expect("the outer map dispatched");
    let second = outer[1]["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    assert!(
        second.ends_with("/work/0/1"),
        "the outer dispatch's frame is its own node, traversal and index: {second}"
    );
    let inner = outer[1]["inner"]
        .as_array()
        .expect("a dispatched `flow.*` carries its instance's trace")
        .iter()
        .find(|entry| entry["node"] == "steps")
        .expect("the instance ran its own map")
        .clone();
    let nested = inner["dispatches"][0]["idempotencyKey"]
        .as_str()
        .expect("a key")
        .to_string();
    assert_eq!(
        nested,
        format!("{second}/steps/0/0"),
        "the inner dispatch's key is the outer path with its own frame appended \
         — the flattened instance path of grammar 9.4"
    );
}

/// A fan-out over an empty array completes immediately, writes nothing, and its
/// outgoing edge fires exactly as if every instance had finished (rule 6).
#[test]
fn an_empty_fan_out_completes_and_its_downstream_edge_still_fires() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "tasks": [] })),
    ));

    let Some(run) = harness::invoke("fanout", "flow.nested", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["rolled"],
        json!([]),
        "the `append` channel is still at its identity element (grammar 10.1)"
    );
    assert_eq!(
        provider.requests().len(),
        1,
        "the planner, and nothing dispatched"
    );

    let work = run.entries("work");
    assert_eq!(work.len(), 1, "the map node ran");
    assert_eq!(work[0]["dispatches"], json!([]));
    assert_eq!(
        work[0]["routing"]["targets"],
        json!(["__end__"]),
        "and its outgoing edge fired: a zero-instance dispatch is a completion"
    );
}

/// The two reduce policies a fan-out may write besides `append`: a `merge`
/// channel resolves each key to the **highest-indexed** item's write, and a
/// `last_wins` channel takes that item's write outright (grammar 8.6 rule 5,
/// 10.2).
///
/// `winner` declares **no** `default:`, which is the shape that makes this more
/// than a restatement of the `append` tests. A channel with no initial value
/// holds nothing until its first write, and LangGraph keeps the first update to
/// an empty channel *verbatim* rather than passing it through the reducer — so a
/// map that handed its ordered batch straight to the channel would leave the
/// batch object there, and every read of `winner` would get an object where the
/// composition declares a string. Nothing in a run says so except the value: it
/// type-checks, it constructs, and the flow's own `outputs:` carry it out.
///
/// Completion order is the reverse of source order, so an implementation that
/// folded in completion order would answer `w-A`/`who: A` here.
#[test]
fn a_merge_and_an_undefaulted_last_wins_channel_take_the_highest_indexed_write() {
    let provider = MockProvider::start().expect("a loopback port");
    let tasks: Vec<Value> = ["A", "B", "C"]
        .into_iter()
        .map(|name| json!({ "title": format!("task-{name}"), "body": "do it" }))
        .collect();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "tasks": tasks })),
    ));
    // Item 0 answers last, item 2 first.
    for (name, delay) in [("A", 300), ("B", 150), ("C", 0)] {
        provider.enqueue(
            Script::new(
                HAIKU,
                Outcome::structured(json!({
                    "piece": { "who": name, "note": format!("note-{name}") },
                    "label": format!("w-{name}"),
                }))
                .after(Duration::from_millis(delay)),
            )
            .matching(format!("task-{name}")),
        );
    }

    let Some(run) = harness::invoke("fanout", "flow.tally", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.succeeded();

    let outputs = run.outputs();
    assert_eq!(
        outputs["winner"],
        json!("w-C"),
        "a `last_wins` channel with no `default:` holds the last write in \
         canonical order — the value, not the batch it arrived in: {outputs}"
    );
    assert_eq!(
        outputs["totals"],
        json!({ "who": "C", "note": "note-C" }),
        "and a `merge` channel resolves each key to the highest-indexed item's \
         write, whatever order the instances finished in: {outputs}"
    );
    assert_eq!(
        run.entries("score")[0]["writes"],
        json!(["totals", "winner"]),
        "the map records both channels it wrote"
    );
    assert!(provider.snapshot().is_drained());
}

/// A map inside a bounded cycle dispatches once per traversal, and the two
/// traversals' detached deliveries carry **different** idempotency keys —
/// grammar 9.4's traversal ordinal, and the first of the two properties it
/// exists for ("distinct effects get distinct keys").
///
/// The keys are asserted where they land rather than only where they are
/// derived: the sink is an `exec:` tool, and what it reports is the environment
/// it was run with. Without the delivery half, a compiler could derive a perfect
/// key, record it in the trace, and send a sink nothing to dedupe on — which is
/// exactly the shape PRD 5.6 settles ("passed to the sink automatically, and
/// sinks are documented to dedupe on it").
#[test]
fn each_traversal_of_a_map_delivers_its_detached_sink_a_key_of_its_own() {
    let provider = MockProvider::start().expect("a loopback port");
    let sweep = |file: &str| {
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": file, "hint": "rename it" },
                    { "kind": "needs_human", "summary": format!("look at {file}"), "severity": "high" },
                ],
            })),
        )
    };
    provider.enqueue_all([
        sweep("a.rs"),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "patch-a" }))).matching("a.rs"),
        sweep("b.rs"),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "patch-b" }))).matching("b.rs"),
    ]);

    let scratch = harness::Scratch::new("audit");
    let log = scratch.path().join("audit.log");
    harness::shim(
        scratch.path(),
        "record-audit",
        // What the sink was handed, one line per delivery: the grammar 9.4 key,
        // out of the `IDEMPOTENCY_KEY` variable that section's delivery surface
        // says an `exec:`-bound target receives it in.
        "printf '%s\\n' \"$IDEMPOTENCY_KEY\" >> \"$AUDIT_LOG\"\nprintf 'logged'\n",
    );

    let mut environment = harness::environment(&provider);
    environment.push(("AUDIT_LOG".to_string(), log.display().to_string()));
    environment.push((
        "PATH".to_string(),
        format!(
            "{}:{}",
            scratch.path().display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    ));

    let Some(run) = harness::invoke_with(
        "fanout",
        "flow.recheck",
        &json!({ "report": "the build is red" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["drafts"],
        json!(["patch-a", "patch-b"]),
        "one pass of the cycle appended each patch"
    );

    // Two traversals of one node, and the ordinal is what tells them apart.
    let passes = run.entries("fan");
    assert_eq!(passes.len(), 2, "the cycle ran the map twice");
    let keys: Vec<String> = passes
        .iter()
        .map(|entry| {
            entry["dispatches"][1]["idempotencyKey"]
                .as_str()
                .unwrap_or_else(|| panic!("the detached dispatch carries a key: {entry}"))
                .to_string()
        })
        .collect();
    assert!(
        keys[0].ends_with("/fan/0/1") && keys[1].ends_with("/fan/1/1"),
        "the traversal ordinal is the component that distinguishes them: {keys:?}"
    );
    assert_eq!(
        passes[0]["dispatches"][1]["outcome"],
        json!("detached"),
        "the sink route is fire-and-forget: {}",
        passes[0]
    );

    // …and what the sink was actually sent. The deliveries are not waited on, so
    // the run reached quiescence without them — but a `spawn`ed child keeps the
    // process alive, so both have landed by the time it exits.
    let delivered: Vec<String> = std::fs::read_to_string(&log)
        .expect("the detached sink ran and wrote what it was handed")
        .lines()
        .map(str::to_string)
        .collect();
    assert_eq!(
        delivered, keys,
        "each delivery carried its own dispatch's key, which is what a sink \
         dedupes on (PRD 5.6, grammar 9.4)"
    );
}

/// A fan-out its own `on_error:` absorbed still says what it dispatched
/// (grammar 8.6 rule 10, PRD 5.3, 5.6).
///
/// This is the `fail` half of rule 10's idiom — "retry each item, and if one
/// still fails, skip the fan-out" — and it is where a map node's record is worth
/// the most and easiest to lose: the node produced no answer, so everything it
/// knew has to travel out on the failure or not at all. The items that did run
/// are the reason it matters. One of them wrote a patch that the skip then
/// discarded, and one of them was **delivered to a sink**: the audit line is on
/// disk, the enqueue is not undone by the node being skipped, and the dispatch
/// record with its idempotency key is the only place a reader learns that it
/// happened at all.
///
/// The failing item says `failed` rather than `skipped`, because `on_item_error`
/// here is the default `fail` and nothing skipped it — what was skipped is the
/// map node.
#[test]
fn a_fan_out_absorbed_by_its_own_on_error_still_records_every_dispatch() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "needs_human", "summary": "look at b.rs", "severity": "high" },
                    { "kind": "auto_fixable", "file": "c.rs", "hint": "widen it" },
                ],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "patch-a" }))).matching("a.rs"),
        // `min_length: 1` on `agent.fixer`'s `patch` refuses this, so item 2
        // fails — and with `on_item_error` at its default `fail`, the map node
        // fails with it.
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "" }))).matching("c.rs"),
    ]);

    let scratch = harness::Scratch::new("absorb");
    let log = scratch.path().join("audit.log");
    harness::shim(
        scratch.path(),
        "record-audit",
        "printf '%s\\n' \"$IDEMPOTENCY_KEY\" >> \"$AUDIT_LOG\"\nprintf 'logged'\n",
    );

    let mut environment = harness::environment(&provider);
    environment.push(("AUDIT_LOG".to_string(), log.display().to_string()));
    environment.push((
        "PATH".to_string(),
        format!(
            "{}:{}",
            scratch.path().display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    ));

    let Some(run) = harness::invoke_with(
        "fanout",
        "flow.absorb",
        &json!({ "report": "the build is red" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["drafts"],
        json!([]),
        "the map node was skipped, so none of its writes landed — including the \
         one item that produced a patch (grammar 9.2)"
    );

    let entry = run.entries("fan")[0].clone();
    assert_eq!(
        entry["outcome"],
        json!("skipped"),
        "the map node's own `on_error: skip` absorbed the failed item: {entry}"
    );
    let records = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("a fan-out that failed still records what it dispatched: {entry}")
        })
        .clone();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record["index"].as_u64().expect("an index"),
                record["route"].as_str().expect("a route").to_string(),
                record["outcome"].as_str().expect("an outcome").to_string(),
            ))
            .collect::<Vec<_>>(),
        [
            (0, "auto_fixable".to_string(), "completed".to_string()),
            (1, "needs_human".to_string(), "detached".to_string()),
            (2, "auto_fixable".to_string(), "failed".to_string()),
        ],
        "every item is accounted for, in source-item order, and the one that \
         failed says so: {entry}"
    );
    assert!(
        entry["error"]
            .as_str()
            .is_some_and(|error| error.contains("item 2")),
        "…and the node's own error names the item that failed it: {entry}"
    );

    // The effect that outlived the skip. A detached delivery is issued at
    // dispatch and never unwound, so the record and its key are what tell a
    // reader the sink was written to (PRD 5.6, grammar 9.4).
    let key = records[1]["idempotencyKey"]
        .as_str()
        .expect("the detached dispatch carries its key")
        .to_string();
    assert!(
        key.ends_with("/fan/0/1"),
        "the key names the dispatch site: {key}"
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .expect("the detached sink ran")
            .lines()
            .collect::<Vec<_>>(),
        [key.as_str()],
        "the sink was really delivered, under exactly the key the record shows"
    );
}

/// A fan-out its own **`timeout:`** cut short still delivers the sink it had
/// queued, and still says what it dispatched (grammar 8.6 rules 7, 9, 9.2,
/// PRD 5.3, 5.6).
///
/// A `map` takes a node policy like any other node, and grammar 9.3 level 3 puts
/// one on every node a `defaults:` block covers — the emitted
/// `examples/triage-fanout` carries `timeoutMs: 90000` on its `dispatch` node —
/// so a fan-out under a deadline is the ordinary compiled shape rather than an
/// exotic one. It is also the one shape where both of a fan-out's promises are
/// hardest to keep, because a deadline is *raced* (grammar 9.2): the map's
/// promise is abandoned where it stands, so nothing it was holding comes back.
///
/// Two things have to survive that, and `max_concurrency: 1` is what makes each
/// decidable. The `auto_fixable` item takes the only permit and waits on an
/// answer scripted to arrive long after the budget; the detached sink behind it
/// is still in the **queue** when the budget runs out.
///
///   * **The delivery is issued anyway.** Its clock is its own rather than the
///     node's, so it does not start against an already-aborted signal, throw
///     before anything reaches the wire, and disappear into the catch that keeps
///     a detached dispatch from failing the flow. That would be a message
///     recorded as `detached` and never sent — the lost message PRD 5.6 trades
///     at-least-once delivery and a dedupe key to avoid. The sink is an `exec:`
///     tool that writes the `IDEMPOTENCY_KEY` it was handed to a file, so the
///     claim is settled on disk rather than in a record.
///   * **The record survives.** The item that was delivered had its effect, and
///     with no answer and no `ItemFailure` the dispatch record is the only
///     account of it there will ever be — while the run itself *succeeds*,
///     because the node's `on_error: skip` absorbs the deadline.
#[test]
fn a_fan_out_cut_short_by_its_own_timeout_still_delivers_and_records_its_sink() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "needs_human", "summary": "look at b.rs", "severity": "high" },
                ],
            })),
        ),
        // Far longer than the node's 400ms budget, so the deadline is what ends
        // the item rather than the answer.
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "patch": "patch-a" })).after(Duration::from_secs(5)),
        )
        .matching("a.rs"),
    ]);

    let scratch = harness::Scratch::new("expire");
    let log = scratch.path().join("audit.log");
    harness::shim(
        scratch.path(),
        "record-audit",
        // The sink does a little work before it acknowledges, and that is what
        // makes the difference observable. `spawn` is handed the delivery's
        // signal, and an already-aborted one kills the child on the next tick —
        // after it has started, so a sink that wrote its line and exited in the
        // same instant would race that kill and settle nothing. Any real sink
        // takes longer than a tick to do its work; this one says so out loud.
        "sleep 0.3\nprintf '%s\\n' \"$IDEMPOTENCY_KEY\" >> \"$AUDIT_LOG\"\nprintf 'logged'\n",
    );

    let mut environment = harness::environment(&provider);
    environment.push(("AUDIT_LOG".to_string(), log.display().to_string()));
    environment.push((
        "PATH".to_string(),
        format!(
            "{}:{}",
            scratch.path().display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    ));

    let Some(run) = harness::invoke_with(
        "fanout",
        "flow.expire",
        &json!({ "report": "the build is red" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["drafts"],
        json!([]),
        "the map node was skipped, so none of its writes landed (grammar 9.2)"
    );

    let entry = run.entries("fan")[0].clone();
    assert_eq!(
        entry["outcome"],
        json!("skipped"),
        "the node's own `on_error: skip` absorbed the deadline: {entry}"
    );
    assert!(
        entry["error"]
            .as_str()
            .is_some_and(|error| error.contains("timed out")),
        "…and the error says what ended it: {entry}"
    );

    // The record the abandoned promise could not have carried. Only the detached
    // dispatch is here: it is resolved at dispatch (D94), while the joined
    // instance was still in flight when the budget expired and so had no outcome
    // to report — the entry's own error is what accounts for that one.
    let records = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("a fan-out its deadline cut short still records what it dispatched: {entry}")
        })
        .clone();
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record["index"].as_u64().expect("an index"),
                record["route"].as_str().expect("a route").to_string(),
                record["outcome"].as_str().expect("an outcome").to_string(),
            ))
            .collect::<Vec<_>>(),
        [(1, "needs_human".to_string(), "detached".to_string())],
        "the dispatch that resolved is accounted for: {entry}"
    );

    // …and the delivery itself, which the bound had merely *delayed* past the
    // deadline. It reaches the sink under exactly the key the record shows.
    let key = records[0]["idempotencyKey"]
        .as_str()
        .expect("the detached dispatch carries its key")
        .to_string();
    assert!(
        key.ends_with("/fan/0/1"),
        "the key names the dispatch site: {key}"
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .expect(
                "the detached sink ran: a delivery still queued for a permit when the node's \
                 budget ran out is issued anyway (grammar 8.6 rule 7, PRD 5.6)"
            )
            .lines()
            .collect::<Vec<_>>(),
        [key.as_str()],
        "the sink was delivered under the key the record shows"
    );
}

/// `context: inherit` shares the caller's conversation with the instance in both
/// directions, and the instantiation site's `policy:` is level 1 for the nodes
/// inside it (grammar 8.5, 9.3, 10.4).
#[test]
fn a_subflow_inherits_the_callers_history_and_takes_its_instantiation_policy() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "findings": [] }))),
        // `flow.normalize`'s `clean` declares no `retry:` of its own. The one it
        // gets is the `again` node's `policy:`, which is level 1 for every node
        // inside the instance — so this failure is retried rather than fatal.
        Script::new(HAIKU, Outcome::rate_limit()).matching("a raw report"),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a clean report" })),
        )
        .matching("a raw report"),
        Script::new(HAIKU, Outcome::structured(json!({ "line": "closed" })))
            .matching("Roll the parts"),
    ]);

    let Some(run) = harness::invoke(
        "fanout",
        "flow.continue",
        &[("report", "a raw report")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["normalized"], "a clean report");

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        4,
        "the opener, two attempts at the subflow's node, and the closer"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert_eq!(
        recorded[1].body()["messages"].as_array().map(Vec::len),
        Some(3),
        "the instance's own node sees the caller's exchange plus its own turn: {}",
        recorded[1].body()["messages"]
    );
    assert_eq!(
        recorded[3].body()["messages"].as_array().map(Vec::len),
        Some(5),
        "…and what the instance said came back, so the caller's next node sees \
         both exchanges: {}",
        recorded[3].body()["messages"]
    );
    assert_eq!(
        run.entries("again")[0]["inner"]
            .as_array()
            .expect("the instance's own trace")
            .iter()
            .map(|entry| entry["node"].clone())
            .collect::<Vec<_>>(),
        [json!("clean")],
        "the instance's routing record is nested under the node that ran it"
    );
    assert!(provider.snapshot().is_drained());
}

/// The documented fan-out example runs: a subgraph, a discriminated fan-out to
/// fixer agents and two sinks, a concurrent branch, and a join whose result is
/// in source-item order (PRD 5.6).
///
/// `examples/triage-fanout` is the project PRD 5.6 is written about, and running
/// the *example* rather than a fixture is the only way an acceptance test speaks
/// about what a reader is shown. It reaches further than any fixture does,
/// because it is written for a reader rather than for a test: a `flow:` node into
/// `flow.enrich`, a `function:` node over an `exec` tool, a four-item tagged
/// union across three routes — two of them `tool.*` sinks the join waits on — a
/// concurrent `http:` branch converging at equal depth, and an inline `exec:`
/// node after it.
///
/// It **stops** at `approve`, which is a `human:` node — the one construct on its
/// path this compiler release does not execute (PRD §7 M1 leaves the `human`
/// runtime to `serve`). That is asserted rather than worked around: the run gets
/// all the way there, the trace holds every step it took, and the node it stopped
/// at names the construct and the bullet that lands it.
///
/// The fixer answering **item 0** is delayed past the one answering item 3, so
/// the two patches complete in the reverse of source order. What `summarize` is
/// then sent is the whole claim of PRD 5.6's replay guarantee, observed on the
/// wire rather than inferred.
#[test]
fn the_triage_fanout_example_routes_every_finding_and_joins_them_in_source_order() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "src/a.rs", "patch_hint": "rename it" },
                    { "kind": "needs_human", "summary": "needs judgement", "severity": "high" },
                    { "kind": "duplicate", "of": "the first finding" },
                    { "kind": "auto_fixable", "file": "src/b.rs", "patch_hint": "widen it" },
                ],
            })),
        ),
        // Item 0 answers last. Completion order is the reverse of source order,
        // which is the only arrangement under which the join means anything.
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "patch": "patch-a", "explanation": "renamed" }))
                .after(Duration::from_millis(300)),
        )
        .matching("src/a.rs"),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "patch": "patch-b", "explanation": "widened" })),
        )
        .matching("src/b.rs"),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "summary": "one fix, one ticket" })),
        )
        .matching("Summarize the triage run"),
    ]);

    let shims = harness::Scratch::new("triage-shims");
    harness::shim(
        shims.path(),
        "repo-grep",
        "printf '{\"matches\":[\"src/a.rs:42:boom\"]}'\n",
    );
    harness::shim(
        shims.path(),
        "run-checks",
        "printf 'two checks failed'\nexit 1\n",
    );
    let sinks = shims.path().join("sinks.jsonl");

    let mut environment = harness::environment(&provider);
    for (name, value) in [
        ("ANTHROPIC_API_KEY", "mock-provider-key".to_string()),
        ("LOCAL_LLM_KEY", "mock-provider-key".to_string()),
        ("LOCAL_LLM_URL", provider.base_url()),
        ("DEPLOY_ENV", "acceptance".to_string()),
        ("QUEUE_HOST", "queue.invalid".to_string()),
        ("QUEUE_TOKEN", "queue-token".to_string()),
        ("TRIAGE_HOST", "triage.invalid".to_string()),
        ("TRIAGE_TOKEN", "triage-token".to_string()),
        ("REPO_ROOT", shims.path().display().to_string()),
        ("RG_CONFIG_PATH", shims.path().display().to_string()),
        ("SINK_LOG", sinks.display().to_string()),
        (
            "PATH",
            format!(
                "{}:{}",
                shims.path().display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        ),
    ] {
        environment.push((name.to_string(), value));
    }

    let Some(run) = harness::invoke_entrypoint(
        &harness::example("triage-fanout"),
        "examples/triage-fanout",
        "flow.triage",
        &json!({ "report": "the parser panics", "pattern": "panic!" }),
        &environment,
        Some(ANSWER_THE_EXAMPLES_INFRASTRUCTURE),
    ) else {
        return;
    };

    // Everything up to the one construct this release does not execute.
    let failure = run.failed();
    assert!(
        failure.contains("a `human` pause is not executed by this compiler release"),
        "the run reaches `approve` and stops there, saying what it is: {failure}"
    );
    assert_eq!(
        run.visited(),
        [
            "enrich",
            "scan",
            "classify",
            "announce",
            "dispatch",
            "verify",
            "summarize",
            "remember",
            "approve",
        ],
        "the whole path, with `dispatch` and `announce` concurrent in one step \
         and `verify` running once after both"
    );

    // The two concurrent branches ran in the same step, and the convergence in
    // the next one — the fan-out barrier is what put them there (grammar 7.6).
    let step = |node: &str| run.entries(node)[0]["step"].clone();
    assert_eq!(step("dispatch"), step("announce"));
    assert_eq!(
        step("verify").as_i64().expect("a step"),
        step("dispatch").as_i64().expect("a step") + 1,
    );

    // Every finding reached its own route, in source-item order, and the two
    // sink routes were waited on like any other (PRD 5.6).
    let dispatched = run.entries("dispatch")[0]["dispatches"].clone();
    assert_eq!(
        dispatched
            .as_array()
            .expect("the map records what it dispatched")
            .iter()
            .map(|record| (
                record["index"].clone(),
                record["route"].clone(),
                record["target"].clone(),
                record["outcome"].clone(),
            ))
            .collect::<Vec<_>>(),
        [
            (
                json!(0),
                json!("auto_fixable"),
                json!("agent.fixer"),
                json!("completed")
            ),
            (
                json!(1),
                json!("needs_human"),
                json!("tool.review_queue"),
                json!("completed")
            ),
            (
                json!(2),
                json!("$default"),
                json!("tool.dead_letter"),
                json!("completed")
            ),
            (
                json!(3),
                json!("auto_fixable"),
                json!("agent.fixer"),
                json!("completed")
            ),
        ],
        "{dispatched}"
    );

    // The join's result, on the wire: `summarize` reads `state.patches`, and the
    // fixer that answered *last* is the one whose patch is *first* — because the
    // order is the source array's, never the providers' (grammar 7.6.4 clause 2).
    let summarized = provider
        .requests()
        .into_iter()
        .find(|request| {
            request.body()["system"]
                .as_str()
                .is_some_and(|prompt| prompt.starts_with("Summarize the triage run"))
        })
        .expect("the summarizer ran");
    let turns = summarized.body()["messages"].clone();
    let asked = turns
        .as_array()
        .and_then(|turns| turns.last())
        .expect("a request carries at least the node's own turn")["content"]
        .as_str()
        .expect("the bound input is the last user turn")
        .to_string();
    assert_eq!(
        asked, "{\"report\":\"a normalized report\",\"patches\":[\"patch-a\",\"patch-b\"]}",
        "the subgraph's output and the fan-out's, both read back from state"
    );

    // What the sinks were actually sent, recorded by the preamble that answered
    // them: the narrowed payload of each variant, and nothing from another's.
    let delivered: Vec<Value> = std::fs::read_to_string(&sinks)
        .expect("the preamble logged what the example's infrastructure was sent")
        .lines()
        .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
        .collect();
    let sent = |path: &str| -> Value {
        delivered
            .iter()
            .find(|entry| entry["path"] == path)
            .unwrap_or_else(|| panic!("nothing reached `{path}`: {delivered:#?}"))["body"]
            .clone()
    };
    assert_eq!(
        sent("/v1/normalize"),
        json!({ "report": "the parser panics" }),
        "the subgraph got its input through the instantiating node's bindings alone"
    );
    assert_eq!(
        sent("/v1/tickets"),
        json!({ "summary": "needs judgement", "severity": "high" }),
        "the `needs_human` route is narrowed to its own variant's payload"
    );
    assert_eq!(
        sent("/v1/dead-letter"),
        json!({ "kind": "duplicate", "payload": "the first finding" }),
        "…and the catch-all to the unrouted variant's"
    );
    assert_eq!(
        sent("/v1/triage-started"),
        json!({ "report": "a normalized report" }),
        "the concurrent branch ran too"
    );
    assert!(provider.snapshot().is_drained());
}

/// The host preamble `examples/triage-fanout` needs to run without a network.
///
/// Two jobs, and both are the *host's* rather than the composition's. The
/// example names no `base_url:`, so its provider resolves `https://api.anthropic.com`
/// and the mock is where that goes — the same redirect
/// `the_review_loop_example_runs_its_cycle_against_the_mock_provider` uses. And
/// the four HTTPS endpoints it talks to are a normalizer, a ticket queue, a
/// dead-letter queue and a webhook: infrastructure the example documents talking
/// to and this milestone is not about. They are answered here, in their own
/// declared shapes, and what each was sent is logged so a test can assert on the
/// payloads a sink route delivered.
const ANSWER_THE_EXAMPLES_INFRASTRUCTURE: &str = r#"import { appendFileSync } from "node:fs";
import process from "node:process";

const upstream = "https://api.anthropic.com";
const mock = process.env["MOCK_BASE_URL"].replace(/\/+$/, "");
const log = process.env["SINK_LOG"];

// Each endpoint answers the shape the tool that calls it declares, at the status
// it declares: `tool.review_queue` expects 201 and decodes two fields,
// `flow.enrich`'s `fetch` expects 200 and decodes one.
const answers = {
  "/v1/normalize": [200, { report_normalized: "a normalized report" }],
  "/v1/tickets": [201, { ticket_id: "T-1", queued: true }],
  "/v1/dead-letter": [200, { accepted: true }],
  "/v1/triage-started": [202, { started: true }],
};

const inner = globalThis.fetch;
globalThis.fetch = async (resource, init) => {
  const href =
    typeof resource === "string"
      ? resource
      : resource instanceof URL
        ? resource.href
        : resource.url;
  if (href.startsWith(upstream)) {
    return inner(mock + href.slice(upstream.length), init);
  }
  const answer = answers[new URL(href).pathname];
  if (answer === undefined) return inner(href, init);
  appendFileSync(
    log,
    `${JSON.stringify({
      path: new URL(href).pathname,
      body: JSON.parse(init?.body ?? "null"),
    })}\n`,
  );
  return new Response(JSON.stringify(answer[1]), {
    status: answer[0],
    headers: { "content-type": "application/json" },
  });
};
"#;

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
    // The abort reason becomes the failure's `cause`, and a run prints the whole
    // chain — so it counts the attempts the node *made* too. One attempt was
    // made here; a chain saying `0` under a message saying `1` reads as two
    // facts about one node.
    assert!(
        failure.contains("after 1 attempt(s)") && !failure.contains("after 0 attempt(s)"),
        "the message and the cause count the same attempts: {failure}"
    );

    // And the trace says the same thing the message does: `on_error: fail` ends
    // the run *at* this node, so the node it ended at is the trace's last entry
    // — with the attempts it made, counted once more (PRD 5.3).
    let review = run.entries("review");
    assert_eq!(
        review.len(),
        1,
        "the node that timed out is in the trace: {:?}",
        run.trace()
    );
    assert_eq!(review[0]["outcome"], "failed", "{}", review[0]);
    assert_eq!(review[0]["attempts"], 1, "{}", review[0]);
    assert_eq!(
        run.visited().last().map(String::as_str),
        Some("review"),
        "…and it is where the run stopped: {:?}",
        run.visited()
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
    // The count the *message* carries is the same one, which it has to be: the
    // abort reason becomes the failure's `cause` and a run prints the chain, so
    // two numbers for one fact would be read as two facts.
    assert!(
        slow[0]["error"]
            .as_str()
            .is_some_and(|error| error.contains("after 1 attempt(s)")),
        "the message counts what the trace counts: {}",
        slow[0]["error"]
    );
}

/// The same budget once more, over the one activity the runtime does not
/// control: a grammar 6.1 `function:` binding whose host implementation never
/// looks at `context.signal`.
///
/// Neither test above decides this. `exec:` and `http:` observe the abort
/// because the runtime is what spawns and fetches for them, so a deadline that
/// only *signalled* — handing `context.signal` to the activity and trusting it
/// — would still pass both. A host function is arbitrary caller code, and so is
/// a host-implemented tool called inside an agent's tool loop, which reaches
/// `invoke` with this same context; grammar 9.2 bounds "one node execution" and
/// names no kind that is exempt. So `runActivity` **races** the deadline against
/// the activity, and the node fails on time whatever the implementation does.
///
/// What racing cannot do is stop the work, and the assertion is about that
/// boundary. The host answers `too late` two seconds into a 300ms budget: a
/// cooperative-only deadline would let the node complete and write that string,
/// while a raced one fails the node at 300ms and *discards* the value when it
/// arrives. `rescued` is the fallback's, so only the second reading produces it.
#[test]
fn a_node_timeout_fires_over_a_host_function_that_ignores_its_signal() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke_hosted(
        "activities",
        "flow.stubborn",
        &json!({}),
        &harness::environment(&provider),
        // No mention of `context.signal` anywhere in here, which is the point:
        // this is what an ordinary host implementation looks like.
        Some(
            r#"import { registerFunction } from "./src/runtime.ts";
registerFunction("rank_candidates", async () => {
  await new Promise((resolve) => setTimeout(resolve, 2000));
  return { ranked: "too late" };
});
"#,
        ),
    ) else {
        return;
    };
    run.succeeded();

    assert_eq!(
        run.outputs()["report"],
        "rescued",
        "the node failed at its deadline, so the abandoned call's answer was \
         discarded rather than written"
    );
    assert_eq!(run.visited(), ["call", "rescue"]);
    let call = run.entries("call");
    assert_eq!(call[0]["outcome"], "failed", "{}", call[0]);
    assert_eq!(call[0]["fallback"], "rescue", "{}", call[0]);
    // One attempt, and the budget is what ended it — not a host function that
    // threw, which would reach the same `outcome` by a different route.
    assert_eq!(call[0]["attempts"], json!(1));
    assert!(
        call[0]["error"]
            .as_str()
            .is_some_and(|error| { error.contains("timed out") && error.contains("300ms budget") }),
        "the trace names the budget that ended it: {}",
        call[0]["error"]
    );
    assert!(
        call[0]["routing"].is_null(),
        "a fallback is taken *instead of* the node's own edges: {}",
        call[0]
    );
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
                   if (typeof flow.stream !== 'function') throw new Error(`${flow.address} has no graph`);\
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
