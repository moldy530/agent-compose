//! The M1 acceptance suite: PRD §7 M1, decomposed into observable behaviours and
//! made runnable.
//!
//! CLAUDE.md's validation strategy asks for a milestone's acceptance criteria to
//! be **defined as runnable tests before implementation of that milestone
//! begins**. This file is that definition. Each test names one thing PRD §7 M1
//! promises, in terms of what a compiled graph does when it runs — what it sends
//! to the provider, what it writes to state, what it prints — rather than in
//! terms of the TypeScript it is made of. `tests/m1_inventory.rs` is the map from
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
//!   `#[ignore = "M1: …"]` naming what must land first, and each has a **real
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
#[path = "m1_acceptance/harness.rs"]
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
// PRD §7 M1, bullet 1 — "IR → deterministic LangGraph TypeScript". Pending.
// ---------------------------------------------------------------------------

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
#[test]
#[ignore = "M1: `agent-compose build` must emit the state model, and `run` must execute it"]
fn state_channels_carry_their_declared_types_and_defaults() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let run = harness::run(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    );
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
#[test]
#[ignore = "M1: codegen must emit Zod discriminated unions for tagged-union outputs"]
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
#[ignore = "M1: codegen must emit agent node fns"]
fn an_agent_node_sends_its_prompt_input_and_output_schema() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let run = harness::run(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    );
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
#[ignore = "M1: codegen must emit agent node fns"]
fn an_agent_node_sends_its_prompt_input_and_output_schema_on_chat_completions() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));

    let run = harness::run(
        "agent-openai",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    );
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
#[ignore = "M1: codegen must emit the agent tool loop"]
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

    let run = harness::run(
        "agent-anthropic",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    );
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

/// A subgraph runs with explicit bindings in, name-based outputs back, and a
/// conversation history of its own.
#[test]
#[ignore = "M1: codegen must emit subgraphs"]
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
#[test]
#[ignore = "M1: codegen must emit routers with embedded CEL"]
fn an_edge_guard_routes_on_the_source_nodes_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "draft": "the first draft" })),
        )
        .matching("ship it"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        )
        .matching("the first draft"),
    ]);

    let run = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    );
    run.succeeded();

    assert_eq!(
        provider.requests().len(),
        2,
        "`approve` takes the `else:` edge to `end`, so the writer never runs again"
    );
    assert_eq!(run.outputs()["draft"], "the first draft");
}

/// The JS evaluator generated routers embed agrees with the Rust one the
/// validator links, over the shared conformance corpus.
///
/// CLAUDE.md makes this a CI gate: two interpreters is a semantic-drift risk, and
/// the corpus is the mitigation. The Rust side already runs in
/// `crates/compose-core/tests/cel_conformance.rs`; this is the other half.
#[test]
#[ignore = "M1: generated routers must embed a JS CEL evaluator"]
fn the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus() {
    let built = harness::build("bounded-cycle", "local");
    built.succeeded();

    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("compose-core/tests/fixtures/cel-conformance");
    let driver = built.root().join("cel-conformance.mjs");
    std::fs::write(
        &driver,
        r#"
import { readdirSync, readFileSync } from "node:fs";
import { evaluate } from "./cel.js";

const corpus = process.argv[2];
const divergences = [];
for (const file of readdirSync(corpus).filter((name) => name.endsWith(".json"))) {
  for (const testCase of JSON.parse(readFileSync(`${corpus}/${file}`, "utf8"))) {
    if (testCase.error !== undefined) continue;
    let actual;
    try {
      actual = evaluate(testCase.expression, testCase.input ?? {});
    } catch (error) {
      divergences.push({ name: testCase.name, threw: String(error) });
      continue;
    }
    if (JSON.stringify(actual) !== JSON.stringify(testCase.result)) {
      divergences.push({ name: testCase.name, expected: testCase.result, actual });
    }
  }
}
process.stdout.write(JSON.stringify(divergences));
"#,
    )
    .expect("the driver is written into the generated project");

    let output = std::process::Command::new("node")
        .arg(&driver)
        .arg(&corpus)
        .current_dir(built.root())
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

/// A bounded cycle stops at its budget and leaves through the escape edge, rather
/// than looping (PRD 5.4).
#[test]
#[ignore = "M1: codegen must emit the per-cycle iteration counter"]
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

    let run = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    );
    run.succeeded();

    // One write, then three (write, review) passes on the back-edge budget: the
    // fourth `revise` finds the budget spent and the `else:` edge fires.
    let calls = provider.requests().len();
    assert_eq!(
        calls, 8,
        "four writer calls and four reviewer calls, then the escape — not a loop"
    );
    assert_eq!(run.outputs()["draft"], "a draft");
}

/// A homogeneous map dispatches one instance per item, bounded by
/// `max_concurrency`, and joins before the downstream edge fires.
#[test]
#[ignore = "M1: codegen must emit `map` as LangGraph `Send`"]
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
#[ignore = "M1: codegen must emit discriminator-routed `map` dispatch"]
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
#[ignore = "M1: codegen must emit index-tagged reducers"]
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
#[ignore = "M1: codegen must emit retry policy"]
fn a_node_retries_its_model_call_per_its_declared_policy() {
    let provider = MockProvider::start().expect("a loopback port");
    // `write` declares `retry: { max: 2, backoff: 1s }`: one attempt plus two
    // retries, and the third answer is the one that lands.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::server_error()).times(2),
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        ),
    ]);

    let run = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    );
    run.succeeded();

    assert_eq!(
        provider.requests().len(),
        4,
        "three attempts at the writer, then the reviewer"
    );
    assert_eq!(run.outputs()["draft"], "a draft");
}

/// A node timeout fires on a provider that never answers, and the node's error
/// policy takes over from there.
#[test]
#[ignore = "M1: codegen must emit timeout policy"]
fn a_node_timeout_fires_and_its_error_policy_takes_over() {
    let provider = MockProvider::start().expect("a loopback port");
    // `review` declares `timeout: 10s`; the scripted provider never answers, and
    // the project's `defaults: { on_error: fail }` is what the run reports.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "draft": "a draft" }))),
        Script::new(SONNET, Outcome::timeout(Duration::from_secs(30))),
    ]);

    let run = harness::run(
        "bounded-cycle",
        "flow.review_loop",
        &[("goal", "ship it")],
        &provider,
    );
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
#[ignore = "M1: codegen must emit store-op nodes over the SQLite/local-disk backends"]
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
#[ignore = "M1: codegen must synthesize store tools"]
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
#[ignore = "M1: codegen must synthesize store tools"]
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
#[ignore = "M1: codegen must emit model routing with trace-recorded failover"]
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
#[ignore = "M1: codegen must emit model routing with trace-recorded failover"]
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
#[test]
#[ignore = "M1: generated code must check env-ref presence at process start"]
fn a_missing_env_ref_fails_at_process_start_naming_the_variable() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(SONNET, Outcome::text("never reached")));

    let environment: Vec<(String, String)> = harness::environment(&provider)
        .into_iter()
        .filter(|(name, _)| name != harness::API_KEY)
        .collect();
    let run = harness::run_with(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &environment,
    );
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
// PRD §7 M1, bullet 2 — build, run, serve, golden files. Pending.
// ---------------------------------------------------------------------------

/// `agent-compose build` writes a TypeScript project for the selected target.
#[test]
#[ignore = "M1: `agent-compose build` must exist"]
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
/// code (CLAUDE.md) — belongs to the codegen PR that has output to commit; what
/// makes those goldens *mean* anything is the determinism asserted here, because
/// a regeneration diff is only signal if identical input regenerates identically.
#[test]
#[ignore = "M1: `agent-compose build` must exist"]
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
#[ignore = "M1: `agent-compose run` must exist"]
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
#[ignore = "M1: `agent-compose serve` must exist"]
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
#[ignore = "M1: `agent-compose serve` must exist, and the `human` node runtime with it"]
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
#[test]
#[ignore = "M1: `agent-compose build` must exist, and the pinned LangGraph toolchain with it"]
fn every_generated_project_type_checks_and_constructs_its_graph() {
    for name in harness::FIXTURES {
        let built = harness::build(name, "local");
        built.succeeded();

        let install = std::process::Command::new("npm")
            .args(["install", "--no-audit", "--no-fund"])
            .current_dir(built.root())
            .output()
            .expect("npm runs");
        assert!(
            install.status.success(),
            "`{name}`'s dependencies did not install: {}",
            String::from_utf8_lossy(&install.stderr)
        );

        let typecheck = std::process::Command::new("npx")
            .args(["tsc", "--noEmit"])
            .current_dir(built.root())
            .output()
            .expect("tsc runs");
        assert!(
            typecheck.status.success(),
            "`{name}` does not type-check:\n{}",
            String::from_utf8_lossy(&typecheck.stdout)
        );

        // Constructing the graph is a stronger check than compiling it: a state
        // model LangGraph refuses, or an edge to a node that is not registered,
        // is a runtime error at build time and a green `tsc` either way.
        let construct = std::process::Command::new("node")
            .args(["--input-type=module", "-e", "await import('./graph.js');"])
            .current_dir(built.root())
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
