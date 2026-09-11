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
    Client, HARNESS_HEADER, MockProvider, Outcome, OutputMechanism, Personality, REFUSED_INVALID,
    REFUSED_UNSCRIPTED, RecordedRequest, Request, Script, ServerToolUse, StructuredOutput, Surface,
    ToolCall,
};
use serde_json::{Value, json};

/// The model ids the fixtures bind, which are the keys their scripts use.
const SONNET: &str = "claude-sonnet-4-6";
const HAIKU: &str = "claude-haiku-4-5";
const LOCAL: &str = "qwen3-coder-30b";
/// The `server-tools` fixture's two OpenAI models: one on each wire, which is
/// the whole point of the pair (Decision D122).
const GPT5: &str = "gpt-5";
const GPT_MINI: &str = "gpt-4o-mini";
/// The `builtin-tools` fixture's OpenAI model, which is the wire PRD resolved
/// q54 ruling d's other half is about: no provider-defined tool types, so the
/// two built-ins go out as ordinary function tools.
const GPT41: &str = "gpt-4.1";
/// The `provider-kinds` fixture's two models: one per Chat Completions kind that
/// is neither `openai_compatible` nor already bound elsewhere.
const AZURE_HOSTED: &str = "gpt-4o-mini";
const OPENAI_DIRECT: &str = "gpt-4o";

// ---------------------------------------------------------------------------
// PRD §7 M1, bullet 3 — "Mock provider server + e2e harness". Live.
// ---------------------------------------------------------------------------

/// The fixtures that validate with a **warning**, one entry per warning.
///
/// A warning is not a failure — the composition builds and runs — but it is also
/// not something a fixture should acquire quietly, so the ones that carry any
/// are named here with what they carry, repeats and all: the list's length is
/// the count `validate` must report, which is what makes a *new* warning a test
/// failure rather than a line nobody reads.
///
/// `server-tools` is the only entry and every one of its four is deliberate.
/// Three are the routes that cross a wire seam — a member with a suite failing
/// over to a member with a different one, which is the shape that makes
/// "a failover across two wires composes" testable at all, and exactly what
/// grammar 12.2 warns about. The fourth is `provider.searching_gateway`: every
/// `openai_compatible` entry is second-tier by construction, since no table
/// could be authoritative about what a gateway honours (Decision D122).
const FIXTURES_WITH_A_WARNING: &[(&str, &[&str])] = &[(
    "server-tools",
    &[
        "unknown-server-tool",
        "mismatched-server-tools",
        "mismatched-server-tools",
        "mismatched-server-tools",
    ],
)];

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
        let expected = FIXTURES_WITH_A_WARNING
            .iter()
            .find(|(fixture, _)| fixture == name)
            .map(|(_, codes)| *codes)
            .unwrap_or_default();
        if expected.is_empty() {
            assert!(
                stderr.ends_with("is valid (target `local`)\n"),
                "unexpected verdict for `{name}`: {stderr}"
            );
            continue;
        }
        assert!(
            stderr.contains(&format!(
                "is valid (target `local`), with {} warning{}\n",
                expected.len(),
                if expected.len() == 1 { "" } else { "s" }
            )),
            "unexpected verdict for `{name}`: {stderr}"
        );
        for code in expected {
            assert!(
                stderr.contains(&format!("warning[{code}]")),
                "`{name}` no longer carries the `{code}` warning it is written for:\n{stderr}"
            );
        }
    }
}

/// The list above names fixtures that exist, so a renamed one cannot leave an
/// entry excusing nothing.
#[test]
fn every_fixture_with_a_declared_warning_exists() {
    for (fixture, codes) in FIXTURES_WITH_A_WARNING {
        assert!(
            harness::FIXTURES.contains(fixture),
            "`{fixture}` is not an acceptance fixture"
        );
        assert!(
            !codes.is_empty(),
            "`{fixture}` names the warnings it carries"
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

/// A provider's `server_tools:` reach the Messages wire **verbatim**, beside the
/// agent's own tools, and a scripted use of one flows through the loop without
/// being dispatched (grammar 12.1, Decision D122, resolved q30).
///
/// Three claims in one run, and each is one a transcript can decide:
///
///  * the config the composition declared is the object on the wire, field for
///    field — the whole of what q30's pass-through promises, and the thing a
///    compiler that "supported" a tool by rewriting it would fail;
///  * the suite is **appended** to the agent's own tools rather than replacing
///    them, which is why the pinned output schema is still there;
///  * a `server_tool_use` block and the result the provider paired with it come
///    back inside the answer, and the agent completes. Nothing was dispatched:
///    the tool ran on the provider's side, and a runtime that read the block as
///    a tool call would have bounced it back as a refusal (Decision D119) and
///    spent the loop.
#[test]
fn a_providers_server_tools_reach_the_messages_wire_verbatim() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the docs say yes" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search_20250305",
                json!({ "query": "does it?" }),
                json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
            ),
        ]),
    ));

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.search",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the docs say yes");

    let asked = provider.requests();
    assert_eq!(asked.len(), 1, "one call, and its answer needed no second");
    assert!(asked[0].is_valid(), "{:?}", asked[0].failures());
    assert_eq!(asked[0].surface, Surface::Anthropic);
    assert_eq!(
        asked[0].server_tools,
        ["web_search_20250305"],
        "the provider's suite reached the request"
    );
    // Verbatim: what the spec declared, as JSON, unrewritten.
    let tools = asked[0].body()["tools"]
        .as_array()
        .expect("a tool array")
        .clone();
    assert_eq!(
        tools.last(),
        Some(&json!({
            "type": "web_search_20250305",
            "name": "web_search",
            "max_uses": 5,
            "allowed_domains": ["docs.example.com"],
        })),
        "the server tool is the config the composition wrote: {tools:?}"
    );
    assert!(
        asked[0].tools.is_empty(),
        "…and the suite is the whole of the array here: this agent declares no \
         `tools:` of its own, and its output schema rides `output_config` rather \
         than a synthetic tool (PRD resolved q53). That the suite is *appended* \
         to an agent's own tools rather than replacing them is decided by the \
         loop fixture below, which has some ({:?})",
        asked[0].tools
    );
    assert!(provider.snapshot().is_drained());
}

/// The **last** run of assistant text is the object a native structured-output
/// call is read from — not the join of every `text` block in the turn (PRD §9
/// resolved q53, Decision D122).
///
/// This composition is only reachable on the native rung, and that is why it is
/// a test rather than a hypothetical: `tool_choice: {type: "tool", name}` was a
/// promise that the pinned call's turn is one `tool_use` block, so no server
/// tool could run on it and the object came out of that block whatever prose
/// surrounded it. `output_config` pins nothing, so the pinned call of an agent
/// whose provider declares `server_tools:` is an ordinary turn — the model may
/// announce a search, run it, and answer afterwards, which is the shape a web
/// search answer most often has.
///
/// A runtime that flattened the turn's `text` blocks and parsed the join would
/// hand `JSON.parse` `Let me search.{"answer":"…"}`, read no object out of a
/// turn that carried one, and fail the node with `asked for … and the answer
/// carried no structured output`.
#[test]
fn a_native_answer_after_a_server_tool_is_read_from_the_turns_last_text() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the docs say yes" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search_20250305",
                json!({ "query": "does it?" }),
                json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
            )
            .preceded_by("Let me search."),
        ]),
    ));

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.search",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["answer"],
        "the docs say yes",
        "the preamble is prose the model wrote before the search and is not part \
         of the object"
    );

    let asked = provider.requests();
    assert_eq!(asked.len(), 1);
    assert!(asked[0].is_valid(), "{:?}", asked[0].failures());
    assert!(
        asked[0].body()["output_config"]["format"]["type"] == "json_schema",
        "…on the rung that makes the composition reachable at all: {}",
        asked[0].body_text
    );
    let mechanisms: Vec<Option<String>> = run
        .trace()
        .iter()
        .flat_map(|entry| entry["models"].as_array().cloned().unwrap_or_default())
        .map(|call| {
            call["outputMechanism"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .collect();
    assert_eq!(mechanisms, [Some("native".to_string())], "{mechanisms:?}");
    assert!(provider.snapshot().is_drained());
}

/// A `server_tool_use` block in an answer the tool loop is reading is **not**
/// dispatched, and is **not** refused (Decision D122, over Decision D119).
///
/// This is the negative half of the pass-through, and it needs a loop to be
/// visible at all: the answer carries a `server_tool_use` block beside a
/// `tool_use` one, and only the second may reach the runtime. A runtime that
/// read the first as a call would find no tool of that name on the agent and
/// bounce it back as a refusal — a `refused` record in the trace, and a wasted
/// turn — so the trace is what decides it. The block still has to *travel*: the
/// next request replays the assistant turn whole, which is where the Messages
/// API requires it back beside the `tool_use` it preceded.
#[test]
fn a_server_tool_block_is_neither_dispatched_nor_refused_by_the_loop() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search_20250305",
                    json!({ "query": "what" }),
                    json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
                )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.search_loop",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "a looked-up snippet");

    // What the loop ran, as the trace records it (`docs/trace.md` §7.3): the
    // agent's own tool, once, and nothing else. A refusal here would be the
    // server tool having been read as a call.
    let asked: Vec<(String, String)> = run
        .entries("ask")
        .iter()
        .flat_map(|entry| {
            entry["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .flat_map(|call| {
            call["toolCalls"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|record| {
            (
                record["name"].as_str().unwrap_or_default().to_string(),
                record["outcome"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();
    assert_eq!(
        asked,
        [("lookup".to_string(), "completed".to_string())],
        "the loop ran the agent's tool and nothing else: {asked:?}"
    );

    // …and the block travelled: the next request replays the assistant turn
    // whole, `server_tool_use` and its result included.
    let requests = provider.requests();
    assert_eq!(requests.len(), 3);
    // The half the tool-less fixture above cannot decide: an agent that
    // declares tools sends them, and the provider's suite is **appended** to
    // them rather than replacing them.
    assert_eq!(
        requests[0].tools,
        ["lookup"],
        "the agent's own tool leads the array: {:?}",
        requests[0].tools
    );
    assert_eq!(
        requests[0].server_tools,
        ["web_search_20250305"],
        "…and the provider's suite follows it: {:?}",
        requests[0].server_tools
    );
    let replayed = requests[1].body()["messages"][1]["content"]
        .as_array()
        .expect("the assistant turn is a block list")
        .iter()
        .map(|block| block["type"].as_str().unwrap_or_default().to_string())
        .collect::<Vec<_>>();
    assert!(
        replayed.contains(&"server_tool_use".to_string())
            && replayed.contains(&"web_search_tool_result".to_string()),
        "the answer went back as it came: {replayed:?}"
    );
    assert!(provider.snapshot().is_drained());
}

/// An `openai` provider that declares a suite speaks the **Responses** API for
/// every call it makes, and everything else about the call still works there
/// (Decision D122).
///
/// The seam is all-or-nothing per provider, so what this decides is that the
/// whole conversation moved: the route the requests arrived at, the function
/// tool the agent declares still being dispatched through its loop, and the
/// structured output still parsing — on a wire where it is `text.format` rather
/// than `response_format`.
///
/// It also pins the two keys the request does **not** carry, `store` and
/// `previous_response_id`, which three documents state as behaviour and no other
/// check can see.
#[test]
fn an_openai_provider_with_server_tools_runs_its_loop_on_the_responses_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The loop's first turn: a server search the provider ran, and a call to
        // the agent's own tool, which the runtime *does* dispatch.
        Script::new(
            GPT5,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search",
                    json!({ "type": "search", "query": "what" }),
                    json!([{ "url": "https://docs.example.com/a" }]),
                )]),
        ),
        // The turn that ends the loop: prose, no calls (grammar 5, D51).
        Script::new(GPT5, Outcome::text("I have what I need.")),
        // …and the pinned call that answers the node.
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "a looked-up snippet");

    let asked = provider.requests();
    assert_eq!(asked.len(), 3, "the loop turned twice, then answered");
    for request in &asked {
        assert!(request.is_valid(), "{:?}", request.failures());
        assert_eq!(
            request.surface,
            Surface::Responses,
            "one provider, one wire: every call it serves is a Responses call"
        );
        assert_eq!(request.path, "/v1/responses");
        assert_eq!(request.server_tools, ["web_search"]);
        // …and both settings this wire spells differently arrived spelled its
        // way (`WIRE-NOTES` (20)). `is_valid()` above is already the half that
        // catches the untranslated spelling — the key list is closed — so what
        // these pin is that the translation produced the right *value* rather
        // than dropping the knob on the floor, which a closed list cannot see.
        assert_eq!(
            request.body()["max_output_tokens"],
            json!(4096),
            "`max_tokens` is `max_output_tokens` here: {}",
            request.body_text
        );
        assert_eq!(
            request.body()["reasoning"],
            json!({ "effort": "high" }),
            "…and `reasoning_effort` is `reasoning: {{ effort }}`: {}",
            request.body_text
        );
        // …and the two keys this wire has that the emitted request deliberately
        // does **not** carry. Both are documented as absences with consequences
        // — `docs/topics/models.md` and `docs/grammar.md` D122 on retention,
        // `WIRE-NOTES` (19) on why no request this server sees holds `store` —
        // and neither is a thing the mock can decide: both are in
        // `responses::REQUEST_KEYS` and `check_settings` takes `store` as any
        // boolean, so a request that pinned either would validate. Pinning
        // `store: false` is the regression that matters, and it is reachable
        // only against the real service: it makes the API refuse a replayed
        // `reasoning` item, so every reasoning-model tool loop on a real
        // `openai` provider with a suite would break on its second turn, with
        // this suite still green.
        assert_eq!(
            request.body()["store"],
            Value::Null,
            "`store` is left to the service's default rather than pinned: {}",
            request.body_text
        );
        assert_eq!(
            request.body()["previous_response_id"],
            Value::Null,
            "…and the conversation travels in `input`, not behind a continuity \
             token the runtime does not hold (`docs/durability.md` §3.1): {}",
            request.body_text
        );
    }
    // The client tool is declared flat on this wire, and its call came back to
    // the graph — a server tool in the same array changed neither.
    assert_eq!(asked[0].tools, ["lookup"]);
    assert!(
        asked[1].body()["input"]
            .as_array()
            .expect("an input list")
            .iter()
            .any(|item| item["type"] == "function_call_output"),
        "the dispatched tool's result went back as an item: {}",
        asked[1].body_text
    );
    // …the server tool's own item is replayed beside it, and is answered by
    // nothing: the provider already ran it (Decision D122).
    assert!(
        asked[1].body()["input"]
            .as_array()
            .expect("an input list")
            .iter()
            .any(|item| item["type"] == "web_search_call"),
        "the server tool's record travelled back untouched: {}",
        asked[1].body_text
    );
    // …and structured output is `text.format` here, and still parsed.
    assert!(matches!(
        asked[2].structured_output,
        Some(StructuredOutput::JsonSchema { .. })
    ));
    assert!(provider.snapshot().is_drained());
}

/// A pinned Responses turn that carries **more than one** `message` is read at
/// the message `text.format` shaped, not at the concatenation of all of them.
///
/// A Responses turn is a **list of items**, and a model that says something
/// before its provider's search runs sends a preamble `message`, the
/// `web_search_call`, and then the shaped `message`. Joining the two texts and
/// parsing the join is `Let me look that up.{"answer":"…"}` reaching
/// `JSON.parse` — which `shapedOutput` answers as an absence, so the node
/// fails with `asked for … and the answer carried no structured output` about a
/// turn that carried one.
///
/// The Messages wire reaches the same shape by the same route, and
/// `a_native_answer_after_a_server_tool_is_read_from_the_turns_last_text` is
/// that test: `output_config` pins nothing, so a pinned call may run a server
/// tool and answer after it, in two runs of `text`. Only Chat Completions is
/// exempt, and by its shape rather than by anything this runtime does — a turn
/// there is the single string `choices[0].message.content`.
#[test]
fn a_pinned_responses_turn_is_read_at_the_message_the_format_shaped() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The turn that ends the loop: prose, no calls.
        Script::new(GPT5, Outcome::text("I have what I need.")),
        // …and the pinned call, answered the way a turn with a server tool in it
        // legitimately arrives: what the model said before the search, the
        // search itself, and the message the format shaped.
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "a searched-for snippet" })).with_server_tools(
                vec![
                    ServerToolUse::new(
                        "web_search",
                        json!({ "type": "search", "query": "what" }),
                        json!([{ "url": "https://docs.example.com/a", "title": "A" }]),
                    )
                    .preceded_by("Let me look that up."),
                ],
            ),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    // The node completed, and what it produced is the *second* message's object.
    // A runtime reading the join fails the node instead, with a parser message.
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "a searched-for snippet");
    assert_eq!(
        provider.requests().len(),
        2,
        "the loop turned once, then answered"
    );
    assert!(provider.snapshot().is_drained());
}

/// …and a pinned Responses turn that carried no parseable object at all fails
/// the node with the sentence the other wires fail it with.
///
/// The other half of the reading above. `text.format` is the service's promise
/// and not this runtime's, so a turn that came back with prose where the schema
/// was asked for is an answer the node cannot use — reported as the *absence* of
/// structured output, exactly as the Messages wire reports a turn missing its
/// pinned `tool_use` block. What that buys is a node error naming the agent, the
/// output it asked for, and what the surface said about why; a `SyntaxError`
/// raised inside `callResponses` names none of the three, and is raised inside
/// the journaled model call, so a resume would replay a recorded failure of a
/// call that had in fact answered.
#[test]
fn a_pinned_responses_turn_carrying_no_object_is_reported_as_no_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(GPT5, Outcome::text("I have what I need.")),
        Script::new(
            GPT5,
            Outcome::raw(
                200,
                json!({
                    "id": "resp_prose",
                    "object": "response",
                    "status": "completed",
                    "model": GPT5,
                    "output": [{
                        "type": "message",
                        "id": "msg_prose",
                        "status": "completed",
                        "role": "assistant",
                        "content": [{
                            "type": "output_text",
                            "text": "Sorry, I could not find anything.",
                            "annotations": [],
                        }],
                    }],
                    "incomplete_details": null,
                    "parallel_tool_calls": true,
                }),
            ),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("`agent.responder` asked for")
            && failure.contains("the answer carried no structured output"),
        "the node error names the agent and what it asked for: {failure}"
    );
    assert!(
        !failure.contains("SyntaxError"),
        "…rather than the parser's message: {failure}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A tool outside the compiler's curated table travels to the wire as written —
/// the second tier of resolved q30, end to end.
///
/// `validate` warns and exits `0`, `build` emits, and the run puts the config on
/// the request unchanged. That is the whole no-treadmill promise: a server tool
/// the vendor ships after this compiler release is usable the day it ships.
#[test]
fn a_server_tool_outside_the_table_is_warned_about_and_still_reaches_the_wire() {
    let scratch = harness::Scratch::new("server-tool-after-this-release");
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(
        &entrypoint,
        unverified_server_tool_spec("web_search_20260101"),
    )
    .expect("the scratch spec is writable");

    let validated = harness::validate_entrypoint(&entrypoint, "local");
    let stderr = String::from_utf8_lossy(&validated.stderr);
    assert_eq!(
        validated.status.code(),
        Some(0),
        "a warning does not refuse a composition:\n{stderr}"
    );
    assert!(
        stderr.contains("warning[unknown-server-tool]") && stderr.contains("web_search_20260101"),
        "the warning names exactly what could not be verified:\n{stderr}"
    );

    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "carried anyway" })),
    ));
    let Some(run) = harness::invoke_entrypoint(
        &entrypoint,
        "server-tool-after-this-release",
        "flow.search",
        &json!({ "question": "does it?" }),
        &harness::environment(&provider),
        None,
    ) else {
        return;
    };
    run.succeeded();

    let asked = provider.requests();
    assert_eq!(
        asked[0].server_tools,
        ["web_search_20260101"],
        "the unverified tool reached the wire"
    );
    assert_eq!(
        asked[0].body()["tools"]
            .as_array()
            .and_then(|tools| tools.last())
            .expect("a tool array")["max_uses"],
        7,
        "…with the config the compiler could not check, as written: {}",
        asked[0].body_text
    );
}

/// A **field** the compiler's table cannot check, on a tool it knows, travels to
/// the wire as written — the second tier one level in (Decision D122).
///
/// A row is keyed on `type:` alone and is a snapshot of that tool at this
/// compiler's release, so a parameter the vendor added since must not be a
/// build-blocking error: there is no way to opt one entry out of the strict
/// tier, and renaming the `type:` to reach the unchecked tier would change which
/// tool runs. So `validate` warns and exits `0`, the strict half of the same
/// entry is still checked, and the run puts the whole config on the request.
#[test]
fn a_server_tool_field_outside_the_table_is_warned_about_and_still_reaches_the_wire() {
    let scratch = harness::Scratch::new("server-tool-field-after-this-release");
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(
        &entrypoint,
        server_tool_spec("web_search_20250305", "\n      result_freshness: week"),
    )
    .expect("the scratch spec is writable");

    let validated = harness::validate_entrypoint(&entrypoint, "local");
    let stderr = String::from_utf8_lossy(&validated.stderr);
    assert_eq!(
        validated.status.code(),
        Some(0),
        "a warning does not refuse a composition:\n{stderr}"
    );
    assert!(
        stderr.contains("warning[unknown-server-tool-field]")
            && stderr.contains("result_freshness"),
        "the warning names exactly the key that could not be verified:\n{stderr}"
    );
    assert!(
        !stderr.contains("warning[unknown-server-tool]:"),
        "the tool itself is one the table knows, and is not warned about:\n{stderr}"
    );

    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "carried anyway" })),
    ));
    let Some(run) = harness::invoke_entrypoint(
        &entrypoint,
        "server-tool-field-after-this-release",
        "flow.search",
        &json!({ "question": "does it?" }),
        &harness::environment(&provider),
        None,
    ) else {
        return;
    };
    run.succeeded();

    let asked = provider.requests();
    assert_eq!(asked[0].server_tools, ["web_search_20250305"]);
    // The **last** entry rather than the second: the provider's suite follows
    // whatever client tools the request carries, and this agent carries none —
    // its output schema rides `output_config` (PRD resolved q53).
    let declared = asked[0].body()["tools"]
        .as_array()
        .and_then(|tools| tools.last())
        .expect("a tool array")
        .clone();
    assert_eq!(
        declared["result_freshness"], "week",
        "the key the compiler could not check reached the wire as written: {}",
        asked[0].body_text
    );
    assert_eq!(
        declared["max_uses"], 7,
        "…beside the fields of the same entry that it could: {}",
        asked[0].body_text
    );
}

/// A failover route whose members are on **two different wires** composes: each
/// attempt speaks its own provider's (Decision D122).
///
/// The first member's provider carries a suite, so its call goes to
/// `/v1/responses`; the second's does not, so its call goes to
/// `/v1/chat/completions`. A rate limit on the first is what moves the ladder,
/// and the answer comes back off a wire the first attempt never touched. The
/// differing suites are also what `validate` warns about, which
/// `the_acceptance_fixtures_validate_clean` pins.
#[test]
fn a_failover_that_crosses_two_wires_composes() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(GPT5, Outcome::rate_limit()),
        Script::new(
            GPT_MINI,
            Outcome::structured(json!({ "answer": "the fallback answered" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.cross",
        &[("question", "who answers?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the fallback answered");

    let asked = provider.requests();
    assert_eq!(
        asked
            .iter()
            .map(|request| (request.surface, request.path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Surface::Responses, "/v1/responses"),
            (Surface::OpenAi, "/v1/chat/completions"),
        ],
        "each member of the ladder spoke its own provider's wire"
    );
    assert_eq!(asked[0].server_tools, ["web_search"]);
    assert!(
        asked[1].server_tools.is_empty(),
        "the fallback's provider declares none, and none was invented for it"
    );
    assert!(provider.snapshot().is_drained());
}

/// A gateway's suite rides its **Chat Completions** `tools` array, verbatim and
/// beside the agent's own (grammar 12.1, Decision D122).
///
/// The third kind that takes the key, and the one with no curated table at all:
/// every entry on an `openai_compatible` provider is second-tier, so `validate`
/// warns and the array travels as written. That makes the runtime half the whole
/// of the promise — a suite the compiler carried through validation and codegen
/// and then dropped on the floor at the request would be the silent no-op D50
/// refuses, and would make the warning's own sentence ("its config … travels to
/// the provider as written") false.
///
/// This is also the one route of the three where the answer is not on a wire the
/// suite moved the connection to: `openai_compatible` stays on Chat Completions
/// precisely because moving a gateway onto a wire it may not implement would
/// break the compositions that work today.
#[test]
fn a_gateways_server_tools_ride_its_chat_completions_tools_array() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::structured(json!({ "answer": "the gateway searched" })),
    ));

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.gateway",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the gateway searched");

    let asked = provider.requests();
    assert_eq!(asked.len(), 1, "one call, and its answer needed no second");
    assert!(asked[0].is_valid(), "{:?}", asked[0].failures());
    assert_eq!(
        (asked[0].surface, asked[0].path.as_str()),
        (Surface::OpenAi, "/v1/chat/completions"),
        "a gateway keeps the wire it had"
    );
    assert_eq!(
        asked[0].server_tools,
        ["web_search"],
        "the suite the composition declared reached the request: {}",
        asked[0].body_text
    );
    let tools = asked[0].body()["tools"]
        .as_array()
        .expect("a tool array")
        .clone();
    assert_eq!(
        tools.last(),
        Some(&json!({ "type": "web_search", "search_context_size": "medium" })),
        "…as the config the composition wrote, unrewritten: {tools:?}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A failover that crosses the two **block-carrying** wires replays a turn the
/// member it landed on can read (Decision D122).
///
/// Both surfaces answer with a list of objects and both take their own list
/// back — a `tool_use` block is not an item type the Responses API has, and a
/// `function_call` item is not a content block the Messages API has — so a turn
/// forwarded across the seam is a 400 naming a type the member never heard of.
/// The turn is therefore *rendered* into the wire that is about to send it,
/// out of the same reading its own composed turns are rendered from.
///
/// The loop is what makes this reachable and is why the agent carries a client
/// tool: the failover has to happen on the **second** call, when there is an
/// assistant turn to replay. `a_failover_that_crosses_two_wires_composes` above
/// rate-limits the first call, where the history is one user turn and any wire
/// can render it.
#[test]
fn a_failover_off_the_responses_wire_rewrites_the_turn_for_the_messages_one() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Turn one, on the Responses wire: a search the provider ran, and a call
        // to the agent's own tool, which the runtime dispatches.
        Script::new(
            GPT5,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search",
                    json!({ "type": "search", "query": "what" }),
                    json!([{ "url": "https://docs.example.com/a" }]),
                )]),
        ),
        // …and from here the first member refuses, so both remaining calls —
        // the one that ends the loop and the pinned one that answers the node —
        // are served by the Anthropic member, each replaying a history whose
        // assistant turn came off the other wire.
        Script::new(GPT5, Outcome::rate_limit()).times(2),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "the fallback finished the loop" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.cross_from_responses",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the fallback finished the loop");

    let asked = provider.requests();
    assert_eq!(
        asked
            .iter()
            .map(|request| (request.surface, request.path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Surface::Responses, "/v1/responses"),
            (Surface::Responses, "/v1/responses"),
            (Surface::Anthropic, "/v1/messages"),
            (Surface::Responses, "/v1/responses"),
            (Surface::Anthropic, "/v1/messages"),
        ],
        "each call starts at the head of the ladder and falls to the member that answers"
    );

    // The one this test exists for: the first Messages request, whose history
    // holds a turn the Responses wire produced.
    let replayed = &asked[2];
    assert!(
        replayed.is_valid(),
        "the replayed turn is a Messages turn: {:?}\n{}",
        replayed.failures(),
        replayed.body_text
    );
    assert_eq!(
        block_types(
            replayed.body()["messages"]
                .as_array()
                .expect("a message list")
        ),
        [
            vec!["<user>".to_string()],
            vec!["tool_use".to_string()],
            vec!["tool_result".to_string()],
        ],
        "the answer was rendered into this wire's vocabulary rather than forwarded: {}",
        replayed.body_text
    );
    assert!(
        !replayed.body_text.contains("web_search_call"),
        "…and the Responses item that has no Messages spelling did not travel: {}",
        replayed.body_text
    );
    assert!(provider.snapshot().is_drained());
}

/// A Responses turn whose **whole** content is items the Messages wire cannot
/// spell is dropped from the replay rather than sent as an empty message
/// (Decision D122).
///
/// The turn above carried a `tool_use` the rebuild had something to render. This
/// one carries nothing: the provider ran its search and the model said no words,
/// which is also the shape an answer cut short by `max_output_tokens` takes. The
/// turn survives `replayed` — it has items, so the model did answer — and then
/// reaches a member on the other wire, where `text` is `""` and there are no
/// tool calls, so the rebuild has no block to emit.
///
/// `{"role": "assistant", "content": []}` is what a rebuild that emitted the
/// turn anyway would send, and the Messages API refuses it (`content: List
/// should have at least 1 item`) — so the node would die on a 400 about the
/// *request*, one call after the model's empty answer, and `rate_limit` is not a
/// status a `route_on:` catches. Dropping the turn keeps the conversation legal.
///
/// It is also the one place two user turns can meet, which is why the assertion
/// is on blocks rather than on a message count. Resolved q52 closes the tool
/// exchange with a fixed user turn before the pinned call, and here the assistant
/// turn between it and the opening user turn is the one being dropped — so the
/// two are folded into a single user message, in the order they were written.
/// Left as two messages they would break the alternation this wire requires, and
/// the node would die on the 400 the drop exists to avoid.
///
/// The fold is conditioned on that drop and on nothing else, which this run is
/// the only witness to: every *other* way two user turns could meet is a bug, and
/// a fold that repaired them all would spend this server's alternation check —
/// the one thing that can still see them — to hide it.
#[test]
fn a_responses_turn_with_nothing_the_messages_wire_can_spell_is_not_replayed_empty() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Turn one, on the Responses wire: a search the provider ran, and not one
        // word of prose beside it. No tool call, so the loop ends here.
        Script::new(
            GPT5,
            Outcome::text("").with_server_tools(vec![ServerToolUse::new(
                "web_search",
                json!({ "type": "search", "query": "what" }),
                json!([{ "url": "https://docs.example.com/a" }]),
            )]),
        ),
        // The pinned call that answers the node falls to the Anthropic member,
        // replaying a history whose only assistant turn is that one.
        Script::new(GPT5, Outcome::rate_limit()),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "the fallback answered anyway" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.cross_from_responses",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the fallback answered anyway");

    let asked = provider.requests();
    assert_eq!(
        asked
            .iter()
            .map(|request| (request.surface, request.path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Surface::Responses, "/v1/responses"),
            (Surface::Responses, "/v1/responses"),
            (Surface::Anthropic, "/v1/messages"),
        ],
        "the loop ended on the first answer, and the pinned call fell to the other wire"
    );

    let replayed = &asked[2];
    assert!(
        replayed.is_valid(),
        "an empty assistant message would be refused here: {:?}\n{}",
        replayed.failures(),
        replayed.body_text
    );
    let messages = replayed.body()["messages"]
        .as_array()
        .expect("a message list")
        .clone();
    assert_eq!(
        block_types(&messages),
        [vec!["text".to_string(), "text".to_string()]],
        "the unspellable turn was dropped, not padded and not emptied: {}",
        replayed.body_text
    );
    assert_eq!(
        messages[0]["content"][0]["text"], "{\"question\":\"what?\"}",
        "the node's own input still opens the conversation: {}",
        replayed.body_text
    );
    assert_eq!(
        messages[0]["content"][1]["text"], "Now produce the structured result.",
        "…and q52's closing turn was folded into it rather than left to break the \
         alternation the drop exposed: {}",
        replayed.body_text
    );
    assert!(provider.snapshot().is_drained());
}

/// The same seam, crossed the other way: a Messages answer replayed to the
/// Responses wire (Decision D122).
///
/// The mirror is worth its own run because the two renderings are two pieces of
/// code and the failure is asymmetric: a `tool_use` block sent to `/v1/responses`
/// is refused by a closed item-type list, and a `function_call` item sent to
/// `/v1/messages` is refused by a closed block-type list *and* leaves a
/// `tool_result` answering a `tool_use` nothing asked for.
#[test]
fn a_failover_off_the_messages_wire_rewrites_the_turn_for_the_responses_one() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search_20250305",
                    json!({ "query": "what" }),
                    json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
                )]),
        ),
        Script::new(SONNET, Outcome::rate_limit()).times(2),
        Script::new(GPT5, Outcome::text("I have what I need.")),
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "the fallback finished the loop" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.cross_from_messages",
        &[("question", "what?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the fallback finished the loop");

    let asked = provider.requests();
    assert_eq!(
        asked
            .iter()
            .map(|request| (request.surface, request.path.as_str()))
            .collect::<Vec<_>>(),
        [
            (Surface::Anthropic, "/v1/messages"),
            (Surface::Anthropic, "/v1/messages"),
            (Surface::Responses, "/v1/responses"),
            (Surface::Anthropic, "/v1/messages"),
            (Surface::Responses, "/v1/responses"),
        ],
        "each call starts at the head of the ladder and falls to the member that answers"
    );

    let replayed = &asked[2];
    assert!(
        replayed.is_valid(),
        "the replayed turn is a Responses input list: {:?}\n{}",
        replayed.failures(),
        replayed.body_text
    );
    assert_eq!(
        replayed.body()["input"]
            .as_array()
            .expect("an input list")
            .iter()
            .map(|item| item["type"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["message", "function_call", "function_call_output"],
        "the answer was rendered into this wire's vocabulary rather than forwarded: {}",
        replayed.body_text
    );
    assert!(
        !replayed.body_text.contains("server_tool_use"),
        "…and the Messages block that has no Responses spelling did not travel: {}",
        replayed.body_text
    );
    assert!(provider.snapshot().is_drained());
}

/// The `type` of every content block of every Messages-wire message, in order.
/// A message whose `content` is a bare string — which is what a user turn is —
/// reports as `<user>`, since it has no blocks to name.
fn block_types(messages: &[Value]) -> Vec<Vec<String>> {
    messages
        .iter()
        .map(|message| match message["content"].as_array() {
            None => vec!["<user>".to_string()],
            Some(blocks) => blocks
                .iter()
                .map(|block| block["type"].as_str().unwrap_or_default().to_string())
                .collect(),
        })
        .collect()
}

/// `server_tools:` on a kind whose wire this release has not been taught is an
/// **error**, and the message says which kinds carry it.
///
/// The other half of the two-tier design, and the one that is not a warning: a
/// suite declared here would be dropped on the floor rather than merely
/// unverified, which is the silent no-op D50 refuses everywhere else.
#[test]
fn server_tools_on_a_kind_whose_wire_has_none_is_refused_by_name() {
    let scratch = harness::Scratch::new("server-tools-on-azure");
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(
        &entrypoint,
        "version: \"0.1\"\n\nprovider.azure:\n  kind: azure_openai\n  base_url: ${AZURE_ENDPOINT}\n  \
         api_key: ${AZURE_API_KEY}\n  api_version: \"2024-10-21\"\n  server_tools:\n    - type: web_search\n",
    )
    .expect("the scratch spec is writable");

    let validated = harness::validate_entrypoint(&entrypoint, "local");
    let stderr = String::from_utf8_lossy(&validated.stderr);
    assert_eq!(validated.status.code(), Some(1), "{stderr}");
    assert!(
        stderr.contains(
            "error[unsupported-server-tools]: provider definition `provider.azure` declares \
             `kind: azure_openai`, whose wire this compiler release does not carry server tools on"
        ),
        "the refusal names the kind:\n{stderr}"
    );
    assert!(
        stderr.contains(
            "server tools launched on `kind: anthropic`, `kind: openai`, `kind: openai_compatible`"
        ),
        "…and the launch scope:\n{stderr}"
    );
}

/// One spec whose provider declares a server tool the compiler's table does not
/// name.
fn unverified_server_tool_spec(type_name: &str) -> String {
    server_tool_spec(type_name, "")
}

/// The same spec with one server tool of `type_name`, carrying `extra` as a
/// further line of that entry's config.
///
/// The two callers are the two granularities the second tier is reached at: an
/// unknown `type:`, and a key the row of a known one does not name.
fn server_tool_spec(type_name: &str, extra: &str) -> String {
    format!(
        r#"version: "0.1"

state:
  answer:
    type: string
    default: nothing yet

provider.searching:
  kind: anthropic
  api_key: ${{MOCK_API_KEY}}
  base_url: ${{MOCK_BASE_URL}}
  server_tools:
    - type: {type_name}
      name: web_search
      max_uses: 7{extra}

model.smart:
  provider: provider.searching
  id: {SONNET}

agent.searcher:
  description: Answers with whatever its provider found.
  model: model.smart
  prompt: |
    You are a researcher. Answer the question.
  input:
    question:
      description: What to answer.
      type: string
  output:
    answer:
      description: The answer, as text.
      type: string

flow.search:
  description: One agent whose provider searches for it.
  inputs:
    question:
      type: string
      min_length: 1
  outputs:
    answer:
      type: string
  nodes:
    ask:
      agent: agent.searcher
      input:
        question: "input.question"
  edges:
    - {{ from: start, to: ask }}
    - {{ from: ask, to: end }}
"#
    )
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
/// module header's *How many model calls an agent node makes* fixes: one call,
/// asking for the output schema in whichever way this wire and this endpoint
/// leave open (PRD resolved q53). What an agent's own `tools:` list looks like
/// on the wire belongs to the tool-loop test — a request that *pins* the output
/// tool cannot leave another one callable, so no single test can assert both.
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
        structured.mechanism(),
        OutputMechanism::Native,
        "the Messages wire's own structured output is the rung a call starts on, \
         and this endpoint takes it (PRD resolved q53): {structured:?}"
    );
    assert!(
        call.tools.is_empty(),
        "…so a tool-less agent offers no tools at all: the schema rides \
         `output_config`, and the synthetic output tool is composed only where \
         the other mechanism pins it ({:?})",
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

/// A provider that declares no `api_key:` sends **no** authentication header,
/// on either vendor wire — and one that declares a key still sends it.
///
/// Grammar 12.1 makes the key conditional on the two kinds with a default
/// endpoint (Decision D120): a `base_url:` names a gateway that injects the
/// vendor credential server-side, so the compiled graph must send none. "None"
/// is the load-bearing word. `x-api-key: ""` and `authorization: Bearer ` are
/// requests that claim to authenticate and fail, which a gateway may refuse
/// before injecting anything and which a header-forwarding gateway turns into a
/// 401 at the vendor — and both spellings are exactly what a `?? ""` in the
/// emitted runtime produces, which is what the code said before this rule
/// landed. Nothing about a flow's outputs can tell the two apart: the header is
/// only visible in the transcript, which is why this is an acceptance test and
/// not a unit test about a string.
///
/// The first run carries the other half of that rule, which is the half a
/// deployment actually runs: dropping the vendor credential is what makes room
/// for the gateway's *own* token, and `headers:` is where every account of D120
/// — grammar 12.1, `explain missing-credential`, `docs/topics/models.md` — says
/// that token goes. So `provider.claude_gateway` declares one, and the recorded
/// request is read for both facts at once: no `x-api-key`, and the declared
/// `authorization` present with the value the composition interpolated. They
/// are different env vars (`MOCK_GATEWAY_TOKEN` is not `MOCK_API_KEY`) so the
/// assertion names which layer the header came from. Without this, the whole
/// documented repair would be proved only by the mock's willingness to *serve*
/// that shape (`anthropic_wire.rs`'s
/// `an_empty_api_key_is_refused_while_a_gateway_token_is_served`), never by the
/// emitter's producing it.
///
/// The third run is the kind D120 **did not** move, and it is the reason the
/// rule is stated over the connection rather than over the row.
/// `openai_compatible` paired an optional `api_key:` with a required `base_url:`
/// before the decision and still does, so nothing in grammar 12.1's table
/// changed for it — and yet a keyless one used to reach the wire holding
/// `authorization: Bearer `, because the emitter's `?? ""` was written per wire
/// and not per kind. `docs/topics/models.md` says in so many words that this
/// kind now sends no `Authorization` header either; this run is what makes that
/// sentence true rather than merely written, on the one row whose grammar gives
/// no other reason to look.
///
/// The last run is the other direction, and it is not decoration: it is the
/// only place the suite asserts that the Messages surface sends its credential
/// when the composition has one. The mock no longer requires it
/// (`crates/mock-provider/WIRE-NOTES.md` (12) — a keyless request is a legal
/// wire shape, so the server cannot be what enforces this), so an emitter that
/// dropped the header for *every* provider would pass every other test in this
/// file.
#[test]
fn a_provider_with_no_key_sends_no_authentication_header_on_either_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ),
        Script::new(
            OPENAI_DIRECT,
            Outcome::structured(json!({ "summary": "the reviewer asked for one change" })),
        ),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "summary": "one change, and the endpoint holds no key" })),
        ),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        ),
    ]);

    let Some(messages) = harness::invoke(
        "keyless-gateway",
        "flow.messages",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    messages.succeeded();
    assert_eq!(messages.outputs()["verdict"], "revise");

    let chat = harness::invoke(
        "keyless-gateway",
        "flow.chat",
        &[("notes", "tighten it")],
        &provider,
    )
    .expect("the toolchain was there a moment ago");
    chat.succeeded();
    assert_eq!(
        chat.outputs()["summary"],
        "the reviewer asked for one change"
    );

    let compatible = harness::invoke(
        "keyless-gateway",
        "flow.compatible",
        &[("notes", "tighten it")],
        &provider,
    )
    .expect("the toolchain was there a moment ago");
    compatible.succeeded();
    assert_eq!(
        compatible.outputs()["summary"],
        "one change, and the endpoint holds no key"
    );

    let keyed = harness::invoke(
        "keyless-gateway",
        "flow.keyed",
        &[("goal", "ship it")],
        &provider,
    )
    .expect("the toolchain was there a moment ago");
    keyed.succeeded();
    assert_eq!(keyed.outputs()["verdict"], "approve");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 4, "one agent node each");

    let keyless_messages = &recorded[0];
    assert!(
        keyless_messages.is_valid(),
        "{:?}",
        keyless_messages.failures()
    );
    assert_eq!(keyless_messages.surface, Surface::Anthropic);
    assert_eq!(keyless_messages.model, SONNET);
    assert!(
        !keyless_messages.headers.contains_key("x-api-key"),
        "a keyless `anthropic` provider sends no credential, not an empty one: {:?}",
        keyless_messages.headers
    );
    assert_eq!(
        keyless_messages.headers["authorization"], "Bearer mock-gateway-token",
        "…and the token its `headers:` declares is what authenticates it instead"
    );
    assert_eq!(
        keyless_messages.headers["anthropic-version"], "2023-06-01",
        "…while the headers that are not credentials are unaffected"
    );

    let keyless_chat = &recorded[1];
    assert!(keyless_chat.is_valid(), "{:?}", keyless_chat.failures());
    assert_eq!(keyless_chat.surface, Surface::OpenAi);
    assert_eq!(keyless_chat.model, OPENAI_DIRECT);
    assert_eq!(keyless_chat.path, "/v1/chat/completions");
    assert!(
        !keyless_chat.headers.contains_key("authorization"),
        "a keyless `openai` provider sends no bearer token, not an empty one: {:?}",
        keyless_chat.headers
    );
    assert!(
        !keyless_chat.headers.contains_key("api-key"),
        "…and not the Azure spelling either: {:?}",
        keyless_chat.headers
    );

    let keyless_compatible = &recorded[2];
    assert!(
        keyless_compatible.is_valid(),
        "{:?}",
        keyless_compatible.failures()
    );
    assert_eq!(keyless_compatible.surface, Surface::OpenAi);
    assert_eq!(keyless_compatible.model, LOCAL);
    assert!(
        !keyless_compatible.headers.contains_key("authorization"),
        "a keyless `openai_compatible` provider sends no header either — the rule \
         is over the connection, not over the kind's row: {:?}",
        keyless_compatible.headers
    );

    let keyed_messages = &recorded[3];
    assert!(keyed_messages.is_valid(), "{:?}", keyed_messages.failures());
    assert_eq!(keyed_messages.surface, Surface::Anthropic);
    assert_eq!(keyed_messages.model, HAIKU);
    assert_eq!(
        keyed_messages.headers["x-api-key"], "mock-provider-key",
        "a provider that declares `api_key:` still sends it"
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

/// The pinned structured-output call after a tool loop **ends on a user turn**,
/// on both wires, and the turn that closes it never becomes conversation
/// (PRD §9 resolved q52).
///
/// The loop exits on the model's own answer, so before q52 the pinned request
/// carried a history ending on an *assistant* turn — which on the Messages wire
/// is the prefill feature, and prefill under `tool_choice: {type: "tool", …}` is
/// a contradiction: "carry on from here" against "your answer must be this call".
/// `api.anthropic.com` tolerates it and strict Anthropic-compatible gateways
/// answer 400, which is how the shape reached a live 0.6.0 deployment with every
/// test green. `crates/mock-provider` now refuses it as the gateway does, so
/// `is_valid()` below is the regression's tripwire; what this test adds is the
/// *positive* half, which no refusal can state — the exact turn that closes the
/// exchange, and where it does and does not appear.
///
/// The second node is why the flow has two: the synthetic turn belongs to one
/// request, and the only place the conversation itself is visible is the request
/// the **next** agent node makes (grammar 10.4, and see
/// `an_agent_nodes_exchange_reaches_the_next_agent_nodes_request`). What that
/// half pins is the *channel*: the conversation an agent node contributes is
/// composed from its rendered input and its structured answer (`MessageLike`),
/// so this asserts that the exchange the closing turn was composed onto is not
/// among what crosses.
///
/// It does **not** pin the composition — a runtime that appended the turn to the
/// loop's own list instead of to a copy would send the identical request and
/// hand the next node the identical history, because nothing downstream reads
/// that list. That is guarded one layer down and by a different gate: `turns` is
/// `readonly Turn[]` in `callAgent`, so appending to it is a type error, and
/// `generated_code_gates::every_generated_project_type_checks_under_the_pinned_toolchain`
/// is what fails on it.
#[test]
fn the_pinned_call_after_a_tool_loop_ends_on_the_turn_that_closes_it() {
    const CLOSING: &str = "Now produce the structured result.";

    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The loop: one tool call, then the answer in prose that ends it.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        ),
        Script::new(SONNET, Outcome::text("found it")),
        // The pinned call, and then the second node, which has no tools of its
        // own and so makes exactly one call.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "tightened" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.research_pair",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["verdict"],
        "approve",
        "the run answered with the structured verdict: {}",
        run.outputs()
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        4,
        "two loop calls, the pinned one, then the second node"
    );
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "every request is one a strict gateway would accept: {:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );

    // The loop's own calls pin nothing and are left exactly as they were: the
    // closing turn is composed for the request that forces the output tool, not
    // for every call the node makes.
    for (ordinal, request) in recorded[..2].iter().enumerate() {
        assert_eq!(
            request.structured_output, None,
            "loop call {ordinal} pins no tool: {}",
            request.body_text
        );
        assert!(
            !request.body_text.contains(CLOSING),
            "…and carries no closing turn: {}",
            request.body_text
        );
    }

    let pinned = recorded[2].body();
    assert_eq!(
        recorded[2]
            .structured_output
            .as_ref()
            .map(StructuredOutput::mechanism),
        Some(OutputMechanism::Native),
        "the third call is the pinned one, asking through the rung this wire \
         starts on (PRD resolved q53): {pinned}"
    );
    let messages = pinned["messages"].as_array().expect("a message list");
    assert_eq!(
        messages
            .iter()
            .map(|message| message["role"].as_str().unwrap_or_default())
            .collect::<Vec<_>>(),
        ["user", "assistant", "user", "assistant", "user"],
        "the input, the call, its result, the answer that ended the loop, and \
         the turn that closes it: {pinned}"
    );
    let last = messages.last().expect("a last message");
    assert_eq!(
        last["content"], CLOSING,
        "the forced call is asked for by a user turn, verbatim: {pinned}"
    );

    // …and the conversation the next node inherits is the exchange alone.
    let next = recorded[3].body();
    let inherited = next["messages"].as_array().expect("a message list");
    assert_eq!(
        inherited.len(),
        3,
        "the first node's exchange and the second node's own turn, and nothing \
         else: {next}"
    );
    assert_eq!(
        inherited[0]["content"], "{\"goal\":\"ship it\"}",
        "what the looping node was asked: {next}"
    );
    assert_eq!(
        inherited[1]["content"][0]["text"], "{\"feedback\":\"a looked-up snippet\"}",
        "…and what it answered — not a turn of its loop: {next}"
    );
    assert!(
        !recorded[3].body_text.contains(CLOSING),
        "the closing turn is one request's shape and never the conversation's: {next}"
    );
    assert!(provider.snapshot().is_drained());

    // The same close on the Chat Completions wire, where the pinned call asks
    // for its shape with `response_format` rather than a forced tool: the turn
    // is appended above the wire, so both surfaces carry it.
    let other = MockProvider::start().expect("a loopback port");
    other.enqueue_all([
        Script::new(
            LOCAL,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        ),
        Script::new(LOCAL, Outcome::text("found it")),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);
    let across = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &other,
    )
    .expect("the toolchain was there a moment ago");
    across.succeeded();

    let asked = other.requests();
    assert_eq!(asked.len(), 3, "two loop calls, then the pinned one");
    assert!(asked.iter().all(RecordedRequest::is_valid));
    let pinned = asked[2].body();
    assert!(
        pinned["response_format"].is_object(),
        "the third call is the pinned one on this wire: {pinned}"
    );
    // The closing turn is composed above the wire and above the mechanism, so
    // it is on the *other* rung too — decided by
    // `the_closing_turn_precedes_the_pinned_call_on_both_mechanisms` (PRD
    // resolved q52 under resolved q53).
    let last = pinned["messages"]
        .as_array()
        .and_then(|messages| messages.last())
        .expect("a last message")
        .clone();
    assert_eq!(last["role"], "user", "and it ends on the user: {pinned}");
    assert_eq!(last["content"], CLOSING, "with the same text: {pinned}");
    assert!(other.snapshot().is_drained());
}

/// The mechanism a structured-output call answers through is recorded, and on
/// an endpoint that carries both it is the wire's **own** parameter (PRD §9
/// resolved q53, `docs/trace.md` §7).
///
/// Two agents in one run, because the two shapes of agent node reach the pinned
/// call differently and both have to be on the same rung: `agent.researcher`
/// spends a tool loop first, `agent.reviewer` makes one call and nothing else.
/// The loop's own calls are the negative half — they ask for no object, so they
/// carry no mechanism at all, and a runtime that stamped every call would be
/// caught here rather than by a reader wondering what mechanism a tool call had.
#[test]
fn a_structured_output_call_records_the_mechanism_that_answered_it() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        ),
        Script::new(SONNET, Outcome::text("found it")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "tightened" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.research_pair",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let mechanisms: Vec<Option<String>> = run
        .trace()
        .iter()
        .flat_map(|entry| {
            entry["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|call| {
            call["outputMechanism"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .collect();
    assert_eq!(
        mechanisms,
        [
            None,
            None,
            Some("native".to_string()),
            Some("native".to_string())
        ],
        "the two loop calls asked for no object and name no mechanism; the two \
         pinned calls answered through the wire's own one: {mechanisms:?}"
    );

    let recorded = provider.requests();
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert!(
        recorded[2].body()["output_config"]["format"]["type"] == "json_schema"
            && recorded[2].body()["tool_choice"].is_null(),
        "…which on this wire is `output_config`, and no pin beside it: {}",
        recorded[2].body_text
    );
    assert!(provider.snapshot().is_drained());
}

/// A gateway that has never heard of the native parameter is laddered past
/// **inside the same call**, and what worked is remembered for the rest of the
/// process (PRD §9 resolved q53).
///
/// `flow.pair` is two agent nodes over one model, which is the smallest run in
/// which memoization is observable at all: the first node's pinned call pays the
/// discovery, and the second must start where the first ended. The mock's
/// request log is the evidence — a refusal it records took nothing from a queue,
/// so a run that never laddered would leave a scripted answer behind and
/// `is_drained` would say so.
#[test]
fn a_native_refusing_endpoint_is_laddered_past_once_and_remembered() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::NativeRejected);
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
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
    assert_eq!(
        run.outputs()["verdict"],
        "approve",
        "the run finished on the rung this endpoint has: {}",
        run.outputs()
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| (
                request.unsupported,
                request
                    .structured_output
                    .as_ref()
                    .map(StructuredOutput::mechanism)
            ))
            .collect::<Vec<_>>(),
        [
            (Some(OutputMechanism::Native), Some(OutputMechanism::Native)),
            (None, Some(OutputMechanism::ForcedTool)),
            (None, Some(OutputMechanism::ForcedTool)),
        ],
        "the first node's call was refused on the native rung and answered on \
         the other; the **second** node's went straight to the one that works, \
         which is the memo and not a second discovery"
    );
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "every one of them was a well-formed request: {:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );

    let mechanisms: Vec<Option<String>> = run
        .trace()
        .iter()
        .flat_map(|entry| {
            entry["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .map(|call| {
            call["outputMechanism"]
                .as_str()
                .map(std::string::ToString::to_string)
        })
        .collect();
    assert_eq!(
        mechanisms,
        [
            Some("forced_tool".to_string()),
            Some("forced_tool".to_string())
        ],
        "and the trace says which one answered, on both nodes: {mechanisms:?}"
    );
    assert!(
        provider.snapshot().is_drained(),
        "a run that laddered exactly as it was asked to is a clean run: {:?}",
        provider.snapshot()
    );
}

/// The mirror image: an output schema the native format could not close starts
/// on the **forced tool**, and a model generation that has removed forced tool
/// use ladders it back to the native parameter (PRD §9 resolved q53).
///
/// Two claims in one run, and neither is reachable without the other. The
/// starting rung is a fact about the *schema*: `agent.loose_reviewer`'s output
/// nests an object with an `optional:` property, so the closed shape a native
/// format's decoder compiles is not available and the ladder starts on the rung
/// that asks for no such thing. Which is exactly what makes the newest
/// generation's endpoint — the one that answers a forced `tool_choice` with a
/// 400 — the mirror of the gateway above.
#[test]
fn an_output_schema_the_native_format_could_not_close_starts_on_the_forced_tool() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::ForcedToolRemoved);
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(
                json!({ "verdict": "revise", "detail": { "headline": "it slips" } }),
            ),
        ),
        Script::new(
            SONNET,
            Outcome::structured(
                json!({ "verdict": "approve", "detail": { "headline": "it holds" } }),
            ),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.loose_review",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["verdict"], "approve");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| (
                request.unsupported,
                request
                    .structured_output
                    .as_ref()
                    .map(StructuredOutput::mechanism)
            ))
            .collect::<Vec<_>>(),
        [
            (
                Some(OutputMechanism::ForcedTool),
                Some(OutputMechanism::ForcedTool)
            ),
            (None, Some(OutputMechanism::Native)),
            (None, Some(OutputMechanism::Native)),
        ],
        "the schema decided the first node's first rung and the endpoint decided \
         its second; the **second** node started on what worked, which is the \
         memo overriding a preference the schema had otherwise fixed"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));

    for node in ["review", "review_again"] {
        let entries = run.entries(node);
        assert_eq!(
            entries[0]["models"][0]["outputMechanism"], "native",
            "and the trace names the one that answered rather than the one that \
             was tried first, on `{node}`: {}",
            entries[0]
        );
    }
    assert!(provider.snapshot().is_drained());
}

/// Which rung an agent starts on is a property of **its own schema**, not of
/// the model its composition shares (PRD §9 resolved q53).
///
/// The reason the memo records what an endpoint **refused** rather than what
/// answered: one composition's agents do not all carry the same schema, and a
/// memo holding the winner would put the second agent here on the rung the first
/// one's schema earned — and have it refused for a reason the ladder does not
/// move on. Nothing fails in this run and nothing ladders; what it decides is
/// that two calls one after the other to one endpoint asked in two different
/// ways, because their schemas differ.
#[test]
fn two_agents_on_one_model_ask_the_way_their_own_schemas_allow() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(
                json!({ "verdict": "approve", "detail": { "headline": "it holds" } }),
            ),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.mixed_review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request
                .structured_output
                .as_ref()
                .map(StructuredOutput::mechanism))
            .collect::<Vec<_>>(),
        [
            Some(OutputMechanism::Native),
            Some(OutputMechanism::ForcedTool)
        ],
        "the closed schema rode the wire's own format; the one with an \
         `optional:` property inside it could not, and asked the other way"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…and neither of them laddered: this endpoint carries both"
    );
    assert_eq!(
        run.entries("review")[0]["models"][0]["outputMechanism"],
        "native"
    );
    assert_eq!(
        run.entries("loose_review")[0]["models"][0]["outputMechanism"],
        "forced_tool"
    );
    assert!(provider.snapshot().is_drained());
}

/// An endpoint that carries **neither** mechanism fails the run with both
/// refusals quoted, because there is no knob (PRD §9 resolved q53).
#[test]
fn an_endpoint_carrying_neither_mechanism_fails_with_both_refusals_quoted() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::BothRejected);
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("output_config: Extra inputs are not permitted"),
        "the first refusal is quoted verbatim: {failure}"
    );
    assert!(
        failure.contains(
            "tool_choice: type \\\"tool\\\" and \\\"any\\\" are not supported for this model."
        ),
        "…and so is the second, quoted out of the JSON body the endpoint sent \
         rather than paraphrased: {failure}"
    );
    assert!(
        failure.contains("nothing to configure"),
        "…and the diagnostic says there is no setting to reach for, because \
         there is not one: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2, "both rungs were tried, and only both");
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.unsupported)
            .collect::<Vec<_>>(),
        [
            Some(OutputMechanism::Native),
            Some(OutputMechanism::ForcedTool)
        ],
    );
    assert_eq!(
        provider.snapshot().queues[SONNET],
        1,
        "and the scripted answer is untouched: nothing answered this node"
    );
}

/// A refusal that is **not** about the mechanism does not ladder — it fails the
/// call exactly as it did before the ladder existed (PRD §9 resolved q53).
///
/// **Every wire**, because the recognizer is per wire: each reads its own key
/// out of its own service's dialect, so a table widened on one of them is a
/// regression the other two's tests would never see. Four shapes, one per way
/// of being wrong:
///
///   * a credential — the family that has nothing to do with the mechanism at
///     all, and the one every wire has to leave alone;
///   * the schema *inside* the parameter — the shape a careless recognizer gets
///     wrong, because a schema the decoder will not compile is refused in a
///     sentence that names the mechanism's own parameter *and* says "not
///     supported". Laddering on it would send the same schema at the other rung,
///     be refused again, and report an endpoint that carries neither when what
///     the operator has is a schema to fix;
///   * a **setting** the model does not take, refused "in this context" — the
///     one that decides the recognizer reads its keys as keys. Responses spells
///     its native mechanism `text.format`, and a body about `store` carries the
///     letters `text` inside the word `context`: a substring test alone answers
///     that a wire parameter is missing, spends a request finding out otherwise,
///     and leaves `native` recorded as refused for the pairing for the life of
///     the process;
///   * a key **inside** the parameter — the one whose cost is not a wasted
///     request. Each of these is a codegen slip that puts one wire's nesting
///     inside another's parameter, refused at the path it sits on: a service
///     that walked into `output_config.format` to complain has `output_config`.
///     A recognizer reading the mention rather than the path ladders, the
///     forced-tool rung is well formed, the run **succeeds**, and the malformed
///     request ships behind a memoized refusal — the one way of being wrong that
///     hides itself.
#[test]
fn a_refusal_that_is_not_about_the_mechanism_does_not_ladder() {
    // One run, and the claim is always the same: the request count did not grow.
    // `before` is what the node spent before the pinned call — the loop turn a
    // tool-carrying agent takes first — so the count is "the refused call, and
    // nothing after it".
    let unladdered = |project: &str,
                      flow: &str,
                      inputs: &[(&str, &str)],
                      model: &str,
                      before: &[Outcome],
                      about: &str,
                      body: Value| {
        let provider = MockProvider::start().expect("a loopback port");
        for outcome in before {
            provider.enqueue(Script::new(model, outcome.clone()));
        }
        provider.enqueue(Script::new(model, Outcome::raw(400, body)));

        let Some(run) = harness::invoke(project, flow, inputs, &provider) else {
            return;
        };
        run.failed();
        assert_eq!(
            provider.requests().len(),
            before.len() + 1,
            "on `{project}`, a 400 about {about} is not a mechanism this endpoint \
             lacks, so the call ended where it always did — no retry"
        );
    };

    let review = &[("goal", "ship it"), ("draft", "a draft")][..];
    for (project, flow, inputs, model, before, cases) in [
        (
            "agent-anthropic",
            "flow.review",
            review,
            SONNET,
            &[][..],
            [
                (
                    "a credential",
                    json!({
                        "type": "error",
                        "error": { "type": "authentication_error", "message": "invalid x-api-key" },
                    }),
                ),
                (
                    "the schema inside the parameter",
                    json!({
                        "type": "error",
                        "error": {
                            "type": "invalid_request_error",
                            "message": "output_config.format.schema: Invalid schema for output_config.format: 'minimum' is not supported.",
                        },
                    }),
                ),
                (
                    "a setting this model does not take",
                    json!({
                        "type": "error",
                        "error": {
                            "type": "invalid_request_error",
                            "message": "This model does not support 'thinking' in this context.",
                        },
                    }),
                ),
                (
                    "a key one level inside the parameter",
                    json!({
                        "type": "error",
                        "error": {
                            "type": "invalid_request_error",
                            "message": "output_config.format.name: Extra inputs are not permitted",
                        },
                    }),
                ),
            ],
        ),
        (
            "agent-openai",
            "flow.review",
            review,
            LOCAL,
            &[][..],
            [
                (
                    "a credential",
                    json!({
                        "error": {
                            "message": "Incorrect API key provided.",
                            "type": "invalid_request_error",
                            "code": "invalid_api_key",
                        },
                    }),
                ),
                (
                    "the schema inside the parameter",
                    json!({
                        "error": {
                            "message": "Invalid schema for response_format 'reviewer_output': 'minimum' is not supported.",
                            "type": "invalid_request_error",
                            "param": "response_format",
                        },
                    }),
                ),
                (
                    "a setting this model does not take",
                    json!({
                        "error": {
                            "message": "This model does not support 'store: false' in this context.",
                            "type": "invalid_request_error",
                            "param": "store",
                        },
                    }),
                ),
                (
                    "a key one level inside the parameter",
                    json!({
                        "error": {
                            "message": "Unrecognized request argument supplied: response_format.json_schema.format",
                            "type": "invalid_request_error",
                            "param": "response_format.json_schema.format",
                        },
                    }),
                ),
            ],
        ),
        (
            "server-tools",
            "flow.respond",
            &[("question", "does it?")][..],
            GPT5,
            // The loop's own turn: prose and no calls, so the next request is the
            // pinned one this is about (grammar 5, D51).
            &[Outcome::text("I have what I need.")][..],
            [
                (
                    "a credential",
                    json!({
                        "error": {
                            "message": "Incorrect API key provided.",
                            "type": "invalid_request_error",
                            "code": "invalid_api_key",
                        },
                    }),
                ),
                (
                    "the schema inside the parameter",
                    json!({
                        "error": {
                            "message": "Invalid schema for 'text.format': 'minimum' is not supported.",
                            "type": "invalid_request_error",
                            "param": "text.format",
                        },
                    }),
                ),
                (
                    "a setting this model does not take",
                    json!({
                        "error": {
                            "message": "This model does not support 'store: false' in this context.",
                            "type": "invalid_request_error",
                            "param": "store",
                        },
                    }),
                ),
                (
                    "a key one level inside the parameter",
                    json!({
                        "error": {
                            "message": "Unrecognized request argument supplied: text.format.json_schema",
                            "type": "invalid_request_error",
                            "param": "text.format.json_schema",
                        },
                    }),
                ),
            ],
        ),
    ] {
        for (about, body) in cases {
            unladdered(project, flow, inputs, model, before, about, body);
        }
    }
}

/// Two more families that name the mechanism's own key and are still not about
/// the mechanism (PRD §9 resolved q53).
///
/// Both are the way of being wrong that **hides itself**, for the same reason the
/// case above about a key one level inside the parameter is: the loss is recorded
/// before the retry is sent, so a body read as a missing mechanism demotes a rung
/// that is there for the life of the process, the retry carries the same
/// offending request to the other rung, and the run dies telling an operator
/// there is nothing to configure about an endpoint that has both mechanisms.
///
///   * the key that is also an **English word**. Responses spells its native
///     mechanism `text.format` and the top-level parameter that sits inside
///     `text`, and `text` is a word every other refusal on that surface is free to
///     use: a model that takes no text input, a media type, a sentence pointing
///     at "the text output" as the remedy for something else, and the `text` a
///     **message** carries, addressed at the path it sits on. Each of those
///     clears a word boundary and each says "not supported", and none of them
///     says the endpoint lacks `text.format`;
///   * the mechanism refused **beside another parameter this same request
///     carried**. `output_config` under `thinking:`, `response_format` under a
///     second OpenAI key: the conversation-shape family one step out, and a
///     statement about a request this runtime composed rather than about what the
///     endpoint carries. What the operator has here is one `settings:` key to
///     drop, which is the opposite of nothing to configure.
#[test]
fn a_refusal_naming_the_key_as_a_word_or_blaming_another_parameter_does_not_ladder() {
    // As above: one run, and the claim is that the request count did not grow.
    let unladdered = |project: &str,
                      flow: &str,
                      inputs: &[(&str, &str)],
                      model: &str,
                      before: &[Outcome],
                      about: &str,
                      body: Value| {
        let provider = MockProvider::start().expect("a loopback port");
        for outcome in before {
            provider.enqueue(Script::new(model, outcome.clone()));
        }
        provider.enqueue(Script::new(model, Outcome::raw(400, body)));

        let Some(run) = harness::invoke(project, flow, inputs, &provider) else {
            return;
        };
        run.failed();
        assert_eq!(
            provider.requests().len(),
            before.len() + 1,
            "on `{project}`, a 400 about {about} is not a mechanism this endpoint \
             lacks, so the call ended where it always did — no retry"
        );
    };

    let respond = &[("question", "does it?")][..];
    // The loop's own turn: prose and no calls, so the next request is the pinned
    // one each case is about (grammar 5, D51).
    let loop_turn = &[Outcome::text("I have what I need.")][..];
    for (about, body) in [
        (
            "the word `text` as the input a model does not take",
            json!({
                "error": {
                    "message": "This model does not support text input.",
                    "type": "invalid_request_error",
                },
            }),
        ),
        (
            "the word `text` inside a media type",
            json!({
                "error": {
                    "message": "Invalid value: 'text/plain'. Supported values are: 'application/pdf'. Unsupported parameter: file content type.",
                    "type": "invalid_request_error",
                },
            }),
        ),
        (
            "the word `text` naming the remedy for another parameter",
            json!({
                "error": {
                    "message": "Unsupported parameter: 'reasoning.summary' is not supported with this model. Use the text output instead.",
                    "type": "invalid_request_error",
                    "param": "reasoning.summary",
                },
            }),
        ),
        (
            // Addressed, and at a path whose **last segment** is the key: a
            // message's own text is not the wire parameter `text.format` sits
            // inside, however a service spells the address it stopped at.
            "the word `text` as the tail of another parameter's path",
            json!({
                "error": {
                    "message": "input.3.content.0.text: Extra inputs are not permitted",
                    "type": "invalid_request_error",
                    "param": "input.3.content.0.text",
                },
            }),
        ),
    ] {
        unladdered(
            "server-tools",
            "flow.respond",
            respond,
            GPT5,
            loop_turn,
            about,
            body,
        );
    }

    let review = &[("goal", "ship it"), ("draft", "a draft")][..];
    unladdered(
        "agent-anthropic",
        "flow.review",
        review,
        SONNET,
        &[],
        "the mechanism refused beside a `settings:` key",
        json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "`output_config` is not supported when `thinking` is enabled.",
            },
        }),
    );
    unladdered(
        "agent-openai",
        "flow.review",
        review,
        LOCAL,
        &[],
        "the mechanism refused beside another request parameter",
        json!({
            "error": {
                "message": "Invalid parameter: 'response_format' is not supported when 'logprobs' is requested.",
                "type": "invalid_request_error",
                "param": "response_format",
            },
        }),
    );
}

/// …and a refusal that **is** about the mechanism ladders however the service
/// happened to word it (PRD §9 resolved q53).
///
/// The recognizer's hard edges, one case each, and every one a wording a gateway
/// sends rather than an invention:
///
///   * a capability refusal wearing a schema complaint's clothes — `Invalid
///     schema for response_format: json_schema response format is not supported
///     with this model.` opens with the boilerplate that disqualifies a schema
///     refusal and then says the endpoint does not have the mechanism. Reading
///     only the opening fails a run the forced function would have served, which
///     is the expensive direction of being wrong; what re-qualifies it is that it
///     names the **model** as the thing that lacks it, and no complaint about a
///     schema's contents does that;
///   * `Unexpected parameter: output_config` — the same family as the
///     "unrecognized"/"unknown" spellings already read, in the words a service
///     that validates against a signature uses;
///   * …and the family neither of those is: a parameter the service **has** and
///     this deployment will not carry. `output_config: This feature requires the
///     beta header anthropic-beta: structured-outputs-2025-11-13.` and `Invalid
///     parameter: 'response_format' is not available on this API version.` say
///     "not here" without saying "unknown" or "not supported" once, and gating
///     is the likeliest way a *young* native parameter is refused by the
///     endpoints q53 is for — a generation behind, or a version behind, the wire
///     they serve. The forced tool answers both.
#[test]
fn a_capability_refusal_ladders_in_the_other_wordings_a_gateway_sends() {
    let ladders =
        |project: &str, model: &str, about: &str, refusal: Value, answer: Value, verdict: &str| {
            let provider = MockProvider::start().expect("a loopback port");
            provider.enqueue_all([
                Script::new(model, Outcome::raw(400, refusal)),
                Script::new(model, Outcome::structured(answer)),
            ]);

            let Some(run) = harness::invoke(
                project,
                "flow.review",
                &[("goal", "ship it"), ("draft", "a draft")],
                &provider,
            ) else {
                return;
            };
            run.succeeded();
            assert_eq!(run.outputs()["verdict"], verdict);
            assert_eq!(
                provider.requests().len(),
                2,
                "{about}: the refusal names the mechanism, so the same call went out \
             once more the other way"
            );
            assert_eq!(
                run.entries("review")[0]["models"][0]["outputMechanism"],
                "forced_tool",
                "{about}: …and the rung that answered is the one the trace names"
            );
            assert!(provider.snapshot().is_drained());
        };

    ladders(
        "agent-openai",
        LOCAL,
        "a capability refusal inside a schema complaint's boilerplate",
        json!({
            "error": {
                "message": "Invalid schema for response_format: json_schema response format is not supported with this model.",
                "type": "invalid_request_error",
                "param": "response_format",
            },
        }),
        json!({ "verdict": "revise", "feedback": "tighten it" }),
        "revise",
    );
    ladders(
        "agent-anthropic",
        SONNET,
        "an unknown parameter spelled `unexpected`",
        json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "Unexpected parameter: output_config",
            },
        }),
        json!({ "verdict": "approve", "feedback": "" }),
        "approve",
    );
    ladders(
        "agent-anthropic",
        SONNET,
        "a native parameter this deployment gates behind a beta header",
        json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "output_config: This feature requires the beta header `anthropic-beta: structured-outputs-2025-11-13`.",
            },
        }),
        json!({ "verdict": "approve", "feedback": "" }),
        "approve",
    );
    ladders(
        "agent-openai",
        LOCAL,
        "a native parameter this API version does not have yet",
        json!({
            "error": {
                "message": "Invalid parameter: 'response_format' is not available on this API version.",
                "type": "invalid_request_error",
                "param": "response_format",
            },
        }),
        json!({ "verdict": "revise", "feedback": "one more pass" }),
        "revise",
    );
    // …and the one PRD §9 resolved q55's ruling d could have taken away. Two of
    // the keywords its tables strip — `minimum` and `maximum` — are ordinary
    // English words, and a capability refusal is free to use one on its way to
    // saying the endpoint lacks the parameter. Read as a schema complaint this
    // would stop laddering and fail a run the forced tool serves, so the
    // recognizer holds a word-shaped keyword to being *named as one* the way
    // `text` is held on the Responses wire.
    ladders(
        "agent-anthropic",
        SONNET,
        "a capability refusal that uses `minimum` as a word",
        json!({
            "type": "error",
            "error": {
                "type": "invalid_request_error",
                "message": "output_config is not supported on this API version. The minimum version for structured outputs is 2025-11-13.",
            },
        }),
        json!({ "verdict": "approve", "feedback": "" }),
        "approve",
    );
}

/// …and a refusal a **gateway relayed** ladders on the complaint it relayed,
/// never on the name it put in front of it (PRD §9 resolved q53).
///
/// A proxy in front of a vendor endpoint is the deployment this ladder is most
/// for — running a generation behind the wire it forwards is what a proxy does —
/// and it does not answer in the upstream service's envelope. It wraps the
/// refusal it received in its own exception class or vendor label:
/// `litellm.BadRequestError: AnthropicException - {…}`, `openai.BadRequestError:
/// …`, `provider.openai: …`.
///
/// That prefix is a dotted name in front of a colon, which is spelled exactly
/// like the address a pydantic dialect writes when it stopped somewhere else in
/// the request (`messages.3:`). A recognizer that read every such name as an
/// address would refuse to ladder for every proxied deployment there is, hand its
/// operator the raw 400, and pair it with a ruling that says there is nothing to
/// configure — the outcome q53 exists to prevent, at the endpoints most likely to
/// need it. What settles it instead is the complaint the wrapper carried: it
/// still **points at** the parameter it was always about, by labelling it or by
/// naming it after the colon, which is what a key merely quoted inside a sentence
/// about somewhere else never does.
///
/// Every wire and both directions, because neither the wrapper nor the key it
/// hides is the same twice: the rung refused is the one the endpoint lacks, and
/// which rung a call starts on is decided before any of it (`flow.loose_review`'s
/// schema starts it on the forced tool, so the mirror image is reachable at all).
#[test]
fn a_capability_refusal_a_gateway_relayed_still_ladders() {
    let ladders = |project: &str,
                   flow: &str,
                   inputs: &[(&str, &str)],
                   model: &str,
                   before: &[Outcome],
                   about: &str,
                   refusal: Value,
                   answers: &[Value],
                   refused: OutputMechanism,
                   answered: OutputMechanism,
                   node: &str| {
        let provider = MockProvider::start().expect("a loopback port");
        for outcome in before {
            provider.enqueue(Script::new(model, outcome.clone()));
        }
        provider.enqueue(Script::new(model, Outcome::raw(400, refusal)));
        for answer in answers {
            provider.enqueue(Script::new(model, Outcome::structured(answer.clone())));
        }

        let Some(run) = harness::invoke(project, flow, inputs, &provider) else {
            return;
        };
        run.succeeded();

        let recorded = provider.requests();
        assert_eq!(
            recorded.len(),
            before.len() + answers.len() + 1,
            "{about}: the wrapper is not what the refusal is about, so the same \
             call went out once more the other way"
        );
        assert!(recorded.iter().all(RecordedRequest::is_valid));
        let pinned = before.len();
        assert_eq!(
            [pinned, pinned + 1]
                .into_iter()
                .map(|at| recorded[at]
                    .structured_output
                    .as_ref()
                    .map(StructuredOutput::mechanism))
                .collect::<Vec<_>>(),
            [Some(refused), Some(answered)],
            "{about}: the rung the relayed body named is the one that was tried \
             and dropped"
        );
        assert_eq!(
            run.entries(node)[0]["models"][pinned]["outputMechanism"],
            answered.as_str(),
            "{about}: …and the rung that answered is the one the trace names"
        );
        assert!(provider.snapshot().is_drained());
    };

    let review = &[("goal", "ship it"), ("draft", "a draft")][..];
    ladders(
        "agent-anthropic",
        "flow.review",
        review,
        SONNET,
        &[],
        "a proxy relaying the Messages API's unknown-argument wording",
        json!({
            "error": {
                "message": "litellm.BadRequestError: AnthropicException - {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"output_config: Extra inputs are not permitted\"}}",
                "code": "400",
            },
        }),
        &[json!({ "verdict": "revise", "feedback": "tighten it" })],
        OutputMechanism::Native,
        OutputMechanism::ForcedTool,
        "review",
    );
    ladders(
        "agent-openai",
        "flow.review",
        review,
        LOCAL,
        &[],
        "a proxy relaying Chat Completions' unknown-argument wording",
        json!({
            "error": {
                "message": "openai.BadRequestError: Unrecognized request argument supplied: response_format",
                "code": "400",
            },
        }),
        &[json!({ "verdict": "approve", "feedback": "" })],
        OutputMechanism::Native,
        OutputMechanism::ForcedTool,
        "review",
    );
    ladders(
        "server-tools",
        "flow.respond",
        &[("question", "does it?")],
        GPT5,
        // The loop's own turn: prose and no calls, so the next request is the
        // pinned one this case is about (grammar 5, D51).
        &[Outcome::text("I have what I need.")],
        "a vendor label in front of the Responses wire's own wording",
        json!({
            "error": {
                "message": "provider.openai: Invalid parameter: 'text.format' of type 'json_schema' is not supported with this model.",
                "code": "400",
            },
        }),
        &[json!({ "answer": "the docs say yes" })],
        OutputMechanism::Native,
        OutputMechanism::ForcedTool,
        "ask",
    );
    // The mirror image: the rung the relayed body names is the forced tool, and
    // the ladder runs the other way — the second node then starts on what worked,
    // which is the memo reading a refusal it only ever saw through a wrapper.
    ladders(
        "agent-anthropic",
        "flow.loose_review",
        &[("goal", "ship it")],
        SONNET,
        &[],
        "a proxy relaying the newest generation's forced-tool removal",
        json!({
            "error": {
                "message": "litellm.BadRequestError: AnthropicException - {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"tool_choice: type \\\"tool\\\" and \\\"any\\\" are not supported for this model.\"}}",
                "code": "400",
            },
        }),
        &[
            json!({ "verdict": "revise", "detail": { "headline": "it slips" } }),
            json!({ "verdict": "approve", "detail": { "headline": "it holds" } }),
        ],
        OutputMechanism::ForcedTool,
        OutputMechanism::Native,
        "review",
    );
}

/// …and the same wrapper does **not** turn a complaint about somewhere else in
/// the request into a mechanism refusal (PRD §9 resolved q53).
///
/// The other side of the case above, and the one that decides how far reading
/// past a wrapper is allowed to go. What re-qualifies a relayed body is that the
/// complaint inside it **points at** the mechanism's key; a body that points
/// somewhere else and merely *quotes* the key on its way past is the family the
/// address disqualifier exists for, wrapper or no wrapper. Both cases here would
/// ladder under a rule that re-qualified on any mention:
///
///   * a **setting** refused at its own address, naming `output_config` as the
///     remedy — the run has one `settings:` key to drop, which is the opposite
///     of an endpoint with nothing to configure;
///   * q52's rule as a strict gateway relays it. This is the one that must not
///     move: the closing turn is composed above the mechanism, so a runtime that
///     had regressed it would carry the same illegal conversation to the other
///     rung, be refused again, and report an endpoint carrying neither mechanism
///     — sending its operator to the endpoint to look for this project's bug.
#[test]
fn a_relayed_complaint_about_another_part_of_the_request_still_does_not_ladder() {
    let unladdered = |about: &str, body: Value| {
        let provider = MockProvider::start().expect("a loopback port");
        provider.enqueue(Script::new(SONNET, Outcome::raw(400, body)));

        let Some(run) = harness::invoke(
            "agent-anthropic",
            "flow.review",
            &[("goal", "ship it"), ("draft", "a draft")],
            &provider,
        ) else {
            return;
        };
        run.failed();
        assert_eq!(
            provider.requests().len(),
            1,
            "a relayed 400 about {about} is not a mechanism this endpoint lacks, \
             so the call ended where it always did — no retry"
        );
    };

    unladdered(
        "a setting at its own address, with the mechanism named as the remedy",
        json!({
            "error": {
                "message": "litellm.BadRequestError: AnthropicException - {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"thinking.budget_tokens: Extra inputs are not permitted. Use `output_config` instead.\"}}",
                "code": "400",
            },
        }),
    );
    unladdered(
        "the shape of the conversation",
        json!({
            "error": {
                "message": "litellm.BadRequestError: AnthropicException - {\"type\":\"error\",\"error\":{\"type\":\"invalid_request_error\",\"message\":\"messages.3: This model does not support assistant message prefill. The conversation must end with a user message when `output_config` asks for structured output.\"}}",
                "code": "400",
            },
        }),
    );
}

/// A refusal about the **conversation's shape** is not a mechanism this endpoint
/// lacks, and is not reported as one (PRD §9 resolved q52 and q53).
///
/// The one 400 whose sentence reads like a capability refusal without being one,
/// and the reason the recognizer reads more than a phrase list. A strict gateway
/// holds q52's rule — a forced `tool_choice` over a history ending on the
/// assistant is prefill against a forced call — and names the mechanism's own key
/// in the clause that says why.
///
/// Read as "this endpoint has no forced tool use" it would ladder — and the
/// closing turn is composed *above* the mechanism, so the other rung carries the
/// same conversation, is refused for the same reason, and the run fails saying
/// the endpoint carries neither mechanism and there is nothing to configure.
/// Every word of which would be false: what such a run has is a q52 regression,
/// and the diagnostic would send its operator to the endpoint to look for it.
///
/// Four wordings, because the sentence varies in two independent ways and the
/// defence must not depend on either:
///
///   * **which mechanism asked** — the mock writes both clauses
///     (`crates/mock-provider/src/anthropic.rs`, `check_prefill`), since the rule
///     covers whichever way the request asked for its object, so a recognizer
///     that fell for one would fall for the other;
///   * **whether the complaint carries an address** — `messages.3: …` is what
///     this project's mock sends and what `WIRE-NOTES` (24) records as an
///     *assumed* wording. A gateway that states the same rule as a bare sentence
///     is the case where the shape-reading disqualifier has nothing to read, and
///     the phrase family is what has to answer instead.
#[test]
fn a_refusal_about_the_conversations_shape_is_not_read_as_a_missing_mechanism() {
    for asked in [
        "`tool_choice` forces a tool",
        "`output_config` asks for structured output",
    ] {
        for address in ["messages.3: ", ""] {
            let message = format!(
                "{address}This model does not support assistant message prefill. The \
                 conversation must end with a user message when {asked}."
            );
            let provider = MockProvider::start().expect("a loopback port");
            provider.enqueue(Script::new(
                SONNET,
                Outcome::raw(
                    400,
                    json!({
                        "type": "error",
                        "error": {
                            "type": "invalid_request_error",
                            "message": message.as_str(),
                        },
                    }),
                ),
            ));

            let Some(run) = harness::invoke(
                "agent-anthropic",
                "flow.review",
                &[("goal", "ship it"), ("draft", "a draft")],
                &provider,
            ) else {
                return;
            };
            let failure = run.failed();
            assert!(
                failure.contains("assistant message prefill"),
                "the complaint the endpoint actually made is what the node failed \
                 with, for `{message}`: {failure}"
            );
            assert!(
                !failure.contains("nothing to configure"),
                "…and not the double-refusal diagnostic, which would be a false \
                 statement about the endpoint, for `{message}`: {failure}"
            );
            assert_eq!(
                provider.requests().len(),
                1,
                "…and the same illegal conversation was not sent a second time in \
                 the other shape, for `{message}`"
            );
        }
    }
}

/// The memo is keyed by the **model** as well as by the endpoint (PRD §9
/// resolved q53).
///
/// What one model id refused says nothing about another on the same address:
/// `openai_compatible` in front of a fleet is one provider serving several
/// generations, and a gateway that 400s `response_format` for the old model it
/// proxies takes it for the new one beside it. A memo keyed by the address alone
/// would let the first refusal it sees demote every model behind that address
/// for the life of the process — silently, because the run still *works*: it
/// answers on the other rung, and only the trace and the provider's log say a
/// mechanism nobody chose was used.
///
/// `flow.triage` is the fixture because its three calls are two model ids on one
/// `provider.mock`, in the order that decides it: the refusing model goes first,
/// so a provider-keyed memo would be in place before the other model's call is
/// composed.
#[test]
fn the_memo_holds_per_model_rather_than_per_endpoint() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(HAIKU, Personality::NativeRejected);
    provider.enqueue_all([
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "normalized": "a normalized report" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [{ "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" }],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "patch": "-a\n+b" }))),
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
    assert_eq!(
        recorded
            .iter()
            .map(|request| (
                request.model.as_str(),
                request.unsupported,
                request
                    .structured_output
                    .as_ref()
                    .map(StructuredOutput::mechanism)
            ))
            .collect::<Vec<_>>(),
        [
            (
                HAIKU,
                Some(OutputMechanism::Native),
                Some(OutputMechanism::Native)
            ),
            (HAIKU, None, Some(OutputMechanism::ForcedTool)),
            (SONNET, None, Some(OutputMechanism::Native)),
            (HAIKU, None, Some(OutputMechanism::ForcedTool)),
        ],
        "one model's refusal moved that model's calls and left the other's \
         alone: the third request is the model nothing refused, still asking the \
         way it prefers"
    );
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "{:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );
    assert_eq!(
        run.entries("classify")[0]["models"][0]["outputMechanism"],
        "native",
        "…and the trace says so for the node that ran on it"
    );
    assert!(provider.snapshot().is_drained());
}

/// …and by the **endpoint** as well, which takes two of them (PRD §9 resolved
/// q53).
///
/// The other half of the same key, and the half no single-server fixture can
/// decide: `flow.triage` above holds that one *model*'s refusal does not travel
/// to another, and every model in it is behind one address, so a memo that had
/// dropped the address from its key would pass it unchanged.
///
/// What that would break is the pairing this fixture makes instead: a lagging
/// Anthropic-compatible gateway and the vendor endpoint beside it, both serving
/// `claude-sonnet-4-6`. The gateway's refusal is a fact about the *gateway*, and
/// a memo keyed by the name alone would demote the vendor endpoint's rung with
/// it — silently, because the run still succeeds: the second node answers
/// through the forced tool, produces exactly the object it was going to produce,
/// and only this transcript and the trace say a mechanism nobody chose was used.
/// So the assertion that matters is the **second endpoint's** log: one request,
/// asked the way it prefers, no discovery.
#[test]
fn the_memo_holds_per_endpoint_rather_than_per_model() {
    let lagging = MockProvider::start().expect("a loopback port");
    let vendor = MockProvider::start().expect("a loopback port");
    lagging.personality(SONNET, Personality::NativeRejected);
    lagging.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
    ));
    vendor.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "looks good" })),
    ));

    // The one place in this suite where two servers are told apart: every
    // fixture provider resolves `${MOCK_BASE_URL}`, and `provider.twin` resolves
    // the second address the harness names — pointed here at a server of its
    // own rather than at the one every other test shares.
    let environment: Vec<(String, String)> = harness::environment(&lagging)
        .into_iter()
        .map(|(name, value)| {
            if name == harness::TWIN_BASE_URL {
                (name, vendor.base_url())
            } else {
                (name, value)
            }
        })
        .collect();

    let Some(run) = harness::invoke_with(
        "agent-anthropic",
        "flow.twin_endpoints",
        &json!({ "goal": "ship it", "draft": "a draft" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["feedback"],
        "looks good",
        "the second node answered, which is what makes its endpoint's log the \
         subject at all"
    );

    let mechanisms = |recorded: &[RecordedRequest]| {
        recorded
            .iter()
            .map(|request| {
                (
                    request.unsupported,
                    request
                        .structured_output
                        .as_ref()
                        .map(StructuredOutput::mechanism),
                )
            })
            .collect::<Vec<_>>()
    };

    let refused = lagging.requests();
    assert_eq!(
        mechanisms(&refused),
        [
            (Some(OutputMechanism::Native), Some(OutputMechanism::Native)),
            (None, Some(OutputMechanism::ForcedTool)),
        ],
        "the gateway refused the native rung and answered on the other one"
    );
    let untouched = vendor.requests();
    assert_eq!(
        mechanisms(&untouched),
        [(None, Some(OutputMechanism::Native))],
        "…and the endpoint beside it, serving that very model id, was asked the \
         way it prefers: one request, and nothing to discover"
    );
    for recorded in [&refused, &untouched] {
        assert!(
            recorded.iter().all(RecordedRequest::is_valid),
            "{:?}",
            recorded
                .iter()
                .map(RecordedRequest::failures)
                .collect::<Vec<_>>()
        );
    }

    assert_eq!(
        run.entries("review")[0]["models"][0]["outputMechanism"],
        "forced_tool",
        "the trace says which rung answered, per node"
    );
    assert_eq!(
        run.entries("twin_review")[0]["models"][0]["outputMechanism"],
        "native",
        "…and the two nodes did not answer on the same one"
    );
    assert!(lagging.snapshot().is_drained());
    assert!(vendor.snapshot().is_drained());
}

/// The **native** rung's pinned call leaves the agent's tools callable, and a
/// model that calls one instead of answering fails the node the way an absent
/// object always has (PRD §9 resolved q53).
///
/// The posture is the one Chat Completions has had for as long as it has sent
/// `response_format` (`WIRE-NOTES` (3)), and on the Messages wire it is new: a
/// forced `tool_choice: {type: "tool"}` was a promise that the turn is the
/// pinned call, so a closing turn that called something else was unreachable
/// there. Under `output_config` it is reachable, and this pins what happens —
/// the same "carried no structured output" the wire's other absences produce,
/// naming the agent, the output it asked for, and the stop reason.
///
/// It is deliberately not closed by sending `tool_choice: {type: "none"}` beside
/// the native parameter, and `provider.mock` is the case that decides it:
/// **it declares no `server_tools:`**, so the reason that closure would break
/// Decision D122 — forbidding the provider's own tools along with the agent's on
/// the very call D122 puts them on — has nothing to bite on here. What holds
/// instead is the ladder's: this rung recovers from an endpoint that will not
/// take its request only through a refusal naming the rung's key
/// (`MECHANISM_KEYS`), and a `tool_choice` beside the format is a second
/// capability whose refusal names the *other* rung's key, so an endpoint
/// carrying `output_config` and not `tool_choice: {type: "none"}` would fail
/// with nowhere to ladder to. Sending it only where a provider declares no
/// server tools would also make this one call's shape differ between two
/// providers of one kind. The tools themselves cannot be dropped either — the
/// history this request replays carries `tool_use`/`tool_result` blocks and the
/// Messages API refuses a request that names tools it does not declare.
///
/// So the cost of leaving it open is this failure, which is a model ignoring a
/// fixed instruction to produce its result, reported as such — and the request
/// that provoked it is asserted below to be the one-parameter shape that
/// argument describes.
#[test]
fn a_native_pinned_call_answered_with_a_tool_call_carries_no_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The turn that ends the loop: prose, no calls (grammar 5, D51).
        Script::new(SONNET, Outcome::text("found it")),
        // …and the closing call answered with one more call instead of the
        // object. Nothing about it is a refusal: the endpoint accepted the
        // request and the model answered something this runtime cannot use.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "lookup",
                json!({ "query": "one more thing" }),
            )]),
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
    let failure = run.failed();
    assert!(
        failure.contains("`agent.researcher` asked for")
            && failure.contains("the answer carried no structured output"),
        "the node error names the agent and what it asked for: {failure}"
    );
    assert!(
        failure.contains("stop_reason: tool_use"),
        "…and what the surface said about why there was none: {failure}"
    );
    assert!(
        !failure.contains("nothing to configure"),
        "…and not the double-refusal diagnostic, which would say the endpoint \
         carries neither mechanism when it refused nothing at all: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        2,
        "the loop's turn and the pinned call, with no retry: an answer this \
         runtime cannot use is not a mechanism the endpoint lacks"
    );
    assert_eq!(
        recorded[1]
            .structured_output
            .as_ref()
            .map(StructuredOutput::mechanism),
        Some(OutputMechanism::Native),
        "…and the call that got it asked the native way, which is the rung this \
         is about"
    );
    assert_eq!(
        recorded[1].tools,
        ["lookup"],
        "the pinned call offers the agent's own tools and pins none of them: {:?}",
        recorded[1].tools
    );
    let body = recorded[1]
        .body
        .as_ref()
        .expect("the pinned call arrived as JSON");
    assert!(
        body.get("tool_choice").is_none(),
        "the native rung composes one parameter and not two: a `tool_choice` \
         beside `output_config` is the second capability the rung would depend \
         on, and an endpoint refusing it names the other rung's key — there \
         would be nowhere to ladder to: {body:#}"
    );
    assert!(
        body.get("output_config").is_some(),
        "…and the one parameter it does compose is the format itself: {body:#}"
    );
    assert!(
        recorded[1].server_tools.is_empty(),
        "`provider.mock` declares no `server_tools:`, which is what makes this \
         the case Decision D122's reason does not cover: {:?}",
        recorded[1].server_tools
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert!(provider.snapshot().is_drained());
}

/// A pinned **Messages** turn that came back as prose fails the node with the
/// sentence the other wires fail it with (PRD §9 resolved q53).
///
/// The native mechanism's reading path, which is the half of the ladder that is
/// not a request: under `output_config` the object arrives as the assistant's
/// *text*, so what a run gets when the format did not shape it is a turn that
/// parses as nothing. `text.format`'s twin is
/// `a_pinned_responses_turn_carrying_no_object_is_reported_as_no_structured_output`
/// and the argument is the same one: the absence is reported as an absence, so
/// the node error names the agent, the output it asked for, and what the surface
/// said about why. A `SyntaxError` raised inside `callMessages` names none of the
/// three and is raised *inside* the journaled model call, so a resume would
/// replay a recorded failure of a call that had in fact answered.
#[test]
fn a_pinned_messages_turn_carrying_no_object_is_reported_as_no_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::raw(
            200,
            json!({
                "id": "msg_prose",
                "type": "message",
                "role": "assistant",
                "model": SONNET,
                "content": [{ "type": "text", "text": "Sorry, I could not review that." }],
                "stop_reason": "end_turn",
                "stop_sequence": null,
                "usage": { "input_tokens": 12, "output_tokens": 9 },
            }),
        ),
    ));

    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("`agent.reviewer` asked for")
            && failure.contains("the answer carried no structured output"),
        "the node error names the agent and what it asked for: {failure}"
    );
    assert!(
        failure.contains("stop_reason: end_turn"),
        "…and what the surface said about why there was none: {failure}"
    );
    assert!(
        !failure.contains("SyntaxError"),
        "…rather than the parser's message: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request
                .structured_output
                .as_ref()
                .map(StructuredOutput::mechanism))
            .collect::<Vec<_>>(),
        [Some(OutputMechanism::Native)],
        "…and the call that got it really did ask the native way, which is the \
         reading path this is about"
    );
}

/// …and so does a pinned **Chat Completions** turn, which is the wire where
/// prose is reachable without any misbehaviour at all.
///
/// The third of the three sibling readings, and the one with an ordinary route
/// into it. `flow.triage`'s output nests an `optional:` property, so this wire's
/// `response_format` goes out with `strict: false` — the row
/// `a_nested_optional_property_costs_the_strict_decoder_and_not_the_parse` pins
/// — and an unstrict decoder is not constrained by the schema at all, so prose
/// is a legal answer to it. (So is a gateway that takes `response_format` and
/// ignores it, which is the endpoint class the ladder exists for.) A
/// `JSON.parse` here would fail the node with `SyntaxError`, naming neither the
/// agent, nor the output it asked for, nor what the surface said about why —
/// and would throw *inside* the journaled model call, recording a call that
/// answered as one that failed, which a resume would then replay forever.
#[test]
fn a_pinned_chat_completions_turn_carrying_no_object_is_reported_as_no_structured_output() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        LOCAL,
        Outcome::raw(
            200,
            json!({
                "id": "chatcmpl_prose",
                "object": "chat.completion",
                "created": 1_735_689_600,
                "model": LOCAL,
                "choices": [{
                    "index": 0,
                    "message": { "role": "assistant", "content": "Sorry, I could not triage that." },
                    "finish_reason": "stop",
                }],
                "usage": { "prompt_tokens": 12, "completion_tokens": 9, "total_tokens": 21 },
            }),
        ),
    ));

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.triage",
        &[("report", "the build is red")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("`agent.triager` asked for")
            && failure.contains("the answer carried no structured output"),
        "the node error names the agent and what it asked for: {failure}"
    );
    assert!(
        failure.contains("stop_reason: stop"),
        "…and what the surface said about why there was none: {failure}"
    );
    assert!(
        !failure.contains("SyntaxError"),
        "…rather than the parser's message: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request
                .structured_output
                .as_ref()
                .map(StructuredOutput::mechanism))
            .collect::<Vec<_>>(),
        [Some(OutputMechanism::Native)],
        "…and the call that got it really did ask through `response_format`, \
         which is the reading path this is about"
    );
}

/// Resolved q52's closing user turn is on **both** mechanisms' requests.
///
/// The turn is composed above the wire and above the mechanism — one fixed user
/// turn appended to a copy of the loop's turns before the output is asked for —
/// so a ladder that swapped the mechanism must not swap that. The lagging
/// gateway is what makes both shapes reachable in one run: the same request is
/// composed twice, once each way, and neither may end on the assistant.
#[test]
fn the_closing_turn_precedes_the_pinned_call_on_both_mechanisms() {
    const CLOSING: &str = "Now produce the structured result.";

    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::NativeRejected);
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
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
    assert_eq!(
        recorded.len(),
        4,
        "two loop calls, then the pinned one twice — once per rung"
    );
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "the strict gateway's own rule is what `is_valid` carries here, and \
         neither shape may break it: {:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );
    for (ordinal, request) in recorded[2..].iter().enumerate() {
        let last = request.body()["messages"]
            .as_array()
            .and_then(|messages| messages.last())
            .expect("a last message")
            .clone();
        assert_eq!(
            last["role"], "user",
            "rung {ordinal} ends on the user (resolved q52): {}",
            request.body_text
        );
        assert_eq!(
            last["content"], CLOSING,
            "…with the same fixed turn: {}",
            request.body_text
        );
    }
    assert_eq!(
        recorded[2].unsupported,
        Some(OutputMechanism::Native),
        "and the two rungs really were the two mechanisms"
    );
    assert_eq!(
        recorded[3]
            .structured_output
            .as_ref()
            .map(StructuredOutput::mechanism),
        Some(OutputMechanism::ForcedTool)
    );
    assert!(provider.snapshot().is_drained());
}

/// The ladder is the **other OpenAI wire's** too: Chat Completions falls from
/// `response_format` to a forced function, and the run finishes on it.
///
/// One test for the second surface rather than a copy of every claim above: what
/// is wire-specific is the pair of shapes and the recogniser that reads each
/// endpoint's refusal, and what is not — the memo, the trace field, the double
/// refusal — is decided once on the Messages wire.
#[test]
fn the_chat_completions_wire_ladders_to_a_forced_function() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(LOCAL, Personality::NativeRejected);
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
    assert_eq!(run.outputs()["verdict"], "revise");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2, "one refusal, then the other rung");
    assert_eq!(recorded[0].unsupported, Some(OutputMechanism::Native));
    assert!(recorded.iter().all(RecordedRequest::is_valid));

    let answered = recorded[1].body();
    assert_eq!(
        answered["tool_choice"]["function"]["name"], "reviewer_output",
        "the second rung is the function pinned by name: {}",
        recorded[1].body_text
    );
    assert!(
        answered["response_format"].is_null(),
        "…and the request carries one mechanism, not both: {}",
        recorded[1].body_text
    );
    assert_eq!(
        answered["tools"][0]["function"]["strict"], true,
        "…with `strict` on the function, which is where this wire takes it: {}",
        recorded[1].body_text
    );
    assert_eq!(
        run.entries("review")[0]["models"][0]["outputMechanism"],
        "forced_tool"
    );
    assert!(provider.snapshot().is_drained());
}

/// …and the **third** wire's, which spells both rungs differently again:
/// Responses falls from `text.format` to a flat forced function, over a loop
/// that also carries the provider's own server tools (Decision D122).
///
/// The wire where the two mechanisms are most easily confused with the tool
/// surface around them: a client function, the provider's suite and the pinned
/// output tool all live in one `tools` array here, so what this pins beside the
/// ladder is that the pin joins that array without displacing either of the
/// other two.
#[test]
fn the_responses_wire_ladders_to_a_forced_function() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(GPT5, Personality::NativeRejected);
    provider.enqueue_all([
        // The turn that ends the loop: prose, no calls (grammar 5, D51).
        Script::new(GPT5, Outcome::text("I have what I need.")),
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "the docs say yes" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "the docs say yes");

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        3,
        "the loop's one call, then the pinned one on each rung"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    assert_eq!(recorded[0].surface, Surface::Responses);
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.unsupported)
            .collect::<Vec<_>>(),
        [None, Some(OutputMechanism::Native), None],
        "the loop call asked for no object; the first pinned one was refused"
    );

    let answered = recorded[2].body();
    let pinned = recorded[2]
        .structured_output
        .as_ref()
        .and_then(StructuredOutput::name)
        .expect("the second rung names the function it pins")
        .to_string();
    assert_eq!(
        answered["tool_choice"],
        json!({ "type": "function", "name": pinned }),
        "flat here, where Chat Completions nests the name: {}",
        recorded[2].body_text
    );
    assert!(
        answered["text"].is_null(),
        "…and one mechanism per request, never both: {}",
        recorded[2].body_text
    );
    assert_eq!(
        recorded[2].tools,
        ["lookup", pinned.as_str()],
        "the agent's own tool leads, the pinned output tool follows it: {:?}",
        recorded[2].tools
    );
    assert_eq!(
        recorded[2].server_tools,
        ["web_search"],
        "…and the provider's suite is still last and still there: {:?}",
        recorded[2].server_tools
    );
    assert_eq!(
        run.entries("ask")[0]["models"][1]["outputMechanism"],
        "forced_tool",
        "the trace names it on the pinned call, and on that one only: {}",
        run.entries("ask")[0]
    );
    assert!(
        run.entries("ask")[0]["models"][0]["outputMechanism"].is_null(),
        "…the loop's call asked for no object: {}",
        run.entries("ask")[0]
    );
    assert!(provider.snapshot().is_drained());
}

/// An array-bearing output schema rides every wire's **native** rung, lowered to
/// what that decoder compiles (PRD §9 resolved q55, rulings a and b).
///
/// The regression the whole ruling is about, on all three wires at once. Grammar
/// D10 makes `max_items` REQUIRED on every array inside a result schema and §3.5
/// sends it as `maxItems`, which no native structured-output decoder compiles —
/// so before lowering, `agent.tallier`'s schema was a 400 on the rung resolved
/// q53 prefers, on every wire, for every composition with an array in its
/// `output:`. Which is nearly all of them.
///
/// What makes this a proof rather than an exercise is the **mock**: each surface
/// enforces the subset its decoder compiles (`WIRE-NOTES` (28)), so an
/// under-lowered schema is a refused request and a failed run rather than a
/// green suite and somebody's first live call. `is_valid()` below is therefore
/// the load-bearing assertion, and the keyword sweep beside it is what says
/// *why* it passed.
///
/// The three fixtures declare one agent word for word, differing only in
/// `model:`, so what the loop below varies is the wire and nothing else.
#[test]
fn an_array_bearing_output_rides_each_wires_native_rung_lowered() {
    let answer = json!({
        "headline": "three things",
        "tallies": [
            { "label": "one", "weight": 0.5 },
            { "label": "two", "weight": 0.25 },
        ],
        "total": 2,
    });
    for (project, model, surface) in [
        ("agent-anthropic", SONNET, Surface::Anthropic),
        ("agent-openai", LOCAL, Surface::OpenAi),
        ("server-tools", GPT5, Surface::Responses),
    ] {
        let provider = MockProvider::start().expect("a loopback port");
        provider.enqueue(Script::new(model, Outcome::structured(answer.clone())));

        let Some(run) = harness::invoke(
            project,
            "flow.tally",
            &[("subject", "a short list")],
            &provider,
        ) else {
            return;
        };
        run.succeeded();

        let recorded = provider.requests();
        assert_eq!(recorded.len(), 1, "on `{project}`: one call, and no ladder");
        let call = &recorded[0];
        assert!(
            call.is_valid(),
            "on `{project}`, the schema this runtime put on the wire is one this decoder cannot \
             compile — the lowering table for this (wire, mechanism) is missing a row (PRD \
             resolved q55): {:?}",
            call.failures()
        );
        assert_eq!(call.surface, surface);
        let asked = call
            .structured_output
            .as_ref()
            .expect("an agent always asks for structured output (PRD 5.2)");
        assert_eq!(
            asked.mechanism(),
            OutputMechanism::Native,
            "on `{project}`: the native rung is the one an array-bearing schema now rides — \
             demoting it to the forced tool is the alternative q55 rejected"
        );
        assert_eq!(
            mock_provider::lowering::carries_refused_keyword(
                surface,
                OutputMechanism::Native,
                asked.schema()
            ),
            None,
            "on `{project}`, the wire schema still carries a keyword this decoder refuses: {}",
            asked.schema()
        );

        // …and every stripped bound is still in the model's view, folded into
        // the description of the node it came off, after the author's own words
        // (ruling b).
        let schema = asked.schema();
        assert_eq!(
            schema["properties"]["tallies"]["description"],
            "What was found, one note per item. At most 3 items.",
            "on `{project}`: {schema}"
        );
        assert_eq!(
            schema["properties"]["headline"]["description"],
            "One line naming the subject. Between 1 and 60 characters.",
            "on `{project}`: {schema}"
        );
        assert_eq!(
            schema["properties"]["tallies"]["items"]["properties"]["weight"]["description"],
            "How much it matters. Between 0 and 1.",
            "on `{project}`, two levels down inside an array of objects: {schema}"
        );
        assert_eq!(
            schema["properties"]["total"]["description"],
            "How many tallies there are. Between 0 and 3.",
            "on `{project}`: {schema}"
        );
        // Nothing else moved: the shape, the names and the closure are the
        // emitted schema's.
        assert_eq!(
            schema["properties"]["tallies"]["items"]["required"],
            json!(["label", "weight"]),
            "on `{project}`: lowering removes constraint keywords and touches nothing else"
        );
        assert_eq!(schema["additionalProperties"], json!(false));
        assert!(provider.snapshot().is_drained());
    }
}

/// An answer that overruns a **stripped** bound fails the emitted parse, exactly
/// as any schema-violating answer does — and is never truncated to fit (PRD §9
/// resolved q55, ruling c).
///
/// The other half of ruling a. Lowering takes `maxItems` off the *request*, so
/// the decoder is no longer refusing a fourth element; what refuses it is the
/// emitted Zod, which runs over the full schema and is what PRD 9.16 makes the
/// contract. Nothing new was invented for the overflow — no retry, no trim, no
/// second ask — so the failure a run gets is the one it has always got for an
/// answer its schema does not admit.
///
/// On every wire, because the *reason* the bound is unenforced differs per wire
/// and the parse does not: a suite that checked one would leave the other two's
/// answers trimmed or accepted with nobody the wiser.
#[test]
fn an_answer_over_a_stripped_bound_fails_the_parse() {
    // Four elements against `max_items: 3` — one more than the bound the wire
    // schema no longer carries.
    let overrun = json!({
        "headline": "four things",
        "tallies": [
            { "label": "one", "weight": 0.5 },
            { "label": "two", "weight": 0.25 },
            { "label": "three", "weight": 0.125 },
            { "label": "four", "weight": 0.0625 },
        ],
        "total": 3,
    });
    for (project, model) in [
        ("agent-anthropic", SONNET),
        ("agent-openai", LOCAL),
        ("server-tools", GPT5),
    ] {
        let provider = MockProvider::start().expect("a loopback port");
        provider.enqueue(Script::new(model, Outcome::structured(overrun.clone())));

        let Some(run) = harness::invoke(
            project,
            "flow.tally",
            &[("subject", "a short list")],
            &provider,
        ) else {
            return;
        };
        let failure = run.failed();
        assert!(
            failure.contains("tally"),
            "on `{project}`: the node whose answer did not parse is named: {failure}"
        );
        assert!(
            failure.contains("tallies"),
            "on `{project}`: …and so is the field whose bound it overran: {failure}"
        );

        let recorded = provider.requests();
        assert_eq!(
            recorded.len(),
            1,
            "on `{project}`: an answer the parse refuses is a node failure, not a reason to ask \
             again — nothing about the overflow path is new (PRD resolved q55 ruling c)"
        );
        assert!(recorded[0].is_valid());
        assert!(
            !recorded[0].was_unsupported(),
            "on `{project}`: nothing was refused by the endpoint — the schema went out lowered \
             and the answer came back over its bound"
        );
    }

    // …and the bound is where the schema says it is rather than one either side
    // of it: the same answer with the third element and not the fourth is a run
    // that succeeds. Which is also what makes the failures above evidence that
    // nothing **truncated** the overrun to fit — a runtime that trimmed to
    // `max_items` would have produced this run three times over.
    let provider = MockProvider::start().expect("a loopback port");
    let mut at_the_bound = overrun.clone();
    at_the_bound["tallies"]
        .as_array_mut()
        .expect("the scripted answer carries an array")
        .pop();
    provider.enqueue(Script::new(SONNET, Outcome::structured(at_the_bound)));
    let Some(run) = harness::invoke(
        "agent-anthropic",
        "flow.tally",
        &[("subject", "a short list")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert!(provider.snapshot().is_drained());
}

/// A 400 naming a **schema keyword** fails the call loudly: no ladder, no memo,
/// and the endpoint's body quoted (PRD §9 resolved q55, ruling d).
///
/// After lowering, this refusal can only mean one thing. The mechanism's own
/// table says that keyword was never sent, so an endpoint complaining about it
/// is not an endpoint that lacks the mechanism — it is this runtime's lowering
/// table missing a row. q53's recognizer would otherwise read the sentence as a
/// capability refusal (it names the rung's key and says "not supported"), ladder
/// to the other rung, be refused there for the same reason, and **memoize** a
/// mechanism as absent from an endpoint that has it — the way of being wrong
/// that hides itself.
///
/// Twice over, because a table has **two** ways of being wrong and only one of
/// them is about a keyword it strips. The second half stages `pattern` — a
/// constraint no table takes off the wire, deliberately, on the posture that
/// under-stripping is the cheap direction of being wrong. That posture is only
/// true if a refusal naming what was left on is loud rather than a ladder trip:
/// the recognizer therefore watches every constraint keyword a schema can carry,
/// not merely the ones some table strips.
///
/// A third half, about what this diagnostic cannot know: `format` is a schema
/// keyword *and* the tail of `text.format`, so on the Responses wire a refusal
/// pointing at a bare `format` is either a decoder refusing the keyword or a
/// gateway refusing the parameter under its short name. Both readings fail the
/// call the same way, and the message names both — the alternative, holding a
/// word-shaped keyword to more than quoting, would hand a schema complaint
/// (`output_config: minimum is not supported.`) back to the ladder that memoizes
/// a mechanism as absent from an endpoint that has it.
///
/// `bounded-cycle`'s `write` node declares `retry: { max: 2 }`, which is what
/// makes the *memo* observable: three attempts, and each one has to start on the
/// native rung again. A remembered refusal would show up as the second attempt
/// asking the other way.
#[test]
fn a_schema_keyword_refusal_fails_the_call_without_laddering_or_remembering() {
    let provider = MockProvider::start().expect("a loopback port");
    // The shape that is *not* addressed at a path and *does* carry a capability
    // phrase, which is the one q53's recognizer would have laddered on.
    provider.enqueue(
        Script::new(
            SONNET,
            Outcome::raw(
                400,
                json!({
                    "type": "error",
                    "error": {
                        "type": "invalid_request_error",
                        "message": "Unsupported parameter: output_config — 'maxItems' is not supported by this model.",
                    },
                }),
            ),
        )
        .times(3)
        .matching("research writer"),
    );

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
        failure.contains("maxItems"),
        "the keyword the endpoint named is what the diagnostic is about: {failure}"
    );
    assert!(
        failure.contains("lowering table"),
        "…and the repair is named as this runtime's own, not the operator's: {failure}"
    );
    assert!(
        failure.contains("'maxItems' is not supported by this model."),
        "…with the provider's body quoted rather than paraphrased: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        3,
        "three attempts of the node's `retry:` policy, and **one** request each: a schema-keyword \
         400 never ladders to the other rung"
    );
    assert!(
        recorded.iter().all(|request| request
            .structured_output
            .as_ref()
            .map(StructuredOutput::mechanism)
            == Some(OutputMechanism::Native)),
        "…and every attempt starts on the native rung, so nothing was remembered: {:?}",
        recorded
            .iter()
            .map(|request| request
                .structured_output
                .as_ref()
                .map(StructuredOutput::mechanism))
            .collect::<Vec<_>>()
    );
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…and the endpoint refused no mechanism: what it refused was the schema"
    );

    // …and the same, for a keyword the tables deliberately **leave on the
    // wire**. `pattern` is one of the two: the ruling's families are array,
    // numeric and string-length bounds, and the posture q55 takes from resolved
    // q30 is to under-strip rather than over-strip, because a keyword left on
    // that a decoder will not compile is a 400 that names it. That trade is only
    // paid for if this refusal behaves exactly like the one above — which is why
    // the recognizer's vocabulary is every constraint keyword a schema can
    // carry, and not the union of the tables.
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(
        Script::new(
            SONNET,
            Outcome::raw(
                400,
                json!({
                    "type": "error",
                    "error": {
                        "type": "invalid_request_error",
                        "message": "Unsupported parameter: output_config — 'pattern' is not supported by this model.",
                    },
                }),
            ),
        )
        .times(3)
        .matching("research writer"),
    );

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
        failure.contains("pattern") && failure.contains("lowering table"),
        "a keyword no table strips is still this runtime's own failure, named and repaired in the \
         same place: {failure}"
    );
    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        3,
        "one request per attempt: a constraint keyword the table left on the wire does not ladder \
         either — the other rung would send the same schema"
    );
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…and nothing was recorded as a mechanism this endpoint lacks"
    );

    // …and the one keyword whose spelling is **also** a parameter's. `format` is
    // a JSON Schema keyword and the tail of `text.format`, the Responses wire's
    // own structured-output parameter, which a gateway that has never heard of
    // that wire refuses under its short name — so a body pointing at a bare
    // `format` is both readings at once and nothing here can decide it. The
    // classification is the same either way (no ladder, no memo, the body
    // quoted), so what the diagnostic owes an operator is the second reading
    // named beside the first rather than a table row edited on a guess.
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        GPT5,
        Outcome::raw(
            400,
            json!({
                "error": {
                    "message": "Unrecognized request argument supplied: format",
                    "type": "invalid_request_error",
                },
            }),
        ),
    ));

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.tally",
        &[("subject", "a short list")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("`format` is a constraint keyword"),
        "the keyword is read as this runtime's own repair, as every other one is: {failure}"
    );
    assert!(
        failure.contains("`text.format`"),
        "…and the parameter it is spelled like is named too, because no rule here tells the two \
         apart: {failure}"
    );
    assert!(
        failure.contains("read the quoted body"),
        "…so the operator is pointed at the body rather than at a row to edit on a guess: \
         {failure}"
    );
    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        1,
        "…and it is still one request: an ambiguous spelling is not a reason to ladder"
    );
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…nor to remember a mechanism as absent"
    );
}

/// The **forced-function** rung sends the same lowered schema, on the two wires
/// whose forced row is not empty (PRD §9 resolved q55, ruling a).
///
/// The native rung is what the ruling was resolved from and what the run above
/// proves; the other rung's row is a table entry like any other, and on the
/// OpenAI wires it is a full one — `strict: true` on a function's `parameters`
/// hands that schema to the same decoder `response_format` and `text.format`
/// hand theirs to. Proven end to end rather than statically, because a
/// forced-rung row is otherwise attested only by a mock check that never fires
/// on a corpus whose forced-rung agents declare no bounds: a vacuous pass looks
/// exactly like a correct one.
///
/// The Messages wire is deliberately absent: its forced row strips nothing, and
/// `wire_lowering_agreement.rs` holds that empty row to the oracle's own.
#[test]
fn the_forced_function_rung_carries_the_lowered_schema_too() {
    let answer = json!({
        "headline": "three things",
        "tallies": [
            { "label": "one", "weight": 0.5 },
            { "label": "two", "weight": 0.25 },
        ],
        "total": 2,
    });
    for (project, model, surface) in [
        ("agent-openai", LOCAL, Surface::OpenAi),
        ("server-tools", GPT5, Surface::Responses),
    ] {
        let provider = MockProvider::start().expect("a loopback port");
        provider.personality(model, Personality::NativeRejected);
        provider.enqueue(Script::new(model, Outcome::structured(answer.clone())));

        let Some(run) = harness::invoke(
            project,
            "flow.tally",
            &[("subject", "a short list")],
            &provider,
        ) else {
            return;
        };
        run.succeeded();

        let recorded = provider.requests();
        assert_eq!(
            recorded.len(),
            2,
            "on `{project}`: the refused native rung, then the forced function"
        );
        assert!(
            recorded.iter().all(RecordedRequest::is_valid),
            "on `{project}`, a request this decoder cannot compile — the lowering table for the \
             forced rung is missing a row (PRD resolved q55): {:?}",
            recorded
                .iter()
                .map(RecordedRequest::failures)
                .collect::<Vec<_>>()
        );
        assert_eq!(recorded[0].unsupported, Some(OutputMechanism::Native));
        let asked = recorded[1]
            .structured_output
            .as_ref()
            .expect("the second rung pins a function carrying the schema");
        assert_eq!(asked.mechanism(), OutputMechanism::ForcedTool);
        assert_eq!(
            mock_provider::lowering::carries_refused_keyword(
                surface,
                OutputMechanism::ForcedTool,
                asked.schema()
            ),
            None,
            "on `{project}`, the pinned function's `parameters` still carries a keyword this \
             decoder refuses: {}",
            asked.schema()
        );
        assert_eq!(
            asked.schema()["properties"]["tallies"]["description"],
            "What was found, one note per item. At most 3 items.",
            "…and the bound it lost is still in the model's view (ruling b): {}",
            asked.schema()
        );
        assert!(provider.snapshot().is_drained());
    }
}

/// A **client tool's** own schema is lowered wherever its wire compiles it, and
/// left whole where it does not (PRD §9 resolved q55, ruling a; grammar §3.5).
///
/// The schema an agent's `output:` rides on is not the only one a request
/// carries. A `tool.…` `input:` may declare `min_length`, `minimum` or
/// `max_items` too, and the Responses wire declares `strict` on every function
/// it is handed — which is the promise that puts a schema through the
/// structured-output compiler. So `tool.lookup`'s `min_length: 1` is a live 400
/// there unless it is lowered like anything else, and this is the run that says
/// so: the mock enforces the subset on any function declared `strict`
/// (`WIRE-NOTES` (28)), so an unlowered tool is a refused request rather than a
/// green suite.
///
/// Chat Completions is the control, and the contrast is the point of pairing
/// them: a client tool goes out there with no `strict` at all, nothing compiles
/// its schema, and the bound stays where the author wrote it. Lowering is per
/// (wire, mechanism) and it does not spread.
#[test]
fn a_client_tools_schema_is_lowered_where_its_wire_declares_strict() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The turn that ends the loop: prose, no calls (grammar 5, D51).
        Script::new(GPT5, Outcome::text("I have what I need.")),
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "the docs say yes" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert!(
        recorded.iter().all(RecordedRequest::is_valid),
        "a tool schema this wire's `strict` decoder cannot compile is a 400 here, exactly as it \
         is live: {:?}",
        recorded
            .iter()
            .map(RecordedRequest::failures)
            .collect::<Vec<_>>()
    );
    let declared = recorded[0].body()["tools"][0].clone();
    assert_eq!(
        declared["name"], "lookup",
        "the agent's own tool leads this wire's `tools`: {}",
        recorded[0].body_text
    );
    assert_eq!(
        declared["strict"], true,
        "…declared `strict`, which is what makes its schema the decoder's business: {}",
        recorded[0].body_text
    );
    assert!(
        declared["parameters"]["properties"]["query"]["minLength"].is_null(),
        "…so the bound comes off the request: {}",
        recorded[0].body_text
    );
    assert_eq!(
        declared["parameters"]["properties"]["query"]["description"],
        "What to look up. At least 1 character.",
        "…and is folded into the description the model reads (ruling b): {}",
        recorded[0].body_text
    );
    assert!(provider.snapshot().is_drained());

    // …and the wire that declares no `strict` on a client tool sends the bound
    // itself: nothing compiles it there, so nothing is taken off.
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(LOCAL, Outcome::text("found it")),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let recorded = provider.requests();
    assert!(recorded.iter().all(RecordedRequest::is_valid));
    let declared = recorded[0].body()["tools"][0]["function"].clone();
    assert_eq!(declared["name"], "lookup");
    assert!(
        declared["strict"].is_null(),
        "a client tool carries no `strict` on this wire: {}",
        recorded[0].body_text
    );
    assert_eq!(
        declared["parameters"]["properties"]["query"]["minLength"],
        json!(1),
        "…so its bound rides the request as the author wrote it: {}",
        recorded[0].body_text
    );
    assert!(provider.snapshot().is_drained());
}

/// …and a schema-keyword 400 drawn by **that** schema is the same loud failure,
/// on a call that pinned nothing at all (PRD §9 resolved q55, ruling d).
///
/// The refusal ruling d is written about arrives on the pinned request, where the
/// mechanism ladder is. A strict client tool's `parameters` reaches the same
/// decoder on every call of the **tool loop**, and those calls pin nothing: the
/// first request of `flow.respond` carries `tool.lookup` with `strict: true` and
/// no `text.format` at all. A table missing a row there is the identical bug with
/// the identical repair, so it has to read as one — otherwise which message an
/// operator gets depends on which call of the loop the model happened to stop at.
///
/// The two hard guarantees are not at stake on this path and the assertions say
/// so anyway: an unpinned call has no second rung, so one request is all there
/// is, and nothing is remembered because only the pinned ladder records a
/// refusal. What the diagnostic has to carry is the keyword, the provider's own
/// body, and whose bug it is.
#[test]
fn a_schema_keyword_refusal_on_a_loop_call_names_the_tools_own_table() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        GPT5,
        Outcome::raw(
            400,
            json!({
                "error": {
                    "message": "Invalid schema for function 'lookup': 'pattern' is not permitted.",
                    "type": "invalid_request_error",
                    "param": "tools[0].parameters",
                },
            }),
        ),
    ));

    let Some(run) = harness::invoke(
        "server-tools",
        "flow.respond",
        &[("question", "does it?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("pattern"),
        "the keyword the endpoint named is what the diagnostic is about: {failure}"
    );
    assert!(
        failure.contains("`parameters`"),
        "…and it is a tool's schema rather than the pinned one, because this call pinned none: \
         {failure}"
    );
    assert!(
        failure.contains("`responses`/`forced_tool` lowering table"),
        "…so the row named is the one a function's `parameters` is lowered through: {failure}"
    );
    assert!(
        failure.contains("'pattern' is not permitted."),
        "…with the provider's body quoted rather than paraphrased: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        1,
        "one request: a call that pins nothing has no second rung to try, and a schema keyword is \
         not a capability refusal on any path"
    );
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…and the endpoint refused no mechanism: what it refused was a schema"
    );
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

/// A `function:` node bound to a value its tool's `input:` refuses **fails the
/// node**, and says so as a mismatch rather than as a refusal (grammar 6, 8.4,
/// Decision D119).
///
/// One `tool.*` definition, two usage surfaces, and this is the surface with no
/// model on it. Grammar 8.4 checks the binding for arity and types, so a
/// `min_length:` a CEL expression misses arrives at the runtime parse — where
/// the arguments are the composition's own and there is nobody to hand them back
/// to. The class in `TraceEntry.error` is the observable, because D119 gives
/// `ToolCallRefused` a meaning a reader is entitled to rely on — the model was
/// handed this and the loop carried on — and a node that just ended must not
/// wear it.
#[test]
fn a_function_nodes_unfit_argument_fails_the_node_as_a_mismatch() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(run) = harness::invoke("activities", "flow.unfit", &[], &provider) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("ResultMismatch: the arguments `tool.echo` was called with"),
        "the node failed on the tool's own contract, named as a mismatch and by \
         the address a person reading this has to look up: {failure}"
    );
    assert!(
        !failure.contains("ToolCallRefused"),
        "…and not as a refusal: nothing here proposed a call, and nothing bounced: {failure}"
    );

    let entries = run.entries("check");
    let [entry] = entries.as_slice() else {
        panic!("`check` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert!(
        entry["error"].as_str().is_some_and(|text| text
            .contains("ResultMismatch: the arguments `tool.echo` was called with")
            && !text.contains("ToolCallRefused")),
        "`docs/trace.md` §3's `<error name>: <message>`, with the name the \
         surface earns: {entry}"
    );
    assert_eq!(
        provider.requests().len(),
        0,
        "no model was called: this failure is the graph's own"
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

/// A flow attached to an agent as a tool **reaches the model** with the contract
/// grammar 5.4 gives it, and a call to it runs the module.
///
/// Both halves are the same claim, and only the first is easy to lose. PRD 5.1
/// makes the two call sites of a `flow.*` interchangeable and grammar 7.7
/// clause 4 carries session coherence, sync interrupt-freedom and recursion
/// through the attachment — so the compiler analyses flow-as-tool as a call
/// everywhere, and an emitter that then dropped the tool would put an agent the
/// validator accepted on the wire *without* it: no diagnostic at `validate`,
/// none at `build`, and a model that cannot call what the composition attached.
/// So what the provider was offered is read off the recorded request rather than
/// off the emitted TypeScript.
///
/// The second half is PRD resolved q19 and q20, over the case that decides both:
/// the model calls the flow **twice**, with different arguments. Each call is
/// its own instance — its own frame beneath the agent node's, its own store
/// write under its own idempotency key, its own trace — and each is findable as
/// a dispatch record the model call links to. A runtime that reused one site for
/// both would leave the second write deduped away by the backend, which is the
/// failure grammar 9.4's frame exists to prevent.
#[test]
fn a_flow_attached_as_a_tool_runs_one_instance_per_call_under_its_own_frame() {
    let provider = MockProvider::start().expect("a loopback port");
    // Sequential by construction, so the queue order *is* the call order: the
    // loop runs one tool call at a time, and the instance it starts makes its
    // own call before the loop asks again. The loop's own calls pin nothing —
    // which is what makes `tool_calls` and `text` legal answers to them — and
    // the pinned call is the one that ends the node.
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

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says two lines");

    let recorded = provider.requests();
    let offered = &recorded[0];
    assert!(offered.is_valid(), "{:?}", offered.failures());
    assert_eq!(
        offered.tools,
        ["condense"],
        "the attached flow is on the wire under its local name, and the loop \
         call pins nothing beside it: {:?}",
        offered.tools
    );
    let schema = offered
        .body()
        .get("tools")
        .and_then(|tools| tools.get(0))
        .and_then(|tool| tool.get("input_schema"))
        .cloned()
        .expect("the offered tool carries a parameter schema");
    assert_eq!(
        schema["properties"]["passage"]["minLength"],
        json!(1),
        "the flow's `inputs:` is the tool's parameter schema, constraints \
         included (grammar 5.4): {schema}"
    );
    assert_eq!(
        offered.body()["tools"][0]["description"],
        "Condense one passage into a single line.",
        "…and the flow's own `description:` is what the model selects on"
    );
    // The result the model was handed is the instance's `outputs:` — the other
    // half of grammar 5.4's schema pair, read off the turn that carried it.
    let returned = serde_json::to_string(&recorded[2].body()).expect("the request serializes");
    assert!(
        returned.contains("the first line"),
        "the first instance's `outputs:` went back as the tool result: {returned}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the agent node reports what its loop instantiated: {entry}"));
    assert_eq!(dispatched.len(), 2, "{entry}");
    assert!(
        entry["inner"].is_null(),
        "an agent node carries no `inner`: a model-invoked instance is under its \
         own dispatch record (`docs/trace.md` §3, §8): {entry}"
    );

    // q19: the frame is `<tool name>/<call ordinal>` beneath the agent node's
    // own, so the two calls are two effect sites rather than one.
    let keys: Vec<&str> = dispatched
        .iter()
        .map(|record| {
            record["idempotencyKey"]
                .as_str()
                .expect("every dispatch record carries its key")
        })
        .collect();
    assert!(
        keys[0].ends_with("/ask/0/condense/0") && keys[1].ends_with("/ask/0/condense/1"),
        "the call ordinal is what tells the two instances apart: {keys:?}"
    );
    for (ordinal, record) in dispatched.iter().enumerate() {
        assert_eq!(record["index"], json!(ordinal), "{record}");
        assert_eq!(record["target"], "flow.condense", "{record}");
        assert_eq!(record["outcome"], "completed", "{record}");
        assert_eq!(record["attempts"], json!(1), "{record}");
        assert!(
            record["route"].is_null() && record["variant"].is_null(),
            "no `map` routed this dispatch: {record}"
        );
        let inner: Vec<&str> = record["inner"]
            .as_array()
            .unwrap_or_else(|| panic!("the instance's own trace is on its record: {record}"))
            .iter()
            .map(|one| one["node"].as_str().expect("a node id"))
            .collect();
        assert_eq!(inner, ["note", "reduce"], "{record}");
    }

    // q20: the tool-call entry inside the model call records the call, the
    // result the model saw, and a link to the instance by the key its dispatch
    // record carries.
    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the agent node reports its own calls: {entry}"));
    assert_eq!(
        models.len(),
        4,
        "the loop's three calls and the pinned one; the instances' calls are on \
         the instances' own entries (`docs/trace.md` §7.2): {models:?}"
    );
    for (ordinal, call) in models.iter().take(2).enumerate() {
        let asked = call["toolCalls"]
            .as_array()
            .unwrap_or_else(|| panic!("the call that asked for a tool records it: {call}"));
        assert_eq!(asked.len(), 1, "{call}");
        assert_eq!(asked[0]["name"], "condense", "{call}");
        assert_eq!(asked[0]["target"], "flow.condense", "{call}");
        assert_eq!(asked[0]["outcome"], "completed", "{call}");
        assert_eq!(
            asked[0]["instance"], keys[ordinal],
            "the link is the dispatch record's own key, so the join is string \
             equality (`docs/trace.md` §7.3): {call}"
        );
        // …and the third clause of q20: the value the loop handed back, which
        // for this tool is the instance's declared `outputs:`. A reader of the
        // model call can see what the model was answered with without opening
        // the instance the link names.
        assert_eq!(
            asked[0]["result"],
            json!({ "line": if ordinal == 0 { "the first line" } else { "the second line" } }),
            "the tool-call entry records the result the model saw \
             (PRD §9.20, `docs/trace.md` §7.3): {call}"
        );
    }
    for call in models.iter().skip(2) {
        assert!(
            call["toolCalls"].is_null(),
            "a call whose answer asked for no tool carries no record of one: {call}"
        );
    }

    // The consequence q19 is *for*: a store write inside the instance derives
    // its key from that instance's path (grammar 9.4, D104), so two calls write
    // under two keys and the backend dedupes neither.
    let writes: Vec<&Value> = dispatched
        .iter()
        .flat_map(|record| {
            record["inner"][0]["stores"]
                .as_array()
                .unwrap_or_else(|| panic!("the instance's store op is on its own entry: {record}"))
        })
        .collect();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(
        writes[0]["idempotencyKey"],
        json!(format!("{}/note/0", keys[0])),
        "{:?}",
        writes[0]
    );
    assert_eq!(
        writes[1]["idempotencyKey"],
        json!(format!("{}/note/0", keys[1])),
        "{:?}",
        writes[1]
    );
    for write in &writes {
        assert_eq!(
            write["deduped"],
            json!(false),
            "a second real write is not a duplicate of the first: {write}"
        );
    }

    // The same subflow at a `flow:` node runs the same way, which is what makes
    // PRD 5.1's equivalence a comparison rather than an assertion.
    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "line": "it says one line" })),
    ));
    let Some(piped) = harness::invoke(
        "flow-as-tool",
        "flow.pipe",
        &[("passage", "a long passage")],
        &provider,
    ) else {
        return;
    };
    piped.succeeded();
    assert_eq!(piped.outputs()["line"], "it says one line");
}

/// Arguments a flow-as-tool call's `inputs:` refuses come back to the **model**
/// as an error tool result, and the loop turns again (Decision D119, PRD §9.22).
///
/// The three tool surfaces an agent can reach are one surface as far as an
/// argument contract goes (grammar 5.4, 6, 11.5): the schema the model was
/// constrained by is the schema its arguments are parsed with (PRD §9.16), and a
/// call the schema refuses is a call the model can make differently — so it is
/// handed the refusal rather than the node being ended over it. Answering it any
/// other way on one of the three would make "attached as a tool" mean one thing
/// for a `flow.*` and another for everything else.
///
/// Three halves are worth pinning here, and none of them is the happy path:
///
///  * **nothing was instantiated** — the entry's one dispatch record is the
///    *corrected* call's, because the refused one never reached a `runSubflow`;
///  * **the refused call spent no ordinal** — the instance that did run is
///    `condense/0`, the very frame it would have had if the model had got it
///    right the first time. Grammar 9.4's frame counts *invocations*, and a
///    runtime that counted the calls a model **made** would move every key
///    behind a refusal — turning a mistake into a re-keying of work that has
///    nothing to do with it;
///  * **the wire carries it as a refusal** — an `is_error` `tool_result` block,
///    which is the Messages API's own shape for one and which
///    `crates/mock-provider` is strict about.
#[test]
fn arguments_a_flow_tools_inputs_refuses_come_back_to_the_model_as_a_tool_error() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `passage` declares `min_length: 1`, which reaches the model as
        // `minLength` and is what this answer does not satisfy.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("condense", json!({ "passage": "" }))]),
        ),
        // …and this is the model doing the thing a refusal exists for.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        // The instance the corrected call started.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["answer"],
        "it says one line",
        "the node completed: a refusal is a turn of the loop, not the end of it"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        5,
        "the refused call, the corrected one, the instance's own call, the turn \
         that ended the loop, and the pinned one"
    );
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
    }

    // The wire shape, on the surface this fixture speaks: the block answers the
    // `tool_use` id it was asked under — an unanswered one is a request the API
    // refuses — and says it is an error rather than a result.
    let answering = recorded[1].body()["messages"][2].clone();
    assert_eq!(answering["role"], "user", "{answering}");
    let block = &answering["content"][0];
    assert_eq!(block["type"], "tool_result", "{answering}");
    assert_eq!(
        block["is_error"],
        json!(true),
        "the Messages API's own flag for a tool result that is a refusal: {answering}"
    );
    assert!(
        block["content"].as_str().is_some_and(|text| text
            .contains("the arguments `condense` was called with")
            && text.contains("passage")),
        "…and the model is told which field of which tool refused it: {answering}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "completed", "{entry}");

    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the agent node reports its own calls: {entry}"));
    assert_eq!(
        models.len(),
        4,
        "the loop's three calls and the pinned one; the instance's call is on \
         the instance's own entry: {entry}"
    );

    let asked = models[0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the call that asked for the tool records it: {entry}"));
    let [refused] = asked.as_slice() else {
        panic!("one tool call: {entry}");
    };
    assert_eq!(refused["name"], "condense", "{refused}");
    assert_eq!(
        refused["target"], "flow.condense",
        "the tool exists — it is the arguments that did not fit: {refused}"
    );
    assert_eq!(
        refused["outcome"], "refused",
        "`docs/trace.md` §7.3's third outcome: the model was handed the refusal \
         and the node carried on: {refused}"
    );
    assert!(
        refused["instance"].is_null(),
        "…and links to no instance, because the call started none: {refused}"
    );
    assert!(
        refused["result"].is_null(),
        "…and carries no result, because there was nothing to answer with: {refused}"
    );
    // The two spellings of one sentence, pinned **against each other** rather
    // than each against a substring: `docs/trace.md` §7.3 says the record's
    // `<message>` half is byte for byte the model's copy and the `<error name>`
    // half is the envelope the model never sees, so a release that reworded
    // either, or that started handing the model the class too, changes this line
    // (grammar D119, `runtime.ToolCallRefused`).
    let handed = block["content"]
        .as_str()
        .unwrap_or_else(|| panic!("the wire carried the refusal as text: {answering}"));
    assert_eq!(
        refused["error"],
        json!(format!("ToolCallRefused: {handed}")),
        "…and its `error` is the model's own copy under §3's envelope: {refused}"
    );
    assert!(
        !handed.contains("ToolCallRefused"),
        "…which the model's copy does not carry: a class it cannot act on is \
         not part of the sentence it was asked to correct from: {answering}"
    );

    let corrected = &models[1]["toolCalls"][0];
    assert_eq!(corrected["outcome"], "completed", "{entry}");
    assert_eq!(
        corrected["result"],
        json!({ "line": "the only line" }),
        "the corrected call is an ordinary one: {entry}"
    );

    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the corrected call instantiated: {entry}"));
    let [record] = dispatched.as_slice() else {
        panic!("one instance, because only one call reached the flow: {entry}");
    };
    assert_eq!(
        record["idempotencyKey"], corrected["instance"],
        "the link is the dispatch record's own key: {entry}"
    );
    assert!(
        record["idempotencyKey"]
            .as_str()
            .is_some_and(|key| key.ends_with("/ask/0/condense/0")),
        "a refused call spends **no** ordinal: grammar 9.4 counts invocations, \
         and this is the first one — the key a loop whose model called correctly \
         the first time derives: {record}"
    );
}

/// A model that names a tool the agent does not offer is told which tools it
/// has, and calls one (Decision D119).
///
/// The other refusal, and the one with no component behind it: `docs/trace.md`
/// §7.3 makes `target`'s absence a presence rule rather than a convenience — it
/// is on a tool-call record "when the agent offers a tool of that name", so this
/// is the only call in the format whose record carries none, and a runtime that
/// filed one anyway (an empty string, the name echoed back, the record skipped)
/// would break a rule §10.1 says a reader may rely on.
///
/// What changed under D119 is what happens next: the loop used to end the node
/// here, and a name is exactly the thing a model can get right on a second try —
/// so the refusal names the tools that *are* on the wire and the loop turns
/// again. `Outcome::raw` is what scripts it, because `crates/mock-provider`
/// refuses to render a scripted call to a tool the request did not offer (it is
/// a codegen bug in every other test) — and this is the "a response generated
/// code must reject" case its refusal points at.
#[test]
fn a_call_to_a_tool_the_agent_does_not_offer_comes_back_with_the_names_it_has() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::raw(
                200,
                json!({
                    "id": "msg_unoffered",
                    "type": "message",
                    "role": "assistant",
                    "model": SONNET,
                    "content": [{
                        "type": "tool_use",
                        "id": "toolu_summarise",
                        // `agent.answerer` offers `condense` and nothing else.
                        "name": "summarise",
                        "input": { "passage": "a long passage" },
                    }],
                    "stop_reason": "tool_use",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 12, "output_tokens": 34 },
                }),
            ),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says one line");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 5, "the loop went on rather than stopping");
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
    }
    // The turn that answers a `tool_use` the request's own `tools` never
    // declared: legal history on both surfaces, because it is the model's own
    // content replayed (`WIRE-NOTES` (18)).
    let block = recorded[1].body()["messages"][2]["content"][0].clone();
    assert_eq!(block["tool_use_id"], "toolu_summarise", "{block}");
    assert_eq!(block["is_error"], json!(true), "{block}");
    assert!(
        block["content"].as_str().is_some_and(
            |text| text.contains("not one of its tools") && text.contains("`condense`")
        ),
        "the model is told what it may call instead (PRD G3): {block}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "completed", "{entry}");

    let asked = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the call that asked for it records it: {entry}"));
    let [record] = asked.as_slice() else {
        panic!("one tool call: {entry}");
    };
    assert_eq!(
        record["name"], "summarise",
        "the record spells the name the model used, not a name the agent has: {record}"
    );
    assert!(
        record.get("target").is_none(),
        "…and carries no `target`, because there is no component behind it \
         (`docs/trace.md` §7.3): {record}"
    );
    assert_eq!(record["outcome"], "refused", "{record}");
    assert!(
        record["error"]
            .as_str()
            .is_some_and(|text| text.starts_with("ToolCallRefused: ")
                && text.contains("was answered with a call to `summarise`")),
        "…and its `error` is in §3's `<error name>: <message>` shape: {record}"
    );
    assert!(
        record["instance"].is_null() && record["result"].is_null(),
        "nothing ran, so there is neither a link nor a result: {record}"
    );
}

/// A synthesized store tool refuses the arguments its own row does not admit,
/// the model corrects — and a *backend* failure still ends the node.
///
/// Grammar 11.5's tools are the third surface Decision D119 quantifies over, and
/// this is where the split it draws is visible in one place: `top_k` is
/// `1..=100` (grammar 11.4), so `0` is a call the model can make again, while a
/// blob key the local backend cannot hold is the store *failing* and no
/// rephrasing helps. The observable that keeps the first honest is the store
/// record: a refused call never reached a backend, so it left none.
#[test]
fn a_store_tool_refuses_arguments_to_the_model_and_still_fails_the_node_on_a_backend_error() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `top_k` is a bounded integer; `0` is below its minimum.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "notes_search",
                json!({ "query": "anything at all", "top_k": 0 }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "notes_search",
                json!({ "query": "anything at all", "top_k": 1 }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "an answer");

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    let refused = &entry["models"][0]["toolCalls"][0];
    assert_eq!(refused["name"], "notes_search", "{entry}");
    assert_eq!(
        refused["target"], "store.notes",
        "the tool exists — its arguments did not fit: {refused}"
    );
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert!(
        refused["error"].as_str().is_some_and(|text| text
            .contains("the arguments `notes_search` was called with")
            && text.contains("top_k")),
        "the field and the constraint, the way a compiler diagnostic names them: {refused}"
    );
    assert_eq!(
        entry["models"][1]["toolCalls"][0]["outcome"], "completed",
        "…and the corrected call ran: {entry}"
    );
    let records = entry["stores"]
        .as_array()
        .unwrap_or_else(|| panic!("the call that ran left a store record: {entry}"));
    assert_eq!(
        records.len(),
        1,
        "a refused call reached no backend, so it left no store record: {records:?}"
    );
    assert_eq!(records[0]["op"], "search", "{records:?}");

    // The other half. A blob key the local backend cannot address is the store
    // failing, not the contract refusing: the argument schema admits the empty
    // string, so the parse passes and the backend is what says no.
    let backend = MockProvider::start().expect("a loopback port");
    backend.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new("artifacts_get", json!({ "key": "" }))]),
    ));
    let broken = harness::invoke(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &backend,
    )
    .expect("the toolchain was there a moment ago");
    let failure = broken.failed();
    assert!(
        failure.contains("addresses no blob"),
        "the backend's own failure is what ended the node: {failure}"
    );
    assert_eq!(
        backend.requests().len(),
        1,
        "…and the loop stopped there: an execution failure leaves the tool"
    );
    let entries = broken.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    let record = &entry["models"][0]["toolCalls"][0];
    assert_eq!(record["name"], "artifacts_get", "{record}");
    assert_eq!(record["target"], "store.artifacts", "{record}");
    assert_eq!(
        record["outcome"], "failed",
        "not `refused`: the tool ran and the store could not answer: {record}"
    );
}

/// The same split on the **other** wire: a `tool.*`'s declared `input:` refuses,
/// the refusal travels as a `tool` role message, and a tool that exits nonzero
/// still ends the node.
///
/// Chat Completions has no `is_error` — a `tool` role message carries `role`,
/// `content` and `tool_call_id` and nothing else (`WIRE-NOTES` (18)) — so the
/// refusal text *is* the message there, and answering the call at all is the
/// load-bearing half: an unanswered `tool_call_id` is a request the surface
/// refuses. Running this on `agent-openai` is what makes Decision D119 a claim
/// about both wire shapes rather than about the Messages API alone.
#[test]
fn a_tool_definitions_input_refuses_on_the_chat_completions_wire_and_an_exit_code_does_not() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `query` declares `min_length: 1`.
        Script::new(
            LOCAL,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "" }))]),
        ),
        Script::new(
            LOCAL,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        ),
        Script::new(LOCAL, Outcome::text("I have the fact.")),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["feedback"], "a looked-up snippet");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 4, "the loop turned after the refusal");
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
        assert_eq!(call.surface, Surface::OpenAi, "{:?}", call.surface);
    }
    // `[3]`, not `[2]`: this surface leads with a `system` message, so the user
    // turn, the assistant's call and the answer to it sit one further along.
    let answering = recorded[1].body()["messages"][3].clone();
    assert_eq!(answering["role"], "tool", "{answering}");
    assert!(
        answering["tool_call_id"].is_string(),
        "the refusal answers the call it was made under — an unanswered \
         `tool_call_id` is a request this surface refuses: {answering}"
    );
    assert!(
        answering["content"].as_str().is_some_and(|text| text
            .contains("the arguments `lookup` was called with")
            && text.contains("query")),
        "the refusal is the whole of the message on this wire, and it names the \
         tool the way the **wire** offered it — `lookup`, the only spelling a \
         correction could use: {answering}"
    );
    assert!(
        answering.get("is_error").is_none(),
        "…and it carries no flag, because this surface has none to carry: {answering}"
    );

    let entries = run.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    let refused = &entry["models"][0]["toolCalls"][0];
    assert_eq!(refused["name"], "lookup", "{refused}");
    assert_eq!(refused["target"], "tool.lookup", "{refused}");
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert!(
        refused["result"].is_null() && refused["instance"].is_null(),
        "{refused}"
    );
    // The record-to-wire relation `docs/trace.md` §7.3 states, pinned on this
    // surface too: the envelope is the format's, not the wire's, so it does not
    // change with the wire the sentence travelled on.
    let handed = answering["content"]
        .as_str()
        .unwrap_or_else(|| panic!("this surface carries the refusal as text: {answering}"));
    assert_eq!(
        refused["error"],
        json!(format!("ToolCallRefused: {handed}")),
        "the record is the model's own copy under §3's envelope: {refused}"
    );
    assert_eq!(
        entry["models"][1]["toolCalls"][0]["outcome"], "completed",
        "{entry}"
    );

    // `tool.audit` runs `false`, so the child exits 1 with nothing on stdout.
    // That is the tool's execution failing, and no argument would have helped.
    let failing = MockProvider::start().expect("a loopback port");
    failing.enqueue(Script::new(
        LOCAL,
        Outcome::tool_calls(vec![ToolCall::new("audit", json!({ "claim": "a claim" }))]),
    ));
    let ended = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &failing,
    )
    .expect("the toolchain was there a moment ago");
    let failure = ended.failed();
    assert!(
        failure.contains("`false` exited 1"),
        "the child's exit code is what ended the node, and the message names \
         both halves of it — the command as the composition spells it and the \
         status it left. A substring like `false` on its own would be satisfied \
         by any JSON `false` a nested detail happened to render: {failure}"
    );
    assert_eq!(
        failing.requests().len(),
        1,
        "…and no second request was sent: an execution failure leaves the tool"
    );
    let entries = ended.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert_eq!(
        entry["models"][0]["toolCalls"][0]["outcome"], "failed",
        "not `refused`: the tool ran and the process failed: {entry}"
    );
}

/// A model that never corrects spends `max_tool_iterations` and fails the node
/// **holding the last refusal** (Decision D119, D51).
///
/// This is what keeps a bounce statically terminating: a correction is another
/// model call, so the bound that already governed the loop governs refusals too
/// and no new counter is needed. It is also why the node error carries the
/// refusal — the same sentence would otherwise describe a model that called its
/// tools correctly and simply never answered, and a reader would have the
/// symptom with none of the cause.
///
/// `agent.researcher` declares `max_tool_iterations: 2`, so the budget is spent
/// after two refusals rather than after eight.
#[test]
fn a_tool_loop_that_never_corrects_spends_its_budget_and_fails_holding_the_last_refusal() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(
        Script::new(
            SONNET,
            // `tool.lookup` declares `query: { min_length: 1 }`.
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "" }))]),
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
        failure.contains("max_tool_iterations") && failure.contains("bound of 2"),
        "the bound is what stopped it: {failure}"
    );
    assert!(
        failure.contains("its last tool call was refused")
            && failure.contains("the arguments `lookup` was called with"),
        "…and the failure carries what the model kept getting wrong: {failure}"
    );
    assert_eq!(
        provider.requests().len(),
        2,
        "two calls, not eight: a bounce costs the loop an iteration"
    );

    let entries = run.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("both calls are on the entry: {entry}"));
    assert_eq!(models.len(), 2, "{entry}");
    for call in models {
        let asked = call["toolCalls"]
            .as_array()
            .unwrap_or_else(|| panic!("each call asked for the tool: {call}"));
        let [record] = asked.as_slice() else {
            panic!("one tool call per answer: {call}");
        };
        assert_eq!(record["name"], "lookup", "{record}");
        assert_eq!(record["target"], "tool.lookup", "{record}");
        assert_eq!(
            record["outcome"], "refused",
            "every one of them was refused, and none of them ended the node: {record}"
        );
        assert!(record["result"].is_null(), "{record}");
    }
}

/// The same bound spent by a loop whose **last call worked**: the failure says
/// the bound stopped it and claims no refusal (Decision D119, D51).
///
/// The sentence the test above pins is about the call the loop was holding when
/// the budget ran out, and it has to stay about that call. A run whose model
/// mis-typed an argument, was told, corrected, and then simply kept calling is a
/// different thing to diagnose from one that never corrected — and a holder that
/// was only ever written to would describe the first as the second, which is
/// exactly the confusion carrying the refusal was meant to remove.
///
/// `agent.researcher` bounds its loop at 2, so iteration 1 refuses, iteration 2
/// runs the tool for real, and the bound trips on the way into the third.
#[test]
fn a_tool_loop_whose_last_call_worked_spends_its_budget_claiming_no_refusal() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `tool.lookup` declares `query: { min_length: 1 }`.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "" }))]),
        ),
        // …corrected, and answered by `printf` — the tool really ran.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
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
    let failure = run.failed();
    assert!(
        failure.contains("max_tool_iterations") && failure.contains("bound of 2"),
        "the bound is still what stopped it: {failure}"
    );
    assert!(
        !failure.contains("refused"),
        "…and it says nothing about a refusal, because the last call was not one: {failure}"
    );

    let entries = run.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert_eq!(
        entry["models"][0]["toolCalls"][0]["outcome"], "refused",
        "the refusal happened — it is only the *holding* that ended: {entry}"
    );
    assert_eq!(
        entry["models"][1]["toolCalls"][0]["outcome"], "completed",
        "{entry}"
    );
}

/// A refusal does not stop the **other calls of the same answer**: the sibling
/// runs, and both are answered (Decision D119).
///
/// Two provider rules meet here. Both surfaces refuse a request that leaves a
/// `tool_use` id or a `tool_call_id` unanswered, so a loop that stopped at the
/// first refusal would send a turn answering one call of two and be refused on
/// the very next request — which is why the loop *continues* over an answer's
/// calls rather than breaking out of it. With one call per answer the two are
/// indistinguishable, so this is the shape that tells them apart.
///
/// The ordinal is the other half: `condense/0` is the instance the second call
/// started, because the refused call ahead of it spent none (grammar §9.4) — a
/// frame counted from the calls a model *made* would move the key of work that
/// has nothing to do with the mistake.
#[test]
fn a_refused_call_does_not_stop_the_calls_beside_it_in_one_answer() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // One answer, two calls: the first is refused, the second is not.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new("condense", json!({ "passage": "" })),
                ToolCall::new("condense", json!({ "passage": "a long passage" })),
            ]),
        ),
        // The instance the second call started.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says one line");

    let recorded = provider.requests();
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
    }
    // Both calls of the answer are answered, in the order the answer asked, and
    // in one turn: the request that carries them is the loop's next one.
    let answering = recorded[2].body()["messages"][2].clone();
    assert_eq!(answering["role"], "user", "{answering}");
    let blocks = answering["content"]
        .as_array()
        .unwrap_or_else(|| panic!("the turn carries a block per call: {answering}"));
    assert_eq!(blocks.len(), 2, "{answering}");
    assert_eq!(blocks[0]["is_error"], json!(true), "{answering}");
    assert!(
        blocks[1].get("is_error").is_none(),
        "the sibling's result is a result, with no flag on it: {answering}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    let asked = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer's calls are recorded: {entry}"));
    let [refused, completed] = asked.as_slice() else {
        panic!("both calls of the answer are recorded: {entry}");
    };
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert_eq!(completed["outcome"], "completed", "{completed}");
    assert!(
        completed["instance"]
            .as_str()
            .is_some_and(|key| key.ends_with("/ask/0/condense/0")),
        "the refused call ahead of it spent no ordinal, so this one takes the \
         frame it was offered: {completed}"
    );
}

/// A `module:` tool runs at a `function:` node, in this process, and its answer
/// is held to the tool's declared `output:` (grammar 6.1, PRD resolved q48).
///
/// The first half of parity: the only thing that differs from an `exec:` tool at
/// the same node is where the code lives. The result reaches state by name, the
/// trace entry is a node execution like any other, and the declared `env:` is
/// what the authored file read — which the marker in the answer is the evidence
/// for, because `src/tools/stamp.ts` spells a different word when the variable
/// is not there.
#[test]
fn a_module_tool_runs_at_a_function_node_and_reads_the_environment_it_declared() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke(
        "module-tools",
        "flow.direct",
        &[("payload", "a release")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["stamped"],
        json!(format!("a release {}", harness::STAMP_MARKER_VALUE)),
        "the authored module ran, and read the variable its binding declared"
    );
    assert_eq!(
        provider.requests().len(),
        0,
        "a module tool at a `function:` node calls no model"
    );

    let entries = run.entries("stamp");
    let [entry] = entries.as_slice() else {
        panic!("`stamp` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "completed", "{entry}");
}

/// A module's declared `env:` belongs to the **call**, not to the process
/// (grammar 6.1, PRD resolved q49).
///
/// The declaration means "these names hold these values when this
/// implementation runs", and the two halves of that sentence are both checked
/// here because only one of them is visible from the tool's own answer.
/// `tool.sealed` declares `SEALED_MARKER`, a name this process does not have,
/// holding the value of one it does — so:
///
///   * the **marker it answers with** says the value reached the code. Without
///     it the second assertion would pass over an environment that was empty
///     for a reason having nothing to do with containment.
///   * the **subprocess started after it** says the value went nowhere else.
///     `flow.sealed`'s second node is an ordinary `exec:` whose child inherits
///     this process's environment, so a runtime that materialised a binding's
///     `env:` into `process.env` would hand `SEALED_MARKER` to it — and to every
///     other `exec:` tool in the composition, on every flow, for the life of the
///     process. Under `serve`, that is one execution's credentials reaching
///     another's subprocesses.
///
/// It is an assertion about *absence*, which is why the fixture goes out of its
/// way to make the name unavailable by any other route: `printenv` answers with
/// what the child really has, and nothing else in this project sets that name.
#[test]
fn a_module_tools_declared_environment_reaches_the_call_and_no_subprocess_after_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(run) = harness::invoke("module-tools", "flow.sealed", &[], &provider) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["marker"],
        json!(harness::STAMP_MARKER_VALUE),
        "the module was not handed the environment its binding declared"
    );
    assert_eq!(
        run.outputs()["leaked"],
        json!(""),
        "a module binding's declared environment escaped into the process, so every \
         subprocess started afterwards holds it"
    );
    assert_eq!(
        provider.requests().len(),
        0,
        "neither node in this flow calls a model"
    );
}

/// The same tool through an agent's **tool loop**, beside its `exec:` twin —
/// and the two trace records differ in nothing but the component behind them.
///
/// The second half of parity, and the one only a comparison can state.
/// `tool.stamp` and `tool.shout` take the same argument, answer the same shape
/// and carry the same description, so a record shape that depended on the
/// binding would show up here as a difference between two calls of one loop. The
/// refusal is in the same run: `payload` declares `min_length: 1`, and an empty
/// one goes back to the model as a `refused` record rather than ending the node
/// (Decision D119).
#[test]
fn a_module_tool_in_a_loop_records_what_an_exec_tool_records_and_bounces_the_same_refusal() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // One answer, three calls: the module tool refused, the module tool
        // correctly, and its `exec:` twin.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new("stamp", json!({ "payload": "" })),
                ToolCall::new("stamp", json!({ "payload": "a release" })),
                ToolCall::new("shout", json!({ "payload": "a release" })),
            ]),
        ),
        Script::new(SONNET, Outcome::text("Both tools answered.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "stamped": "both tools answered" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "module-tools",
        "flow.assisted",
        &[("payload", "a release")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["stamped"], "both tools answered");

    let recorded = provider.requests();
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
    }
    // The module's refusal reaches the model exactly as any other tool's does:
    // one `tool_result` block, flagged, in the loop's next turn.
    let answering = recorded[1].body()["messages"][2].clone();
    let blocks = answering["content"]
        .as_array()
        .unwrap_or_else(|| panic!("the turn carries a block per call: {answering}"));
    assert_eq!(blocks.len(), 3, "{answering}");
    assert_eq!(blocks[0]["is_error"], json!(true), "{answering}");
    assert!(
        blocks[0]["content"]
            .as_str()
            .is_some_and(|text| text.contains("the arguments `stamp` was called with")),
        "the refusal names the tool the way the wire offered it: {answering}"
    );
    assert!(
        blocks[1]["content"]
            .as_str()
            .is_some_and(|text| text.contains(harness::STAMP_MARKER_VALUE)),
        "…and the call that was admitted really ran the authored module: {answering}"
    );

    let entries = run.entries("assist");
    let [entry] = entries.as_slice() else {
        panic!("`assist` ran once: {entries:?}");
    };
    let calls = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer's calls are recorded: {entry}"));
    let [refused, module, exec] = calls.as_slice() else {
        panic!("all three calls of the answer are recorded: {entry}");
    };
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert_eq!(refused["target"], "tool.stamp", "{refused}");
    assert_eq!(module["outcome"], "completed", "{module}");
    assert_eq!(exec["outcome"], "completed", "{exec}");
    // The comparison the test exists for: same keys, same values but the two
    // that name the component. A record that grew a field for one binding —
    // or lost one — fails here rather than in a reviewer's eye.
    let keys = |record: &Value| {
        let mut held: Vec<String> = record
            .as_object()
            .unwrap_or_else(|| panic!("a record is an object: {record}"))
            .keys()
            .cloned()
            .collect();
        held.sort();
        held
    };
    assert_eq!(
        keys(module),
        keys(exec),
        "a module tool's trace record is an `exec:` tool's, field for field:\n\
         {module}\n{exec}"
    );
    assert_eq!(module["name"], "stamp", "{module}");
    assert_eq!(module["target"], "tool.stamp", "{module}");
    assert_eq!(exec["target"], "tool.shout", "{exec}");
    assert!(
        module["result"].is_null() && exec["result"].is_null(),
        "`docs/trace.md` §11 keeps a tool's answer out of the trace, on both \
         bindings:\n{module}\n{exec}"
    );
}

/// The policy chain of grammar §9.3 governs a module binding, at all three of
/// its members.
///
/// `retry:` is the first: `tool.flaky` throws on its first call and answers its
/// second, and the counter it keeps is module-level state in the graph's own
/// process — the plainest possible evidence that the retry really made a second
/// call rather than replaying the first.
///
/// `timeout:` and `on_error:` are the other two, over a module that never
/// answers and never looks at `context.signal`. Grammar §9.2 bounds one node
/// execution with no exemption for a kind, so the node fails on time and its
/// fallback runs — which is `activities`' `flow.stubborn` with a `module:`
/// binding in place of a `function:` one.
#[test]
fn a_module_tool_is_retried_bounded_and_routed_around_like_every_other_binding() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(retried) = harness::invoke("module-tools", "flow.retried", &[], &provider) else {
        return;
    };
    retried.succeeded();
    assert_eq!(
        retried.outputs()["attempts"],
        json!(2),
        "the first call threw and the node's `retry:` made a second"
    );

    let bounded = harness::invoke("module-tools", "flow.impatient", &[], &provider)
        .expect("the toolchain was there a moment ago");
    bounded.succeeded();
    assert_eq!(
        bounded.outputs()["outcome"],
        "gave up",
        "the node's own `timeout:` stopped a module that ignores the deadline, \
         and `on_error:` scheduled the fallback"
    );
    assert_eq!(
        bounded.visited(),
        ["wait", "give_up"],
        "the fallback is the only way into that node"
    );
}

/// A call to a tool the agent does not offer is corrected on the **Chat
/// Completions** wire too — where the invented name may not be replayed.
///
/// This is the one place the two wire shapes do not agree, and the disagreement
/// is a refusal rather than a preference: Chat Completions re-validates an
/// assistant turn's `tool_calls` against the request's own `tools` and answers
/// `400 Invalid value: '<name>'. This message calls a function the request does
/// not define.`, while the Messages API has no such check (`WIRE-NOTES` (18)).
/// So the turn is rewritten for this surface — the undeclared call is dropped
/// and its refusal travels as a `user` turn — and a runtime that replayed it
/// would turn Decision D119's correction into a provider failure one request
/// later, on a request the model never saw.
#[test]
fn a_call_to_a_tool_the_agent_does_not_offer_is_corrected_on_the_chat_completions_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            LOCAL,
            // `Outcome::raw` for the same reason the Messages twin needs it: the
            // mock refuses to *render* a call to a function the request does not
            // offer, which is a codegen bug everywhere else.
            Outcome::raw(
                200,
                json!({
                    "id": "chatcmpl-mock-unoffered",
                    "object": "chat.completion",
                    "created": 1_700_000_000,
                    "model": LOCAL,
                    "system_fingerprint": "fp_mock",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": Value::Null,
                            "tool_calls": [{
                                "id": "call_mock_summarise",
                                "type": "function",
                                // `agent.researcher` offers `lookup` and `audit`.
                                "function": {
                                    "name": "summarise",
                                    "arguments": "{\"query\":\"a fact\"}",
                                },
                            }],
                            "refusal": Value::Null,
                        },
                        "logprobs": Value::Null,
                        "finish_reason": "tool_calls",
                    }],
                    "usage": {
                        "prompt_tokens": 12,
                        "completion_tokens": 34,
                        "total_tokens": 46,
                    },
                }),
            ),
        ),
        Script::new(
            LOCAL,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
        ),
        Script::new(LOCAL, Outcome::text("I have the fact.")),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["feedback"], "a looked-up snippet");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 4, "the loop went on rather than stopping");
    for call in &recorded {
        assert!(
            call.is_valid(),
            "every request this run sent is one the surface accepts: {:?}",
            call.failures()
        );
    }

    // The request *after* the refusal is the one that would have carried the
    // invented name. It carries the refusal instead, as a turn this surface can
    // hold — and nothing anywhere in it names `summarise` as a function.
    let messages = recorded[1].body()["messages"].clone();
    let listed = messages
        .as_array()
        .unwrap_or_else(|| panic!("the request carries its history: {messages}"));
    assert!(
        listed.iter().all(
            |message| message["tool_calls"].as_array().is_none_or(|calls| calls
                .iter()
                .all(|call| call["function"]["name"] != "summarise"))
        ),
        "no message may name a function the request does not declare: {messages}"
    );
    let refusal = listed
        .iter()
        .find(|message| {
            message["content"]
                .as_str()
                .is_some_and(|text| text.contains("not one of its tools"))
        })
        .unwrap_or_else(|| panic!("the model is told what it called: {messages}"));
    assert_eq!(
        refusal["role"], "user",
        "a `tool` message answering a dropped `tool_call_id` is refused just as \
         loudly, so the refusal travels as the turn that needs no id: {refusal}"
    );
    assert!(
        refusal["content"]
            .as_str()
            .is_some_and(|text| text.contains("`lookup`") && text.contains("`audit`")),
        "…and it names the tools the agent does have (PRD G3): {refusal}"
    );

    // The trace is the wire's rendering apart: a refused call, recorded with no
    // `target`, and the corrected one after it.
    let entries = run.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    let record = &entry["models"][0]["toolCalls"][0];
    assert_eq!(record["name"], "summarise", "{record}");
    assert!(record["target"].is_null(), "{record}");
    assert_eq!(record["outcome"], "refused", "{record}");
    assert_eq!(
        entry["models"][1]["toolCalls"][0]["outcome"], "completed",
        "{entry}"
    );
}

/// One Chat Completions answer carrying **both** an unoffered call and an
/// offered one: both come back to the model, in the two shapes *this surface*
/// has for them. The Messages twin below is the same answer on the other wire,
/// where there is only one shape.
///
/// This is the one place the two wires hand the model materially different
/// things about one answer, and it is forced rather than chosen. The Messages
/// API takes a `tool_result` block per call, in the order the answer asked, so
/// the refusal and the result travel together. Chat Completions refuses a `tool`
/// message whose `tool_call_id` no assistant message asked for (`WIRE-NOTES`
/// (18)), and the assistant turn may not name `summarise` at all — so the
/// refusal cannot be a `tool` message and travels as a `user` turn *after* the
/// `tool` messages, because nothing may come between an assistant turn and the
/// answers to it. The model therefore reads its `lookup` result first and the
/// refusal second.
///
/// With one call per answer the ordering is unobservable, which is why the two
/// single-call tests above do not pin it: it takes a **mixed** answer for the
/// rewrite to have two things to place, and a request this surface refuses is
/// what a runtime that placed them wrongly would produce.
#[test]
fn an_answer_mixing_an_unoffered_call_with_an_offered_one_is_answered_in_both_chat_completions_shapes()
 {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            LOCAL,
            // `Outcome::raw` for the reason the single-call twin needs it: the
            // mock refuses to render a call to a function the request does not
            // offer. The offered call rides along in the same answer.
            Outcome::raw(
                200,
                json!({
                    "id": "chatcmpl-mock-mixed",
                    "object": "chat.completion",
                    "created": 1_700_000_000,
                    "model": LOCAL,
                    "system_fingerprint": "fp_mock",
                    "choices": [{
                        "index": 0,
                        "message": {
                            "role": "assistant",
                            "content": Value::Null,
                            "tool_calls": [
                                {
                                    "id": "call_mock_summarise",
                                    "type": "function",
                                    // `agent.researcher` offers `lookup` and `audit`.
                                    "function": {
                                        "name": "summarise",
                                        "arguments": "{\"passage\":\"a long passage\"}",
                                    },
                                },
                                {
                                    "id": "call_mock_lookup",
                                    "type": "function",
                                    "function": {
                                        "name": "lookup",
                                        "arguments": "{\"query\":\"a fact\"}",
                                    },
                                },
                            ],
                            "refusal": Value::Null,
                        },
                        "logprobs": Value::Null,
                        "finish_reason": "tool_calls",
                    }],
                    "usage": {
                        "prompt_tokens": 12,
                        "completion_tokens": 34,
                        "total_tokens": 46,
                    },
                }),
            ),
        ),
        Script::new(LOCAL, Outcome::text("I have the fact.")),
        Script::new(
            LOCAL,
            Outcome::structured(json!({ "feedback": "a looked-up snippet" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "agent-openai",
        "flow.research",
        &[("goal", "ship it")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["feedback"], "a looked-up snippet");

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        3,
        "one answer corrected both calls, so the loop turned once more and ended"
    );
    for call in &recorded {
        assert!(
            call.is_valid(),
            "every request this run sent is one the surface accepts: {:?}",
            call.failures()
        );
    }

    // The rewritten turn and its two answers, in the order this surface forces.
    let messages = recorded[1].body()["messages"].clone();
    let listed = messages
        .as_array()
        .unwrap_or_else(|| panic!("the request carries its history: {messages}"));
    let assistant = listed
        .iter()
        .find(|message| message["role"] == "assistant")
        .unwrap_or_else(|| panic!("the answer is replayed: {messages}"));
    let calls = assistant["tool_calls"]
        .as_array()
        .unwrap_or_else(|| panic!("…carrying the call this request may name: {assistant}"));
    let [offered] = calls.as_slice() else {
        panic!("only the offered call survives the rewrite: {assistant}");
    };
    assert_eq!(offered["function"]["name"], "lookup", "{assistant}");

    let tail: Vec<&Value> = listed
        .iter()
        .skip_while(|message| message["role"] != "assistant")
        .skip(1)
        .collect();
    let [answered, refusal] = tail.as_slice() else {
        panic!("the answer is followed by exactly its two answers: {messages}");
    };
    assert_eq!(
        answered["role"], "tool",
        "the offered call is answered by id, and first — nothing may come \
         between an assistant turn and the answers to it: {answered}"
    );
    assert_eq!(answered["tool_call_id"], "call_mock_lookup", "{answered}");
    assert_eq!(
        refusal["role"], "user",
        "…and the refusal, which has no id it may be answered under, follows as \
         the turn that needs none: {refusal}"
    );
    assert!(
        refusal["content"]
            .as_str()
            .is_some_and(|text| text.contains("`summarise`")
                && text.contains("`lookup`")
                && text.contains("`audit`")),
        "…naming what was called and what it could have called (PRD G3): {refusal}"
    );

    // The trace is where the two answers are one shape again: both calls of the
    // one answer, in the order the model asked them (`docs/trace.md` §7.3).
    let entries = run.entries("research");
    let [entry] = entries.as_slice() else {
        panic!("`research` ran once: {entries:?}");
    };
    let asked = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer's calls are recorded: {entry}"));
    let [refused, completed] = asked.as_slice() else {
        panic!("both calls of the answer are recorded: {entry}");
    };
    assert_eq!(refused["name"], "summarise", "{refused}");
    assert!(
        refused["target"].is_null(),
        "the one record with no `target`: {refused}"
    );
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert_eq!(completed["name"], "lookup", "{completed}");
    assert_eq!(completed["outcome"], "completed", "{completed}");
    assert!(
        provider.snapshot().is_drained(),
        "the loop ran to its pinned call"
    );
}

/// The same mixed answer on the **Messages** wire, where one shape carries both:
/// the refusal and the result are `tool_result` blocks of a single turn, each
/// under the id its call was asked with, in the order the answer asked.
///
/// This is the half of `WIRE-NOTES` (18) the Chat Completions twin above cannot
/// show, and it is what makes "every call of an answer comes back to the model"
/// a claim about both wires rather than about one. Here the assistant turn is
/// replayed **whole** — `summarise` and all — because this surface does not
/// re-check a history's tool names against the request's `tools`, so the refusal
/// needs no turn of its own and the divergence is entirely the other surface's.
///
/// `Outcome::raw` is what scripts it, for the reason both single-call twins need
/// it: `crates/mock-provider` refuses to *render* a scripted call to a tool the
/// request did not offer.
#[test]
fn an_answer_mixing_an_unoffered_call_with_an_offered_one_is_answered_in_one_messages_turn() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::raw(
                200,
                json!({
                    "id": "msg_mixed",
                    "type": "message",
                    "role": "assistant",
                    "model": SONNET,
                    "content": [
                        {
                            "type": "tool_use",
                            "id": "toolu_summarise",
                            // `agent.answerer` offers `condense` and nothing else.
                            "name": "summarise",
                            "input": { "passage": "a long passage" },
                        },
                        {
                            "type": "tool_use",
                            "id": "toolu_condense",
                            "name": "condense",
                            "input": { "passage": "a long passage" },
                        },
                    ],
                    "stop_reason": "tool_use",
                    "stop_sequence": null,
                    "usage": { "input_tokens": 12, "output_tokens": 34 },
                }),
            ),
        ),
        // The instance the offered call of that same answer started.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says one line");

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        4,
        "one answer corrected both calls, so the loop turned once more and ended"
    );
    for call in &recorded {
        assert!(call.is_valid(), "{:?}", call.failures());
    }

    // The turn the model sent goes back whole, invented name and all — the
    // rewrite the other wire forces has no counterpart here.
    let messages = recorded[2].body()["messages"].clone();
    let replayed = messages[1]["content"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer is replayed: {messages}"));
    let names: Vec<&str> = replayed
        .iter()
        .filter(|block| block["type"] == "tool_use")
        .filter_map(|block| block["name"].as_str())
        .collect();
    assert_eq!(
        names,
        ["summarise", "condense"],
        "both calls are replayed, in the order the answer asked: {messages}"
    );

    // …and one turn answers both, under the ids they were asked with.
    let answering = messages[2].clone();
    assert_eq!(answering["role"], "user", "{answering}");
    let blocks = answering["content"]
        .as_array()
        .unwrap_or_else(|| panic!("the turn carries a block per call: {answering}"));
    let [refusal, result] = blocks.as_slice() else {
        panic!("exactly the answer's two calls are answered: {answering}");
    };
    assert_eq!(refusal["tool_use_id"], "toolu_summarise", "{refusal}");
    assert_eq!(
        refusal["is_error"],
        json!(true),
        "this surface has a flag for a result that is a refusal: {refusal}"
    );
    assert!(
        refusal["content"].as_str().is_some_and(
            |text| text.contains("not one of its tools") && text.contains("`condense`")
        ),
        "…naming what was called and what it could have called (PRD G3): {refusal}"
    );
    assert_eq!(result["tool_use_id"], "toolu_condense", "{result}");
    assert!(
        result.get("is_error").is_none(),
        "the sibling's result is a result, with no flag on it: {result}"
    );

    // The trace is the one shape both wires share (`docs/trace.md` §7.3).
    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    let asked = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the answer's calls are recorded: {entry}"));
    let [refused, completed] = asked.as_slice() else {
        panic!("both calls of the answer are recorded: {entry}");
    };
    assert_eq!(refused["name"], "summarise", "{refused}");
    assert!(
        refused.get("target").is_none(),
        "the one record with no `target`: {refused}"
    );
    assert_eq!(refused["outcome"], "refused", "{refused}");
    assert_eq!(completed["name"], "condense", "{completed}");
    assert_eq!(completed["target"], "flow.condense", "{completed}");
    assert_eq!(completed["outcome"], "completed", "{completed}");
    assert!(
        completed["instance"]
            .as_str()
            .is_some_and(|key| key.ends_with("/ask/0/condense/0")),
        "the refused call ahead of it spent no ordinal, so this one takes the \
         frame it was offered: {completed}"
    );
    assert!(
        provider.snapshot().is_drained(),
        "the loop ran to its pinned call"
    );
}

/// A flow-as-tool call whose **instance** failed fails the agent node, and the
/// instance's trace is the account of why.
///
/// The failure never reaches the model as a plausible result — PRD 5.3 makes the
/// runtime decide every transition, and a subflow that did not run is not a
/// thing the model gets to answer around. What a reader is left with is the
/// whole of what happened inside the boundary, on the dispatch record rather
/// than on the agent node's own entry: `TraceEntry.inner` is a `flow:` node's
/// field (`docs/trace.md` §3, §8), and an agent node that claimed one would
/// report an instance it never ran.
#[test]
fn a_flow_tool_call_whose_instance_failed_fails_the_agent_node_carrying_its_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        // `agent.summariser` declares `line: { min_length: 1 }`, so the instance
        // fails at its own contract (PRD 5.2) after its store node has written.
        Script::new(SONNET, Outcome::structured(json!({ "line": "" }))),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("flow.condense"),
        "the failure names the flow the model called: {failure}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert!(
        entry["inner"].is_null(),
        "the instance is on its dispatch record, never on the agent's entry: {entry}"
    );
    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the instance that failed is still reported: {entry}"));
    let [record] = dispatched.as_slice() else {
        panic!("one call, one record: {entry}");
    };
    assert_eq!(record["outcome"], "failed", "{record}");
    assert!(
        record["error"]
            .as_str()
            .is_some_and(|text| text.contains("agent.summariser")),
        "{record}"
    );
    let inner = record["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("a failed instance carries its trace too: {record}"));
    assert_eq!(
        inner
            .iter()
            .map(|one| (
                one["node"].as_str().expect("a node id"),
                one["outcome"].as_str().expect("an outcome")
            ))
            .collect::<Vec<_>>(),
        [("note", "completed"), ("reduce", "failed")],
        "the store node's write happened and the model call is what failed: {record}"
    );
    assert_eq!(
        entry["models"][0]["toolCalls"][0]["outcome"], "failed",
        "…and the call the model made says so too: {entry}"
    );
    assert_eq!(
        entry["models"][0]["toolCalls"][0]["instance"], record["idempotencyKey"],
        "…and still links to the instance it ran: {entry}"
    );
    assert!(
        entry["models"][0]["toolCalls"][0]["result"].is_null(),
        "…and carries no result, because the model saw none: the failure left \
         the tool (`docs/trace.md` §7.3): {entry}"
    );
}

/// Two calls to one flow-tool in a **single** model answer get two ordinals.
///
/// The happy path above spends two loop turns to make its two calls, which
/// exercises the counter but not the case grammar 9.4 names first: "distinct
/// calls in one tool loop get distinct instance paths". A loop that assigned the
/// ordinal per *turn* rather than per call would give both calls
/// `condense/0`, and nothing else in the trace would say so — the link on each
/// tool-call entry would still resolve, to the same record, and the two children
/// would share one idempotency key so the second's store write would be deduped
/// away as a repeat of work it never repeated.
#[test]
fn two_flow_tool_calls_in_one_model_answer_get_distinct_ordinals() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // One answer, two calls. The loop runs them in the order the answer
        // asked, one at a time, so the two instances' model calls follow.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new("condense", json!({ "passage": "the first passage" })),
                ToolCall::new("condense", json!({ "passage": "the second passage" })),
            ]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the first line" })),
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

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.ask",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("both calls instantiated: {entry}"));
    let keys: Vec<&str> = dispatched
        .iter()
        .map(|record| record["idempotencyKey"].as_str().expect("a key"))
        .collect();
    assert!(
        keys[0].ends_with("/ask/0/condense/0") && keys[1].ends_with("/ask/0/condense/1"),
        "the ordinal counts invocations, not answers (grammar 9.4): {keys:?}"
    );
    assert_ne!(keys[0], keys[1], "two calls, two effect sites: {keys:?}");
    assert_eq!(dispatched[0]["index"], json!(0), "{entry}");
    assert_eq!(dispatched[1]["index"], json!(1), "{entry}");

    // One model call asked for both, so both records are on **one**
    // `toolCalls` array, in the order the answer asked (`docs/trace.md` §7).
    let asked = entry["models"][0]["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("the one call that asked for both records both: {entry}"));
    assert_eq!(asked.len(), 2, "{entry}");
    assert_eq!(asked[0]["instance"], keys[0], "{entry}");
    assert_eq!(asked[1]["instance"], keys[1], "{entry}");

    // And the consequence: two keys means two writes the backend kept.
    let writes: Vec<&Value> = dispatched
        .iter()
        .flat_map(|record| {
            record["inner"][0]["stores"]
                .as_array()
                .unwrap_or_else(|| panic!("the instance's store op is on its own entry: {record}"))
        })
        .collect();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_ne!(
        writes[0]["idempotencyKey"], writes[1]["idempotencyKey"],
        "{writes:?}"
    );
    for write in &writes {
        assert_eq!(write["deduped"], json!(false), "{write}");
    }
}

/// An agent-node `retry:` **restarts** the call ordinals, so the second
/// attempt's Nth call reuses the first attempt's Nth key.
///
/// Grammar 9.4 and `docs/trace.md` §8 both state this openly as an at-least-once
/// compromise rather than a property, which is exactly why it needs a test: a
/// change that made the counter outlive the attempt — hoisting the `Map` out of
/// the agent call, say — would silently re-key every retried composition's child
/// flows, and nothing about the run would look different. What the reuse costs
/// is inside the child, so that is where it is read: the second attempt's store
/// write is `deduped`, against a key the first attempt already applied.
#[test]
fn an_agent_node_retry_restarts_the_flow_tool_call_ordinals() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Attempt one: a tool call, its instance, and a pinned answer that fails
        // `agent.answerer`'s own `answer: { min_length: 1 }` contract.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(SONNET, Outcome::structured(json!({ "answer": "" }))),
        // Attempt two: the same shape, answered properly.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.retried",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says one line");

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("a retry is one node execution, so one entry: {entries:?}");
    };
    assert_eq!(entry["attempts"], json!(2), "{entry}");

    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("both attempts' instances are reported: {entry}"));
    assert_eq!(
        dispatched.len(),
        2,
        "one call per attempt, and the failed attempt's is not dropped: {entry}"
    );
    assert_eq!(
        dispatched[0]["idempotencyKey"], dispatched[1]["idempotencyKey"],
        "the ordinals restart with the attempt, so the second attempt's first \
         call derives the first attempt's first key (grammar 9.4): {entry}"
    );
    assert!(
        dispatched[0]["idempotencyKey"]
            .as_str()
            .is_some_and(|key| key.ends_with("/ask/0/condense/0")),
        "…and it is ordinal `0` that both derive: {entry}"
    );

    // What positional reuse actually costs, read where it costs it.
    let writes: Vec<&Value> = dispatched
        .iter()
        .flat_map(|record| {
            record["inner"][0]["stores"]
                .as_array()
                .unwrap_or_else(|| panic!("the instance's store op is on its own entry: {record}"))
        })
        .collect();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(
        writes[0]["deduped"],
        json!(false),
        "the first attempt's write is the one that happened: {}",
        writes[0]
    );
    assert_eq!(
        writes[1]["deduped"],
        json!(true),
        "…and the second derives the same key, so the backend refuses it — the \
         at-least-once compromise grammar 9.4 states openly: {}",
        writes[1]
    );
    assert!(
        provider.snapshot().is_drained(),
        "both attempts ran their whole loop"
    );
}

/// A refusal ahead of a call does not move that call's key **across a retry**,
/// which is the property grammar 9.4's "invoked" is worth counting for.
///
/// The two rules above meet here and would fight if the ordinal counted the
/// calls a model *made*: a node `retry:` restarts the ordinals, and the two
/// attempts of one retried node are not obliged to make the same mistakes. This
/// run's first attempt calls `condense` wrongly and then correctly; its second
/// gets it right first time. Under a count of attempted calls the two attempts'
/// instances would derive `condense/1` and `condense/0` — two keys for one piece
/// of work, so every side effect of the child re-fires on a retry whose only
/// difference was that the model needed no correction. Under grammar 9.4 as it
/// is written they derive one key, and the observable is inside the child, where
/// the reuse costs something: the second attempt's store write is `deduped`
/// against the key the first already applied (Decision D119, PRD resolved q22).
#[test]
fn a_refusal_does_not_move_the_key_a_retried_attempt_re_derives() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Attempt one: a refused call, the correction, its instance, and a
        // pinned answer that fails `agent.answerer`'s own
        // `answer: { min_length: 1 }` contract.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("condense", json!({ "passage": "" }))]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(SONNET, Outcome::structured(json!({ "answer": "" }))),
        // Attempt two: no mistake to correct, and answered properly.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "the only line" })),
        ),
        Script::new(SONNET, Outcome::text("I have the line.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "it says one line" })),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.retried",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "it says one line");

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("a retry is one node execution, so one entry: {entries:?}");
    };
    assert_eq!(entry["attempts"], json!(2), "{entry}");

    // The refusal happened, and is on the record of the call that asked for it:
    // without this the rest of the test would pass on a run that never refused.
    let refused: Vec<&Value> = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("both attempts' calls are reported: {entry}"))
        .iter()
        .flat_map(|call| call["toolCalls"].as_array().into_iter().flatten())
        .filter(|record| record["outcome"] == "refused")
        .collect();
    let [record] = refused.as_slice() else {
        panic!("exactly one call was refused, in the first attempt: {entry}");
    };
    assert!(
        record["instance"].is_null(),
        "…and it started no instance to key: {record}"
    );

    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("both attempts' instances are reported: {entry}"));
    assert_eq!(
        dispatched.len(),
        2,
        "one instance per attempt: the refused call reached no flow, so it filed \
         no record: {entry}"
    );
    assert_eq!(
        dispatched[0]["idempotencyKey"], dispatched[1]["idempotencyKey"],
        "the attempt that needed a correction and the attempt that did not \
         derive one key: {entry}"
    );
    assert!(
        dispatched[0]["idempotencyKey"]
            .as_str()
            .is_some_and(|key| key.ends_with("/ask/0/condense/0")),
        "…and it is ordinal `0`, because grammar 9.4 counts invocations and the \
         refused call was none: {entry}"
    );

    // What that buys, read where a moved key would have cost it.
    let writes: Vec<&Value> = dispatched
        .iter()
        .flat_map(|record| {
            record["inner"][0]["stores"]
                .as_array()
                .unwrap_or_else(|| panic!("the instance's store op is on its own entry: {record}"))
        })
        .collect();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_eq!(
        writes[0]["deduped"],
        json!(false),
        "the first attempt's write is the one that happened: {}",
        writes[0]
    );
    assert_eq!(
        writes[1]["deduped"],
        json!(true),
        "…and the retried attempt's is refused as a repeat, which it would not \
         have been had the first attempt's refusal moved the key: {}",
        writes[1]
    );
    assert!(
        provider.snapshot().is_drained(),
        "both attempts ran their whole loop"
    );
}

/// A `map` dispatching to an agent that carries a flow-tool: the child instance
/// nests beneath the **item's** frame, not the map node's.
///
/// This is the one entry shape that holds both dispatch carriers at once
/// (`docs/trace.md` §5) — the fan-out's own `dispatches` and the tool loop's
/// `toolDispatches` — and the shape the `index` row spends a paragraph on,
/// because two items' first calls both carry `index: 0` and only the
/// `idempotencyKey` tells them apart. A call site that threaded the map node's
/// own path instead of the item's would give both children one key, and every
/// effect the second dispatched would be deduped away as a repeat of the first's.
#[test]
fn a_map_dispatched_agents_flow_tool_calls_nest_under_the_items_frame() {
    let provider = MockProvider::start().expect("a loopback port");
    // `max_concurrency: 1`, so the queue order is the dispatch order: item 0's
    // whole loop, then item 1's.
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
        Script::new(SONNET, Outcome::text("I have it.")),
        Script::new(SONNET, Outcome::structured(json!({ "reply": "first" }))),
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
        Script::new(SONNET, Outcome::text("I have it.")),
        Script::new(SONNET, Outcome::structured(json!({ "reply": "second" }))),
    ]);

    let Some(run) = harness::invoke_with(
        "flow-as-tool",
        "flow.fan",
        &json!({ "questions": [{ "text": "the first?" }, { "text": "the second?" }] }),
        &harness::environment(&provider),
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["replies"], json!(["first", "second"]));

    let entries = run.entries("fan");
    let [entry] = entries.as_slice() else {
        panic!("`fan` ran once: {entries:?}");
    };

    // Carrier one: the fan-out's own records, one per source item.
    let items = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the map reports what it dispatched: {entry}"));
    assert_eq!(items.len(), 2, "{entry}");
    for (index, item) in items.iter().enumerate() {
        assert_eq!(item["index"], json!(index), "{item}");
        assert_eq!(item["target"], "agent.batcher", "{item}");
        assert!(
            item["inner"].is_null(),
            "an `agent.*` target runs no instance of its own: {item}"
        );
    }

    // Carrier two: what the *models* dispatched, one record per item's call,
    // each beneath its own item's frame.
    let tools = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the items' tool loops instantiated: {entry}"));
    assert_eq!(tools.len(), 2, "{entry}");
    let keys: Vec<&str> = tools
        .iter()
        .map(|record| record["idempotencyKey"].as_str().expect("a key"))
        .collect();
    assert!(
        keys[0].ends_with("/fan/0/0/condense/0") && keys[1].ends_with("/fan/0/1/condense/0"),
        "the item index is a frame of its own, between the map node's and the \
         tool call's (grammar 9.4): {keys:?}"
    );
    assert_eq!(
        (tools[0]["index"].clone(), tools[1]["index"].clone()),
        (json!(0), json!(0)),
        "both are the first call of their own loop, so the two records repeat an \
         `index` and `idempotencyKey` is what tells them apart (`docs/trace.md` \
         §5): {entry}"
    );

    // The items' model calls are the map node's (`docs/trace.md` §7.2), and each
    // tool-call entry still links to the child its own item ran.
    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the map node carries its items' calls: {entry}"));
    let links: Vec<&Value> = models
        .iter()
        .filter_map(|call| call["toolCalls"].as_array())
        .flatten()
        .map(|asked| &asked["instance"])
        .collect();
    assert_eq!(links, [&json!(keys[0]), &json!(keys[1])], "{entry}");

    // And the effect the frames exist for: two items, two keys, neither deduped.
    let writes: Vec<&Value> = tools
        .iter()
        .flat_map(|record| {
            record["inner"][0]["stores"]
                .as_array()
                .unwrap_or_else(|| panic!("the instance's store op is on its own entry: {record}"))
        })
        .collect();
    assert_eq!(writes.len(), 2, "{writes:?}");
    assert_ne!(
        writes[0]["idempotencyKey"], writes[1]["idempotencyKey"],
        "{writes:?}"
    );
    for write in &writes {
        assert_eq!(write["deduped"], json!(false), "{write}");
    }
}

/// A node deadline that catches a tool call **mid-flight** records neither half
/// of it — and, crucially, records no *empty* half either.
///
/// `docs/trace.md` §5.3 is this shape, and both keys it names are presence rules
/// a reader is entitled to rely on (§10.1): `toolCalls` is documented as never
/// empty, so `"toolCalls": []` is a value the format promises cannot occur. The
/// loop attaches the key with its first record rather than with the empty array
/// it fills, and that is the whole of what keeps the promise: the window between
/// "the model asked for a tool" and "the tool answered" is exactly where a
/// raced deadline lands, and it is wide — a subflow is a whole graph.
#[test]
fn a_node_deadline_that_abandons_a_tool_call_records_neither_half_of_it() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "condense",
                json!({ "passage": "a long passage" }),
            )]),
        ),
        // The child's own model call, answered long after `ask`'s 300ms budget
        // is spent — so the tool call is in flight when the deadline fires.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "too late" })).after(Duration::from_millis(2_000)),
        ),
    ]);

    let Some(run) = harness::invoke(
        "flow-as-tool",
        "flow.rush",
        &[("question", "what does it say?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("300ms budget was spent"),
        "the node's own deadline is what ended it (grammar 9.2): {failure}"
    );

    let entries = run.entries("ask");
    let [entry] = entries.as_slice() else {
        panic!("`ask` ran once: {entries:?}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert!(
        entry["toolDispatches"].is_null(),
        "no outcome resolved for the one call, so the key is absent rather than \
         empty (`docs/trace.md` §5.3): {entry}"
    );
    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the loop's first call came back and is reported: {entry}"));
    assert_eq!(models.len(), 1, "{entry}");
    assert!(
        models[0]["toolCalls"].is_null(),
        "…and the call it asked for resolved nothing, so `toolCalls` is absent \
         rather than the empty array `docs/trace.md` §7 promises never to write: \
         {entry}"
    );
    let serialized = serde_json::to_string(entry).expect("the entry serializes");
    assert!(
        !serialized.contains("\"toolCalls\":[]"),
        "an empty `toolCalls` is a value a reader may treat as impossible \
         (`docs/trace.md` §10.1): {serialized}"
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
/// built project embeds (`src/cel.ts`). The driver is committed rather than
/// written inline, because it carries a JSON reader that keeps `1` and `1.0`
/// apart — the distinction the corpus's int64 cases exist to pin, and one
/// `JSON.parse` throws away.
///
/// This is the **Bun** column, which is the runtime PRD §9.18 makes a generated
/// project's default. The same driver is run over the same corpus under the Node
/// fallback by gate 15 of `crates/compose-core/tests/generated_code_gates.rs`,
/// which is why it lives in the shared toolchain fixture rather than beside this
/// file: what an evaluator built on `BigInt` and `RegExp` answers is the engine's,
/// so one runtime alone would leave the other's readers unchecked.
///
/// It takes the runtime rather than the install ([`harness::bun_command`] rather
/// than [`harness::installed`]) because that is all it needs: the driver imports
/// `src/cel.ts` and nothing else, and that module has no imports of its own. What
/// it must not do is take neither — [`harness::bun`] panics when Bun is absent,
/// and a machine without it is owed the skip `support/toolchain.rs` documents
/// rather than one test out of step with the rest of this binary.
#[test]
fn the_generated_cel_evaluator_agrees_with_the_validator_on_the_conformance_corpus() {
    let Some(mut bun) = harness::bun_command() else {
        return;
    };

    let built = harness::build("bounded-cycle", "local");
    built.succeeded();

    let corpus = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("crates/")
        .join("compose-core/tests/fixtures/cel-conformance");
    let driver = harness::toolchain::root().join("cel-conformance.mjs");

    let output = bun
        .arg(&driver)
        .arg(built.root())
        .arg(&corpus)
        .output()
        .expect("bun runs the generated evaluator");
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
    // The homogeneous form declares no `route_by:`, so its items are not a
    // union and no dispatch has a variant to name — neither a `route` (there is
    // one target) nor a `variant` (there is no discriminator).
    let dispatched = run.entries("work")[0]["dispatches"].clone();
    for record in dispatched.as_array().expect("the map records its dispatch") {
        assert!(
            record["route"].is_null() && record["variant"].is_null(),
            "a homogeneous dispatch names neither a route nor a variant: {record}"
        );
    }
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
    // The same fact read off the other field it decides, which `docs/trace.md`
    // §5 states as a presence rule: this record is written when the dispatch is
    // *issued*, before the instance it names has run anything, so it carries no
    // `inner` — and it would carry none if the route pointed at a `flow.*`,
    // which rule 7 admits and which is the case a reader would otherwise expect
    // a subgraph trace from.
    assert!(
        records[2].get("inner").is_none(),
        "a detached dispatch's record is written before its delivery runs, so there \
         is no inner trace to carry: {}",
        records[2]
    );
    // …and which variant each item *was*, which the route alone does not say.
    // A named route's tag is the variant tag, so the two agree there; the
    // catch-all's is `$default`, and without this the one item that fell
    // through to it would be the one item whose variant the trace never named
    // (PRD §7 M2: "which map variant a discriminator chose" as trace data).
    assert_eq!(
        records
            .iter()
            .map(|record| record["variant"].clone())
            .collect::<Vec<_>>(),
        [
            json!("auto_fixable"),
            json!("needs_human"),
            json!("duplicate"),
            json!("auto_fixable"),
        ],
        "every dispatch names the discriminator its item carried: {dispatched}"
    );
    assert!(
        records[3]["error"]
            .as_str()
            .is_some_and(|error| error.contains("agent.fixer")),
        "the skipped item says what was wrong with it: {}",
        records[3]
    );
    // The detached delivery's own model call is the *unscripted* one this
    // fixture is built around, so the mock refuses it — and that refusal is not
    // on this entry. `models` is what the join observed (`docs/trace.md` §5.1,
    // §7.2), and a delivery the join never waited for would put its record here
    // or not depending on when the provider answered.
    // `a_detached_deliverys_effects_stay_off_the_map_nodes_entry` decides the
    // same rule with the ordering pinned; this is it on the path where the
    // delivery fails.
    let routed = run.entries("route")[0].clone();
    for call in routed["models"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        assert!(
            call.get("refused").is_none(),
            "the map node's entry carries the joined items' calls, and the only refused \
             call this run makes is the detached delivery's: {call}"
        );
    }

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

/// A detached delivery's own effects stay **off** the map node's trace entry
/// (`docs/trace.md` §5.1, §6, §7.2).
///
/// The entry is written when the join finishes, and by Decision D94 the join
/// does not wait for a detached delivery. A model call or a store op made
/// through the node's own collectors would therefore be on that entry or not
/// depending on when the sink answered — two runs of one composition producing
/// two trace documents, which is the one thing a versioned format cannot do
/// (`docs/trace.md` §10). The record the format gives such a dispatch is
/// `outcome: "detached"` with `attempts: 0`, and it says what the join observed:
/// nothing.
///
/// The fixture decides the race rather than leaving it to the scheduler. The one
/// joined item's model call is answered after 400ms and the delivery's at once,
/// so the delivery has certainly answered — the assertion below reads its
/// request off the provider — well before the join returns. Without that
/// ordering an absence would pass on a runtime that shares the collectors,
/// whenever the sink happened to be the slower of the two.
///
/// Model calls are the half a composition can reach: a detached route's target
/// is a `node:`, and only an agent with attached stores would record store ops
/// through one. Both travel on the same two fields of the delivery's context, so
/// the reachable half is what holds the rule.
#[test]
fn a_detached_deliverys_effects_stay_off_the_map_nodes_entry() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "findings": [
                    { "kind": "auto_fixable", "file": "a.rs", "hint": "rename it" },
                    { "kind": "duplicate", "of": "issue-7" },
                ],
            })),
        ),
        // The joined item, deliberately slow…
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "patch": "patch-a" })).after(Duration::from_millis(400)),
        )
        .matching("a.rs"),
        // …and the detached delivery's own call, deliberately immediate.
        Script::new(HAIKU, Outcome::structured(json!({ "text": "a duplicate" })))
            .matching("issue-7"),
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

    // The delivery was made and was answered: without this the absence below
    // would be evidence about a call that never happened.
    let requests = provider.requests();
    assert_eq!(
        requests.len(),
        3,
        "the sorter, the joined item and the detached delivery all reached the provider: \
         {requests:?}"
    );
    assert!(
        requests
            .iter()
            .any(|request| request.body_text.contains("issue-7")),
        "the detached delivery's own model call is one of them: {requests:?}"
    );

    // …and exactly one of the three is on the map node's entry: the joined
    // item's. `models` is what the join observed, and the delivery is what it
    // did not.
    let entry = run.entries("route")[0].clone();
    let calls = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the map node made a model call: {entry}"))
        .clone();
    assert_eq!(
        calls.len(),
        1,
        "the map node's entry carries the joined item's model call and not the detached \
         delivery's, which the join never waited for (`docs/trace.md` §5.1): {entry}"
    );
    assert!(
        entry.get("stores").is_none(),
        "and nothing else the delivery did either: {entry}"
    );
    // The dispatch record is the whole account of that delivery, which is the
    // other half of the same rule.
    let detached = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the map records what it dispatched: {entry}"))[1]
        .clone();
    assert_eq!(detached["outcome"], json!("detached"), "{detached}");
    assert_eq!(detached["attempts"], json!(0), "{detached}");
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

/// A dispatched instance's trace is under **its own** dispatch record and
/// nowhere else, including when its failure is what ended the map node
/// (`docs/trace.md` §3, §8).
///
/// The format has two nesting fields and they say two different things:
/// `TraceEntry.inner` is the instance a `flow:` node ran, and
/// `DispatchRecord.inner` is the instance a `map` dispatched. A `map` is not a
/// `flow:` node, so its own entry carries no `inner` however its dispatches
/// went — otherwise a reader walking a trace for every subgraph run counts one
/// instance twice, and counts *which* one by a rule the document does not state:
/// the lowest-indexed failure, on the `on_error: fail` path alone.
///
/// The path is the one that reaches it. The failing item's error travels out of
/// the fan-out inside an `ItemFailure`, which the node's failure wraps, so a
/// walk of the `cause` chain that does not stop at that boundary finds the
/// dispatched instance's `SubflowFailure` underneath and attaches its trace to
/// the map node's own entry.
#[test]
fn a_dispatched_instances_trace_stays_under_its_own_record() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "tasks": [{ "steps": ["alpha"] }, { "steps": ["gamma"] }],
            })),
        ),
        // Item 0 runs its instance through: one step, then the roll-up.
        Script::new(HAIKU, Outcome::structured(json!({ "part": "made-alpha" }))).matching("alpha"),
        Script::new(HAIKU, Outcome::structured(json!({ "line": "line-0" }))).matching("made-alpha"),
        // Item 1's own step is refused, which fails the instance's inner map,
        // the instance, the item, and — `on_item_error` and `on_error` both
        // being `fail` here — the outer map node and the run.
        Script::new(HAIKU, Outcome::server_error()).matching("gamma"),
    ]);

    let Some(run) = harness::invoke("fanout", "flow.nested", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.failed();

    let entries = run.entries("work");
    assert_eq!(entries.len(), 1, "one map node, one entry: {entries:?}");
    let entry = entries[0].clone();
    assert_eq!(entry["outcome"], json!("failed"), "{entry}");

    // The account of the fan-out survives the failure, one record per source
    // item, and the failed one carries the instance it dispatched.
    let records = entry["dispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("a failed map still says what it dispatched: {entry}"))
        .clone();
    assert_eq!(records.len(), 2, "one record per source item: {entry}");
    assert_eq!(records[1]["outcome"], json!("failed"), "{}", records[1]);
    let dispatched = records[1]["inner"]
        .as_array()
        .unwrap_or_else(|| {
            panic!(
                "the failed item's instance is under its record: {}",
                records[1]
            )
        })
        .clone();
    assert!(
        dispatched
            .iter()
            .any(|held| held["node"] == "steps" && held["outcome"] == "failed"),
        "…and it is that instance's own trace, ending at the node it aborted at: {dispatched:?}"
    );

    // The one this test exists for: the same instance is not *also* on the map
    // node's entry, where §3 says a `flow:` node's instance is.
    assert!(
        entry.get("inner").is_none(),
        "a `map` node's entry carries no `inner`: a dispatched instance is under its \
         own record (`docs/trace.md` §3, §8), and this one reports it twice: {entry}"
    );
}

/// A child instance's fan-out stays on the **child's** entries, and a `flow:`
/// node that ran it carries no `dispatches` of its own.
///
/// The mirror of the test above, across the other module boundary. `docs/trace.md`
/// §3 gives `dispatches` to `map` nodes — "a node that is not a `map` never
/// carries the key" — and a `flow:` node dispatches nothing: it runs one
/// instance, whose whole account is its `inner`, the failing `map` entry and
/// that entry's own records included.
///
/// The path that breaks it is a `cause` chain read one boundary too far. A child
/// whose fan-out failed raises an `ItemFailure` carrying the records, its map
/// node's failure wraps that, the instance's `SubflowFailure` wraps *that*, and
/// the caller's own failure wraps the lot — so a walk looking for an
/// `ItemFailure` and not stopping at the subflow boundary finds the child's
/// records and files them under the caller. The trace then reports one fan-out
/// twice, in two places that mean different things, and claims the outer node
/// dispatched instances it never had.
#[test]
fn a_child_instances_fan_out_stays_inside_the_flow_nodes_inner_trace() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({
                "tasks": [
                    { "title": "first", "body": "do alpha" },
                    { "title": "second", "body": "do beta" },
                ],
            })),
        ),
        Script::new(HAIKU, Outcome::structured(json!({ "result": "did-alpha" })))
            .matching("do alpha"),
        // `agent.worker` declares `result: { min_length: 1 }`, so item 1 fails
        // its own contract — and with `on_item_error` and the map node's
        // `on_error` both at their default `fail`, the child instance fails.
        Script::new(HAIKU, Outcome::structured(json!({ "result": "" }))).matching("do beta"),
    ]);

    let Some(run) = harness::invoke("fanout", "flow.enclose", &[("goal", "ship it")], &provider)
    else {
        return;
    };
    run.failed();

    let entries = run.entries("inside");
    let [entry] = entries.as_slice() else {
        panic!("one `flow:` node, one entry: {entries:?}");
    };
    assert_eq!(entry["outcome"], json!("failed"), "{entry}");

    // The one this test exists for.
    assert!(
        entry.get("dispatches").is_none(),
        "a `flow:` node dispatched nothing, so it carries no `dispatches`: the \
         records under its child's failure are the **child's** map node's \
         (`docs/trace.md` §3): {entry}"
    );
    assert!(
        entry.get("toolDispatches").is_none(),
        "…and no model dispatched anything here either: {entry}"
    );

    // …and the records really are reported, one boundary in, on the entry that
    // owns them.
    let inner = entry["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("a failed instance still carries its trace: {entry}"));
    let child = inner
        .iter()
        .find(|held| held["node"] == "work")
        .unwrap_or_else(|| panic!("the child's map node has an entry: {entry}"));
    assert_eq!(child["outcome"], json!("failed"), "{child}");
    let records = child["dispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("a failed map still says what it dispatched: {child}"));
    assert_eq!(
        records
            .iter()
            .map(|record| (
                record["index"].as_u64().expect("an index"),
                record["outcome"].as_str().expect("an outcome").to_string(),
            ))
            .collect::<Vec<_>>(),
        [(0, "completed".to_string()), (1, "failed".to_string())],
        "one record per source item, on the node that made them: {child}"
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
    assert_eq!(
        work[0]["dispatches"],
        json!([]),
        "and the key is present and empty rather than absent: `docs/trace.md` §5 \
         makes an empty array \"nothing was dispatched\" and an absent key \
         \"nothing resolved\", and this map dispatched nothing"
    );
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
/// It **stops** at `approve`, which is a `human:` node: an in-process invocation
/// has no resume surface, so the pause is where the run ends (grammar 8.7,
/// PRD 5.11). That is asserted rather than worked around — the run gets all the
/// way there, the trace holds every step it took, and the node it stopped at
/// says what it is waiting for and where an answer would come from.
///
/// The fixer answering **item 0** is delayed past the one answering item 3, so
/// the two patches complete in the reverse of source order. What `summarize` is
/// then sent is the whole claim of PRD 5.6's replay guarantee, observed on the
/// wire rather than inferred.
#[test]
fn the_triage_fanout_example_routes_every_finding_and_joins_them_in_source_order() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // `agent.triage` attaches `store.docs`, so it has a tool — the
        // synthesized `docs_search` of grammar 11.5 — and an agent with tools
        // makes a loop call before the pinned one. This is that call, and the
        // model here searches nothing.
        Script::new(SONNET, Outcome::text("I have what I need.")),
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
        //
        // Two calls per dispatched instance, for `agent.triage`'s reason one
        // level down: `agent.fixer` holds `tool.checkout`, a `builtin: files`
        // binding (grammar 5.5, 6.1), so
        // each instance makes a loop call before its pinned one. The pair is
        // told apart by which item the request carries rather than by position,
        // since two instances run concurrently and either may ask first.
        Script::new(HAIKU, Outcome::text("I have read enough.")).matching("src/a.rs"),
        Script::new(
            HAIKU,
            Outcome::structured(json!({ "patch": "patch-a", "explanation": "renamed" }))
                .after(Duration::from_millis(300)),
        )
        .matching("src/a.rs"),
        Script::new(HAIKU, Outcome::text("I have read enough.")).matching("src/b.rs"),
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

    // Everything up to the pause an in-process invocation cannot answer.
    let failure = run.failed();
    assert!(
        failure.contains("is waiting for a human and this run has no way to answer"),
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

    let Some(run) = harness::run(
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
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
fn an_attached_store_synthesizes_its_tool_surface_in_the_model_request() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
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
    // Grammar 11.5's third row, which is the one no other assertion here
    // reaches: a `blob` synthesizes three tools rather than two.
    assert!(
        call.tools.contains(&"artifacts_get".to_string())
            && call.tools.contains(&"artifacts_list".to_string())
            && call.tools.contains(&"artifacts_put".to_string()),
        "a blob store under the `read_write` default synthesizes all three: {:?}",
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
fn agent_access_read_withholds_the_write_tool() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
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
    // The comparison that makes this a narrowing rather than a kind that has no
    // write tool: `store.notes` is the same `kind: vector` at the `read_write`
    // default, on the same attachment list, and its upsert **is** on offer.
    assert!(
        call.tools.contains(&"notes_upsert".to_string())
            && call.tools.contains(&"notes_search".to_string()),
        "the same kind under `read_write` offers both: {:?}",
        call.tools
    );
}

/// A model that **calls** a synthesized store tool reaches the same backend the
/// store-op nodes do, with the arguments it supplied (grammar 11.5, PRD 5.8).
///
/// The two tests above decide the tool *list*, which is what a request offers. A
/// list is not a surface: the store tools carry the op's own parameter row with
/// the CEL positions replaced by what the model sends (`runStoreTool` in
/// `src/stores.ts`), and a mapping that dropped or misnamed one of those would
/// leave every tool still on offer and still named correctly. So the model here
/// drives its own loop, and every claim is decided by what a **later** tool call
/// answers rather than by the arguments going out — nothing is staged behind
/// these stores, so what comes back is what a previous call put there.
///
/// Which is what makes each parameter load-bearing:
///
/// * `key`, `query`, `top_k` and `limit` are *required* by the backend, so a
///   mapping that failed to deliver one fails the node naming it;
/// * `value` and `metadata` default to empty and `filter` and `prefix` default to
///   "match everything", so each is decided by a read that must **not** answer
///   with everything: a `filter:` on the metadata of the second document alone, a
///   `prefix:` matching one blob key of two, and a `limit:` of one against two.
///
/// Two runs of one project, because two of the three surfaces would need more
/// than `max_tool_iterations` in one node otherwise (Decision D51's default is
/// 8), and because a `blob` and a `vector` fail in different places.
#[test]
fn an_agent_calling_a_synthesized_store_tool_reaches_the_backend_with_its_arguments() {
    let Some(project) = harness::scratch_project("store-tools") else {
        return;
    };

    // --- The `vector` and `kv` surfaces, in one node's loop. ---
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Two documents that differ in **both** their text and their metadata,
        // so the search below can only answer with the second one if the
        // `filter:` really reached the backend: the query is the *first*
        // document's text, which outscores the second on similarity.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "notes_upsert",
                json!({
                    "key": "note-1",
                    "value": "the quick brown fox",
                    "metadata": { "source": "the model" },
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "notes_upsert",
                json!({
                    "key": "note-2",
                    "value": "a lazy dog",
                    "metadata": { "source": "somebody else" },
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "notes_search",
                json!({
                    "query": "the quick brown fox",
                    "top_k": 1,
                    "filter": { "source": "somebody else" },
                }),
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
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "prefs_get",
                json!({ "key": "preferences" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ),
    ]);

    let run = harness::run_into(
        &project,
        "stores",
        "flow.answer",
        &[("question", "what is it?")],
        None,
        &harness::environment(&provider),
    );
    run.succeeded();

    // Every tool call is a store record on the node's own trace entry, marked as
    // the surface that ran it (PRD 5.8's two modes). The node also runs three
    // store-op nodes, but those are other nodes' entries — `ask`'s records are
    // the agent's alone.
    let records = store_records(&run, "ask");
    assert!(
        records.iter().all(|record| record["via"] == "tool"),
        "an agent's store ops are recorded as tool calls: {records:?}"
    );
    assert_eq!(
        records
            .iter()
            .map(|record| format!("{} {}", record["store"], record["op"]))
            .collect::<Vec<_>>(),
        [
            "\"store.notes\" \"upsert\"",
            "\"store.notes\" \"upsert\"",
            "\"store.notes\" \"search\"",
            "\"store.prefs\" \"set\"",
            "\"store.prefs\" \"get\"",
        ],
        "in the order the model called them"
    );

    // The search: one match, and it is the document the `filter:` names rather
    // than the one the `query:` scores highest. A dropped `filter` answers
    // `note-1` here, and a dropped `metadata` on the upserts answers nothing.
    let matches = records[2]["answer"]["matches"]
        .as_array()
        .unwrap_or_else(|| panic!("a `search` records what it answered: {}", records[2]));
    assert_eq!(matches.len(), 1, "{matches:?}");
    assert_eq!(matches[0]["id"], "note-2", "the filter chose the match");
    assert_eq!(
        matches[0]["text"], "a lazy dog",
        "…and the document carries the `value:` the upsert sent"
    );
    assert_eq!(
        matches[0]["metadata"]["source"], "somebody else",
        "…with the `metadata:` it sent, which is what the filter matched on"
    );

    // The kv round trip, which is `value` in the other direction: what `set`
    // sent is what `get` answered, field for field.
    assert_eq!(records[4]["answer"]["found"], json!(true), "{}", records[4]);
    assert_eq!(records[4]["answer"]["value"]["theme"], "dark");
    assert_eq!(records[4]["answer"]["value"]["verbosity"], "high");
    assert!(
        provider.snapshot().is_drained(),
        "the loop made exactly the calls the script staged"
    );

    // --- The `blob` surface, in a second run of the same project. ---
    let second = MockProvider::start().expect("a loopback port");
    second.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "artifacts_put",
                json!({ "key": "from-the-model", "value": "a note the model wrote" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "artifacts_put",
                json!({ "key": "zz-something-else", "value": "and another" }),
            )]),
        ),
        // Two keys are now there, so a `prefix:` that reaches the backend
        // answers with one of them and a dropped one answers with both.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "artifacts_list",
                json!({ "prefix": "from", "limit": 10 }),
            )]),
        ),
        // …and a `limit:` of one against a prefix that matches both.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "artifacts_list",
                json!({ "prefix": "", "limit": 1 }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "artifacts_get",
                json!({ "key": "from-the-model" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "another answer" })),
        ),
    ]);

    let blobs = harness::run_into(
        &project,
        "stores",
        "flow.answer",
        &[("question", "and now?")],
        None,
        &harness::environment(&second),
    );
    blobs.succeeded();

    let written = store_records(&blobs, "ask");
    assert_eq!(
        written
            .iter()
            .map(|record| format!("{} {}", record["store"], record["op"]))
            .collect::<Vec<_>>(),
        [
            "\"store.artifacts\" \"put\"",
            "\"store.artifacts\" \"put\"",
            "\"store.artifacts\" \"list\"",
            "\"store.artifacts\" \"list\"",
            "\"store.artifacts\" \"get\"",
        ],
    );
    assert_eq!(
        written[2]["answer"]["keys"],
        json!(["from-the-model"]),
        "the `prefix:` narrowed the listing to one of the two keys: {}",
        written[2]
    );
    assert_eq!(
        written[3]["answer"]["keys"],
        json!(["from-the-model"]),
        "…and the `limit:` cut the unfiltered listing to one: {}",
        written[3]
    );
    assert_eq!(written[4]["answer"]["found"], json!(true), "{}", written[4]);
    assert_eq!(
        written[4]["answer"]["value"], "a note the model wrote",
        "the blob answers with the `value:` the model put there"
    );
    assert!(
        second.snapshot().is_drained(),
        "the loop made exactly the calls the script staged"
    );
}

/// One node's store records, flattened across its trace entries in step order.
fn store_records(run: &harness::Run, node: &str) -> Vec<Value> {
    run.entries(node)
        .into_iter()
        .filter_map(|entry| entry["stores"].as_array().cloned())
        .flatten()
        .collect()
}

/// The other two kinds, over the backends PRD 5.8 promises with no
/// infrastructure: a `vector` store indexed and searched, and a `blob` written,
/// read back and listed.
///
/// The vector half is the one that needs a **provider**: grammar 11.2 makes
/// `embed.provider` the connection that turns text into a vector, so a `search`
/// is a store op with a real HTTP round trip inside it. `crates/mock-provider`
/// answers that surface deterministically (it is the one route there that is not
/// scripted), which is what makes "the document I just indexed is the one the
/// search finds" an assertion rather than a coin flip.
///
/// What the search answered is read off the **trace**, because that is the only
/// place it is observable: a node's result is readable from an edge guard and a
/// `map.over` and nowhere else (Decision D42), and PRD 5.8 says a store read is
/// recorded — so the record is both the mechanism and the evidence.
#[test]
fn a_vector_and_a_blob_store_round_trip_through_the_local_backends() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(run) = harness::run(
        "stores",
        "flow.ground",
        &[("text", "the quick brown fox")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    let outputs = run.outputs();

    // The blob round trip. Every channel here declares a default the ops never
    // produce, so a backend that did nothing could not answer this.
    assert_eq!(outputs["stored_id"], "doc-1");
    assert_eq!(outputs["blob_found"], json!(true));
    assert_eq!(outputs["blob_value"], "the quick brown fox");
    assert_eq!(outputs["blob_keys"], json!(["note.txt"]));

    // …and the vector one, from the record the read left behind.
    let searched = run.entries("find");
    let matches = searched[0]["stores"][0]["answer"]["matches"]
        .as_array()
        .unwrap_or_else(|| panic!("a `search` records what it answered: {}", searched[0]));
    assert_eq!(matches.len(), 1, "{matches:?}");
    assert_eq!(matches[0]["id"], "doc-1");
    assert_eq!(matches[0]["text"], "the quick brown fox");
    assert_eq!(
        matches[0]["metadata"]["source"], "fixture",
        "a store that declares a `metadata_schema:` carries it on every match \
         (Decision D114)"
    );
    assert!(
        matches[0]["score"]
            .as_f64()
            .is_some_and(|score| score > 0.5),
        "the document scores against its own text: {}",
        matches[0]
    );

    // Both halves of the replay discipline, in the record itself (PRD 5.8): a
    // read keeps what it answered, and a write keeps the idempotency key of
    // grammar 9.4 plus whether the backend had already applied it.
    let indexed = run.entries("index");
    let write = &indexed[0]["stores"][0];
    assert_eq!(write["effect"], "write");
    assert_eq!(write["deduped"], json!(false));
    assert!(
        write["idempotencyKey"]
            .as_str()
            .is_some_and(|key: &str| key.ends_with("/index/0")),
        "the key is the execution id and the store node's flattened instance \
         path (grammar 9.4): {write}"
    );
    assert_eq!(searched[0]["stores"][0]["effect"], "read");
}

/// A `scope: session` store outlives the execution that wrote it, and a run with
/// no session identity is refused at start naming the store (PRD 5.8,
/// grammar 11.3, 13.2).
///
/// Two runs of one built project, because that is where a store's data lives:
/// cross-session memory is a session-scoped store plus a trigger-supplied
/// session key, and nothing else. The third run is the negative half — the same
/// flow with no `--session` at all, which grammar 13.2 says fails at start
/// naming the store rather than silently addressing an unnamed partition.
#[test]
fn a_session_scoped_store_outlives_the_execution_that_wrote_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(project) = harness::scratch_project("session") else {
        return;
    };
    let environment = harness::environment(&provider);

    let first = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "the first thing")],
        Some("session-a"),
        &environment,
    );
    first.succeeded();
    assert_eq!(
        first.outputs()["seen_before"],
        json!(false),
        "a session with nothing in it yet"
    );

    let second = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "the second thing")],
        Some("session-a"),
        &environment,
    );
    second.succeeded();
    assert_eq!(
        second.outputs()["seen_before"],
        json!(true),
        "the first run's write survived its execution"
    );
    assert_eq!(second.outputs()["recalled"]["note"], "the first thing");

    // A different session key is a different partition, not a different store.
    let other = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "somebody else's")],
        Some("session-b"),
        &environment,
    );
    other.succeeded();
    assert_eq!(
        other.outputs()["seen_before"],
        json!(false),
        "one session does not read another's"
    );

    let refused = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "nowhere to put it")],
        None,
        &environment,
    );
    let failure = refused.failed();
    assert!(
        failure.contains("store.memory") && failure.contains("scope: session"),
        "the refusal names the store, which is what the author has to look at: {failure}"
    );
    assert!(
        failure.contains("where one is declared, it answered the empty string for this invocation"),
        "…and names all three ways a run arrives with no identity, the one this \
         run did not take included: a trigger's declared `session_key:` that \
         evaluated to nothing is an identity-less run whose author has already \
         taken both of the other two remedies, and a message offering only those \
         two would send that reader to look at a line already there (PRD G3): \
         {failure}"
    );
    // …and it is a **usage** failure, which is what a caller branches on.
    // Grammar 11.3 likens the check to env-ref presence (§4.3), and `main.rs`'s
    // table puts that in the `2` column: `1` is "a run produced no answer" and
    // `2` is "the command could not be run at all". Nothing ran here — the
    // argument is missing, and no repetition of the same command can supply it —
    // so a supervisor that retries `1` and reports `2` must be told `2`. The
    // sibling below is the same mistake spelled differently, and the two
    // agreeing is the whole claim.
    assert_eq!(
        refused.output.status.code(),
        Some(2),
        "a `--session` a flow needs and did not get is an argument to fix, not a \
         run to retry: {failure}"
    );
    let mistyped = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("nope", "not a field of this flow")],
        Some("session-a"),
        &environment,
    );
    let mistyped_failure = mistyped.failed();
    assert_eq!(
        mistyped.output.status.code(),
        Some(2),
        "the neighbouring argument mistake grammar 13.2 names in the same \
         sentence: {mistyped_failure}"
    );
}

/// A declared `manual` trigger's `session_key:` remaps the `--session` the CLI
/// was given, and the run addresses the partition it names (grammar 13.2, 11.3).
///
/// The remap is the one thing declaring a `manual` trigger adds — §13's preamble:
/// writing it out "lets it carry a `description:` or a `session_key:` remap; it
/// neither enables nor restricts anything the CLI would otherwise do" — so a
/// compiler that accepted the key and emitted nothing would leave a composition
/// that namespaces sessions per tenant silently sharing one partition, with
/// every run reporting success.
///
/// Which is why this is three runs rather than two. The first writes through
/// `flow.remember`, which **no** trigger names, so its session identity is the
/// argument itself: the note lands at `tenant/s1`. The second reads through
/// `flow.tenant_recall`, whose trigger declares `session_key: "tenant/" +
/// payload.session`, and finds it while passing `--session s1` — which it can
/// only do if the expression ran. The third is the control: the same argument
/// through the untriggered flow finds nothing, so the second run's hit is the
/// remap's doing rather than two spellings of one partition.
#[test]
fn a_declared_manual_trigger_remaps_the_session_the_cli_was_given() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(project) = harness::scratch_project("session-remap") else {
        return;
    };
    let environment = harness::environment(&provider);

    let raw = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "written where the remap points")],
        Some("tenant/s1"),
        &environment,
    );
    raw.succeeded();
    assert_eq!(raw.outputs()["seen_before"], json!(false));

    let remapped = harness::run_into(
        &project,
        "stores",
        "flow.tenant_recall",
        &[],
        Some("s1"),
        &environment,
    );
    remapped.succeeded();
    assert_eq!(
        remapped.outputs()["seen_before"],
        json!(true),
        "`--session s1` reached `tenant/s1`, which is what the trigger's \
         `session_key:` says it addresses"
    );
    assert_eq!(
        remapped.outputs()["recalled"]["note"],
        "written where the remap points"
    );

    let unremapped = harness::run_into(
        &project,
        "stores",
        "flow.remember",
        &[("note", "somewhere else entirely")],
        Some("s1"),
        &environment,
    );
    unremapped.succeeded();
    assert_eq!(
        unremapped.outputs()["seen_before"],
        json!(false),
        "the same argument through a flow no trigger names addresses `s1` itself, \
         so the run above found what it found by evaluating the remap"
    );
}

/// A retried `flow:` node reports **every** instance it ran, so a store write an
/// earlier attempt really made is in the trace beside the duplicate the backend
/// refused (PRD 5.3, 5.8, grammar 8.5, 9.4).
///
/// The gap this closes is not "one entry is missing". An attempt of a `flow:`
/// node is a whole instance — its own nodes, its own routing decisions, its own
/// effects — and a report that kept only the last one describes an execution
/// that never wrote anything: what survives is a `save` marked `deduped`, with
/// nothing in the trace it could be a duplicate *of*, and a `set` whose value is
/// in the store with no record of the write that put it there.
///
/// Both ways out of the retry are decided, over one pair of flows, because the
/// two are different code paths — the one that recovers joins the failed
/// attempts to the answer's instance, and the one that does not has no answer to
/// join to:
///
/// * the model fails the first attempt's `confirm` and answers the second, so
///   the node completes with two attempts; and
/// * the model fails both, so the node fails with two.
///
/// The store record is what makes each an assertion about *effects* rather than
/// about entry counts: `save` runs before `confirm` in the instance, so it
/// really happened on the attempt that failed, and grammar 9.4 derives the same
/// idempotency key on the next one — which is the same key, and `deduped` only
/// on the second.
#[test]
fn a_retried_subflow_reports_the_instance_of_every_attempt() {
    // The attempt that recovers.
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::server_error()),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "line": "written down" })),
        ),
    ]);
    let Some(recovered) = harness::invoke(
        "stores",
        "flow.reingest",
        &[("note", "a thing to remember")],
        &provider,
    ) else {
        return;
    };
    recovered.succeeded();
    assert_eq!(recovered.outputs()["ingested"], "written down");

    let entries = recovered.entries("sub");
    assert_eq!(entries.len(), 1, "one node, one entry: {entries:?}");
    let entry = &entries[0];
    assert_eq!(entry["outcome"], "completed", "{entry}");
    assert_eq!(entry["attempts"], json!(2), "{entry}");

    let inner = entry["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("the node reports the instances it ran: {entry}"));
    assert_eq!(
        inner
            .iter()
            .map(|held| (held["node"].clone(), held["outcome"].clone()))
            .collect::<Vec<_>>(),
        [
            (json!("save"), json!("completed")),
            (json!("confirm"), json!("failed")),
            (json!("save"), json!("completed")),
            (json!("confirm"), json!("completed")),
        ],
        "the failed attempt's whole instance, then the one that answered: {entry}"
    );

    let writes: Vec<&Value> = inner
        .iter()
        .filter_map(|held| held["stores"].as_array())
        .flatten()
        .collect();
    assert_eq!(writes.len(), 2, "both attempts wrote: {entry}");
    assert_eq!(writes[0]["store"], "store.memory", "{}", writes[0]);
    assert_eq!(writes[0]["op"], "set", "{}", writes[0]);
    assert_eq!(
        writes[0]["deduped"],
        json!(false),
        "the first attempt's write is the one that happened: {}",
        writes[0]
    );
    assert_eq!(
        writes[1]["deduped"],
        json!(true),
        "…and the second is the duplicate the backend refused: {}",
        writes[1]
    );
    assert_eq!(
        writes[0]["idempotencyKey"], writes[1]["idempotencyKey"],
        "the key a retry re-derives is the same key (grammar 9.4): {entry}"
    );
    assert!(
        provider.snapshot().is_drained(),
        "both attempts reached the model"
    );

    // The attempt that does not recover: the node fails, and the account of what
    // ran inside the boundary is all this trace has.
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::server_error()),
        Script::new(SONNET, Outcome::server_error()),
    ]);
    let Some(exhausted) = harness::invoke(
        "stores",
        "flow.reingest",
        &[("note", "a thing to remember")],
        &provider,
    ) else {
        return;
    };
    exhausted.failed();

    let entries = exhausted.entries("sub");
    assert_eq!(entries.len(), 1, "one node, one entry: {entries:?}");
    let entry = &entries[0];
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert_eq!(entry["attempts"], json!(2), "{entry}");
    assert_eq!(
        entry["inner"]
            .as_array()
            .unwrap_or_else(|| panic!("a failed node reports its instances too: {entry}"))
            .iter()
            .map(|held| (held["node"].clone(), held["outcome"].clone()))
            .collect::<Vec<_>>(),
        [
            (json!("save"), json!("completed")),
            (json!("confirm"), json!("failed")),
            (json!("save"), json!("completed")),
            (json!("confirm"), json!("failed")),
        ],
        "neither attempt's instance is dropped because the node failed: {entry}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A route fails over to its next member on a declared condition, and the trace
/// records that it did (PRD 5.9).
#[test]
fn a_route_fails_over_to_its_next_member_and_the_trace_records_it() {
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
    // PRD 5.9 asks for failover to be "deterministic runtime behavior recorded
    // in the trace (`served by model.fast, fallback #1`)", so the assertion is
    // against the record rather than against the sentence the terminal prints:
    // the `model.*` the agent named, the member that answered, its ordinal, and
    // the condition the member it replaced refused with.
    let entries = run.entries("ask");
    let calls: Vec<&Value> = entries
        .iter()
        .filter_map(|entry| entry["models"].as_array())
        .flatten()
        .collect();
    assert_eq!(calls.len(), 1, "one model call was made: {entries:?}");
    let call = calls[0];
    assert_eq!(call["model"], "model.default", "{call}");
    assert_eq!(call["servedBy"], "model.fast", "{call}");
    assert_eq!(call["fallback"], 1, "served by fallback #1: {call}");
    let failovers = call["failovers"]
        .as_array()
        .unwrap_or_else(|| panic!("the refusals on the way are recorded: {call}"));
    assert_eq!(failovers.len(), 1, "{call}");
    assert_eq!(failovers[0]["model"], "model.smart", "{call}");
    assert_eq!(
        failovers[0]["condition"], "rate_limit",
        "the record names the `route_on:` condition, not just that something failed: {call}"
    );

    let rendered = run.stderr();
    assert!(
        rendered.contains("served by model.fast, fallback #1"),
        "the human report is PRD 5.9's own sentence: {rendered}"
    );
}

/// A failure condition outside `route_on:` fails the node instead of failing
/// over — otherwise the declaration would mean nothing.
#[test]
fn a_condition_outside_route_on_fails_the_node_instead_of_failing_over() {
    let provider = MockProvider::start().expect("a loopback port");
    // `model.default` routes on rate_limit, overloaded, and timeout — not on
    // server_error.
    provider.enqueue(Script::new(SONNET, Outcome::server_error()));

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();

    let recorded = provider.requests();
    assert_eq!(
        recorded.len(),
        1,
        "the fallback was never tried: {:?}",
        recorded.iter().map(|call| &call.model).collect::<Vec<_>>()
    );
    assert_eq!(recorded[0].model, SONNET);
    // And it failed for the reason the fixture staged, rather than for some
    // earlier reason that would have made the one-request assertion vacuous.
    assert!(
        failure.contains("500") && !failure.contains("model.fast"),
        "the run failed on the refusal itself, and named no fallback: {failure}"
    );
}

/// A route condition raised by the **second rung** of the structured-output
/// mechanism ladder still fails over, exactly as one raised by the first would
/// (PRD 5.9, PRD §9 resolved q53).
///
/// The seam between the two ladders, in the direction that must stay open. The
/// mechanism ladder is deliberately *inside* one route member's call — it sends
/// the same request the other way when the endpoint says it does not carry the
/// rung that was asked — and everything it does not recognize is re-thrown into
/// the route ladder untouched. So a member whose working rung then answers 429 is
/// a `rate_limit` like any other, and `route_on:` has to reach the next member.
///
/// Nothing but a test holds that. Nothing here declares which errors the
/// mechanism ladder passes through: it is one `throw` of the caught value, and a
/// later change that wrapped the second rung's refusal the way the double refusal
/// below is wrapped would turn every such 429 into a plain `Error` whose
/// condition classifies as nothing — and a route declaring `rate_limit` would
/// silently stop failing over on it.
///
/// `model.smart`'s endpoint is a lagging gateway, so the first member ladders
/// inside itself before it ever reaches the scripted 429: three requests, one
/// trace record, one failover.
#[test]
fn a_route_condition_the_mechanism_ladder_uncovered_still_fails_over() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::NativeRejected);
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
    assert_eq!(run.outputs()["answer"], "42");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| (
                request.model.as_str(),
                request.unsupported,
                request
                    .structured_output
                    .as_ref()
                    .map(StructuredOutput::mechanism)
            ))
            .collect::<Vec<_>>(),
        [
            (
                SONNET,
                Some(OutputMechanism::Native),
                Some(OutputMechanism::Native)
            ),
            (SONNET, None, Some(OutputMechanism::ForcedTool)),
            (HAIKU, None, Some(OutputMechanism::Native)),
        ],
        "the first member laddered to the rung its endpoint has, was rate limited \
         there, and the route moved on — to a member whose own endpoint carries \
         the native parameter, since the memo is a fact about a pairing and not \
         about the route"
    );
    assert!(recorded.iter().all(RecordedRequest::is_valid));

    let entries = run.entries("ask");
    let calls: Vec<&Value> = entries
        .iter()
        .filter_map(|entry| entry["models"].as_array())
        .flatten()
        .collect();
    assert_eq!(calls.len(), 1, "one model call, laddering and all");
    let call = calls[0];
    assert_eq!(call["servedBy"], "model.fast", "{call}");
    assert_eq!(call["fallback"], 1, "{call}");
    assert_eq!(call["failovers"][0]["model"], "model.smart", "{call}");
    assert_eq!(
        call["failovers"][0]["condition"], "rate_limit",
        "the 429 the second rung answered is the condition `route_on:` names, \
         not something the mechanism ladder absorbed: {call}"
    );
    assert_eq!(
        call["outputMechanism"], "native",
        "…and the record names the mechanism that *answered*, which is the \
         fallback's rather than the rung the member before it settled on: {call}"
    );
    assert!(provider.snapshot().is_drained());
}

/// …and the double refusal is the other direction of the same seam: it fails the
/// node where it stands, without spending the route (PRD 5.9, PRD §9 resolved
/// q53).
///
/// An endpoint carrying neither mechanism is a statement about *that endpoint*,
/// and grammar 12.2 routes on infrastructure conditions only — so the diagnostic
/// is a plain `Error` that classifies as no condition and ends the node. A route
/// that failed over on it would be worse than wasteful: the next member would
/// answer, the run would pass, and an endpoint a compiled graph cannot use at all
/// would be invisible until the day it was first in the ladder alone.
///
/// The fixture's fallback has a working answer waiting, so "the route was not
/// spent" is checkable rather than vacuous: an untouched queue is a member that
/// was never called.
#[test]
fn an_endpoint_carrying_neither_mechanism_fails_the_node_without_failing_over() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.personality(SONNET, Personality::BothRejected);
    provider.enqueue(Script::new(
        HAIKU,
        Outcome::structured(json!({ "answer": "42" })),
    ));

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    let failure = run.failed();
    assert!(
        failure.contains("nothing to configure"),
        "the node failed on the double-refusal diagnostic: {failure}"
    );
    assert!(
        !failure.contains("model.fast") && !failure.contains("spent its route"),
        "…and named no fallback, because none was tried: {failure}"
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| (request.model.as_str(), request.unsupported))
            .collect::<Vec<_>>(),
        [
            (SONNET, Some(OutputMechanism::Native)),
            (SONNET, Some(OutputMechanism::ForcedTool)),
        ],
        "both rungs of the first member, and nothing else"
    );
    assert_eq!(
        provider.snapshot().queues[HAIKU],
        1,
        "the fallback's answer is untouched: `route_on:` never saw a condition"
    );

    let entries = run.entries("ask");
    let call = &entries[0]["models"][0];
    assert_eq!(call["refused"]["model"], "model.smart", "{call}");
    assert_eq!(
        call["refused"]["condition"],
        Value::Null,
        "a refusal that is no `route_on:` condition is recorded as one: {call}"
    );
    assert_eq!(
        call["failovers"],
        json!([]),
        "…and the route recorded no member spent: {call}"
    );
}

/// The rest of `route_on:`'s vocabulary, each against the wire shape it names
/// (grammar 12.2, PRD 5.9).
///
/// `rate_limit` has its own test above because it is the one that also decides
/// what the trace records. These three are about *classification*: an
/// `overloaded` is a status the surface chooses (529 on Anthropic, 503 on
/// OpenAI), a `server_error` is any other 5xx, and a `timeout` is not a status at
/// all — it is a request that never gets an answer, which reaches the ladder as a
/// transport failure rather than as a refusal the provider wrote. A ladder that
/// classified any of them as something outside `route_on:` would fail the node,
/// so a run that produces the fallback's answer is the assertion.
///
/// With `rate_limit`'s own test, that is grammar 12.2's whole enumeration staged
/// positively — which is what keeps the negative test below from being the only
/// place a condition appears. `a_condition_outside_route_on_fails_the_node…`
/// stages a 500 against a route that does **not** declare `server_error`, and it
/// would go on passing if the classifier stopped recognising 5xx at all: the row
/// here is what fails in that case, because `flow.ask_resiliently` routes on
/// `server_error` and nothing else.
///
/// All three run in one project: the ladder is per model call, so three
/// conditions need three runs, and building once keeps this a test about failover
/// rather than about the toolchain.
#[test]
fn every_declared_condition_is_recognized_from_the_shape_the_provider_answers_with() {
    let Some(project) = harness::scratch_project("failover-conditions") else {
        return;
    };
    for (flow, condition, staged) in [
        ("flow.ask", "overloaded", Outcome::overloaded()),
        (
            "flow.ask",
            "timeout",
            Outcome::timeout(Duration::from_millis(50)),
        ),
        // The one condition `model.default` withholds, through the route that
        // declares it: same two members, same order, a 500 from the first.
        (
            "flow.ask_resiliently",
            "server_error",
            Outcome::server_error(),
        ),
    ] {
        let provider = MockProvider::start().expect("a loopback port");
        provider.enqueue_all([
            Script::new(SONNET, staged),
            Script::new(HAIKU, Outcome::structured(json!({ "answer": condition }))),
        ]);

        let run = harness::run_into(
            &project,
            "model-failover",
            flow,
            &[("question", "what is it?")],
            None,
            &harness::environment(&provider),
        );
        run.succeeded();
        assert_eq!(
            run.outputs()["answer"],
            condition,
            "the fallback's answer is what the run produced, so the ladder went on"
        );

        let call = run
            .entries("ask")
            .into_iter()
            .find_map(|entry| {
                entry["models"]
                    .as_array()
                    .and_then(|calls| calls.first().cloned())
            })
            .unwrap_or_else(|| panic!("the model call is recorded ({condition})"));
        assert_eq!(call["servedBy"], "model.fast", "{call}");
        assert_eq!(
            call["failovers"][0]["condition"], condition,
            "the refusal is classified as the condition `route_on:` names it by: {call}"
        );
    }
}

/// `route_on: [timeout]` fires for a provider that answers **nothing**, inside
/// the node's own `timeout:` budget (PRD 5.9, grammar 9.2, 12.2).
///
/// The other three conditions are things a provider says, and the test above
/// stages each of them as a wire shape. This one is the condition nothing says:
/// a member that accepted the request and is still holding it. There is no
/// status to classify, so the only way the ladder can reach `model.fast` is for
/// the runtime to give the first member a bounded share of the node's budget and
/// give up on it when the share is spent.
///
/// The fixture's `flow.ask_promptly` declares `timeout: 4s` on its node and the
/// route has two members, so the first gets ~2s; the script holds the first
/// member's answer for far longer than the whole node budget, so a run that
/// produces the fallback's answer **at all** is a run in which the share fired,
/// and a run that produced it after the node's own deadline would not have
/// produced it. The node budget is the ceiling either way: nothing here can make
/// the ladder outlive grammar 9.2's bound.
#[test]
fn a_route_member_that_answers_nothing_fails_over_inside_the_nodes_budget() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // Accepted, never answered — the mock closes long after this node's
        // whole 4s budget, so nothing but the per-member share can end the wait.
        Script::new(SONNET, Outcome::timeout(Duration::from_secs(30))),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "in time" }))),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask_promptly",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "in time");

    let call = run
        .entries("ask")
        .into_iter()
        .find_map(|entry| {
            entry["models"]
                .as_array()
                .and_then(|calls| calls.first().cloned())
        })
        .expect("the model call is recorded");
    assert_eq!(call["servedBy"], "model.fast", "{call}");
    assert_eq!(call["fallback"], 1, "{call}");
    assert_eq!(
        call["failovers"][0]["condition"], "timeout",
        "a provider that answered nothing in time is `route_on:`'s `timeout`: {call}"
    );
    assert_eq!(call["failovers"][0]["model"], "model.smart", "{call}");
}

/// A node that **failed** still records every model call it made (PRD 5.9).
///
/// The trace is what a reader opens when a run did not work, and the failover
/// record is data rather than prose precisely so it can be read there. A route
/// that spent every member made two real calls, and a record that survived only
/// on the success path would leave the one run that needs it with nothing but
/// the sentence inside the error.
#[test]
fn an_exhausted_route_records_every_member_it_spent_in_the_failed_nodes_trace() {
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
    let failure = run.failed();
    assert!(
        failure.contains("model.smart") && failure.contains("model.fast"),
        "the error names the route it spent: {failure}"
    );

    let entries = run.entries("ask");
    let entry = entries.first().expect("the failed node has a trace entry");
    assert_eq!(entry["outcome"], "failed", "{entry}");
    let calls = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("a failed node still records its model calls: {entry}"));
    assert_eq!(
        calls.len(),
        1,
        "one call was made — one ladder, walked to its end: {entry}"
    );
    let refusals = calls[0]["failovers"]
        .as_array()
        .unwrap_or_else(|| panic!("the refusals on the way are recorded: {entry}"));
    assert_eq!(refusals.len(), 1, "{entry}");
    assert_eq!(refusals[0]["model"], "model.smart", "{entry}");
    assert_eq!(refusals[0]["condition"], "rate_limit", "{entry}");
    // Nothing served this call — `failovers` holds the members that moved the
    // ladder on, and the last member's refusal is what ended it, so that one is
    // recorded as the refusal rather than as a failover that never happened.
    assert!(
        calls[0]["servedBy"].is_null() && calls[0]["fallback"].is_null(),
        "a call nothing answered claims no member: {entry}"
    );
    assert_eq!(calls[0]["refused"]["model"], "model.fast", "{entry}");
    assert_eq!(calls[0]["refused"]["condition"], "rate_limit", "{entry}");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.model.as_str())
            .collect::<Vec<_>>(),
        [SONNET, HAIKU],
        "both members were really called"
    );
}

/// A node that **succeeded** on a retry still records what its earlier attempts
/// called (PRD 5.9).
///
/// The mirror of the test above, and the case that is easy to lose: a failed node
/// carries its model calls out on the failure, while a successful one is
/// described by the answer its winning attempt returned — and that answer holds
/// only the calls that attempt made. Everything the attempts before it did,
/// including a ladder spent to its last member, happened against a real provider
/// and has nowhere else to be reported.
///
/// So: two attempts over a two-member route, four scripted outcomes, one entry.
/// A trace that named one model call would describe a run in which two of the
/// four requests the provider recorded never happened.
#[test]
fn a_node_that_succeeded_on_a_retry_records_what_its_earlier_attempt_called() {
    let provider = MockProvider::start().expect("a loopback port");
    // Per-model FIFO: `model.smart` refuses twice, `model.fast` refuses once and
    // then answers — so attempt 1 spends the whole ladder and attempt 2 is
    // served by the fallback.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::structured(json!({ "answer": "42" }))),
    ]);

    let Some(run) = harness::run(
        "model-failover",
        "flow.ask_again",
        &[("question", "what is it?")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["answer"], "42");

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.model.as_str())
            .collect::<Vec<_>>(),
        [SONNET, HAIKU, SONNET, HAIKU],
        "two attempts, each walking the route in order"
    );

    let entries = run.entries("ask");
    let entry = entries.first().expect("the node has a trace entry");
    assert_eq!(entry["attempts"], 2, "{entry}");
    let calls = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the node records its model calls: {entry}"));
    assert_eq!(
        calls.len(),
        2,
        "one call per attempt, and the first attempt's is not dropped: {entry}"
    );

    // The lost attempt, first: nothing served it, and the ladder it spent is in
    // it — which is the record the provider's four requests are otherwise the
    // only evidence of.
    assert!(
        calls[0]["servedBy"].is_null(),
        "the first attempt's call was answered by nobody: {entry}"
    );
    assert_eq!(calls[0]["failovers"][0]["model"], "model.smart", "{entry}");
    assert_eq!(
        calls[0]["failovers"][0]["condition"], "rate_limit",
        "{entry}"
    );
    assert_eq!(calls[0]["refused"]["model"], "model.fast", "{entry}");

    // …then the attempt that answered, in the ordering the answer imposes.
    assert_eq!(calls[1]["servedBy"], "model.fast", "{entry}");
    assert_eq!(calls[1]["fallback"], 1, "{entry}");
    assert_eq!(calls[1]["failovers"][0]["model"], "model.smart", "{entry}");

    let rendered = run.stderr();
    assert!(
        rendered.contains("refused by model.fast")
            && rendered.contains("served by model.fast, fallback #1"),
        "the human report shows both attempts: {rendered}"
    );
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
fn run_executes_a_manual_trigger_and_prints_the_flow_outputs() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    // No `triggers:` section declares this flow: the implicit manual entry
    // exists for every flow (D64), which is what `run` invokes.
    let Some(run) = harness::run(
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        &provider,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["verdict"], "approve");

    // An input the schema refuses is refused at run start, naming the field
    // (grammar 13.2) — the same check a declared trigger's bindings get at
    // compile time.
    //
    // Grammar 13.2 puts three failures in one sentence — "an unknown argument
    // name, a missing REQUIRED field, or a value that does not fit the declared
    // type fails the run naming the field" — so all three are asserted, and each
    // is asserted on *what it says* rather than on the field name appearing
    // somewhere: a schema library's own multi-line dump contains the field name
    // too, and PRD G3 makes the difference between that and a sentence the
    // product feature.
    let refusals = [
        (
            vec![("goal", ""), ("draft", "a draft")],
            "the `inputs:` of `flow.review`: goal: expected at least 1 character (found \"\")",
            "a value the declared type refuses",
        ),
        (
            vec![("draft", "a draft")],
            "the `inputs:` of `flow.review`: goal: Invalid input: expected string, received undefined",
            "a REQUIRED field nobody passed",
        ),
        (
            vec![("goal", "ship it"), ("draft", "a draft"), ("nope", "x")],
            "`nope` is not an input of `flow.review`: it declares goal, draft",
            "an argument name the flow does not declare",
        ),
    ];
    for (inputs, expected, what) in refusals {
        let bad = harness::run("agent-anthropic", "flow.review", &inputs, &provider)
            .expect("the toolchain answered once already");
        let failure = bad.failed();
        assert_eq!(
            failure.trim_end(),
            format!("error: {expected}"),
            "{what} is refused by naming the field and what was wrong with it"
        );
        assert_eq!(
            bad.output.status.code(),
            Some(2),
            "…and all three are one kind of failure: a command that could not \
             run, rather than a run that produced no answer"
        );
    }
}

/// `--format json` folds the answer and the report into one document on stdout,
/// and leaves stderr empty.
///
/// The default format splits them — outputs on stdout, what ran on stderr — so
/// this is the shape a caller that parses one stream reads, and the two are
/// asserted together because "the report format changed the report" is only a
/// claim beside what the other format does.
#[test]
fn run_reports_its_whole_record_under_the_json_format() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let Some(project) = harness::scratch_project("run-json") else {
        return;
    };
    let run = harness::run_formatted(
        &project,
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        None,
        Some("json"),
        &harness::environment(&provider),
    );
    run.succeeded();
    assert_eq!(
        run.stderr(),
        "",
        "under `--format json` the whole answer is the document on stdout"
    );

    let answered = run.outputs();
    assert_eq!(answered["flow"], "flow.review");
    assert_eq!(answered["status"], "completed");
    assert_eq!(answered["outputs"]["verdict"], "approve");
    // The routing record of PRD 5.3, in the same document rather than in a file
    // a second reader would have to find.
    let trace = answered["trace"]
        .as_array()
        .expect("the record carries the run's trace");
    assert_eq!(
        trace
            .iter()
            .map(|entry| entry["node"].clone())
            .collect::<Vec<_>>(),
        [json!("review")]
    );
    assert!(
        answered["trace_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(".json")),
        "…and still says where the file went: {answered}"
    );
    let execution = answered["execution_id"]
        .as_str()
        .unwrap_or_else(|| panic!("the record names the execution it reports: {answered}"));
    assert!(
        answered["trace_path"]
            .as_str()
            .is_some_and(|path| path.ends_with(&format!("-{execution}.json"))),
        "…which is what the trace file is named after, so one record ties the \
         two together: {answered}"
    );

    // A run that produced **no** answer answers with the same record, and this
    // is the one a reader most needs the file for: the whole trace of a failure
    // is longer than the summary, and under this format there is no stderr line
    // naming it. A route that spent every member is a failure with a trace to
    // have.
    let spent = MockProvider::start().expect("a loopback port");
    spent.enqueue_all([
        Script::new(SONNET, Outcome::rate_limit()),
        Script::new(HAIKU, Outcome::rate_limit()),
    ]);
    let Some(project) = harness::scratch_project("run-json-failed") else {
        return;
    };
    let failed = harness::run_formatted(
        &project,
        "model-failover",
        "flow.ask",
        &[("question", "what is it?")],
        None,
        Some("json"),
        &harness::environment(&spent),
    );
    failed.failed();
    let record = failed.outputs();
    assert_eq!(record["flow"], "flow.ask");
    assert_eq!(record["status"], "failed");
    assert!(
        record["error"]
            .as_str()
            .is_some_and(|text| text.contains("model.smart")),
        "the record carries what went wrong: {record}"
    );
    assert!(
        !record["trace"].as_array().is_none_or(Vec::is_empty),
        "…and the trace the run did make: {record}"
    );
    let written = record["trace_path"]
        .as_str()
        .unwrap_or_else(|| panic!("a failed run names the file its trace went to: {record}"));
    let held: Value = serde_json::from_str(
        &std::fs::read_to_string(written).expect("the path names a file that exists"),
    )
    .expect("the trace file holds the trace");
    // The file is the envelope of `docs/trace.md` — a document that says which
    // format it is in — and its `entries` are the record's `trace`, so the two
    // surfaces carry one trace rather than two readings of it.
    assert_eq!(
        held["entries"], record["trace"],
        "and the file holds what the record does"
    );
    assert_eq!(
        held["trace_version"], record["trace_version"],
        "…under the same declared format version: {record}"
    );
    assert_eq!(held["flow"], record["flow"], "{held}");
    assert_eq!(held["execution_id"], record["execution_id"], "{held}");
    assert_eq!(held["status"], json!("failed"), "{held}");
}

/// `run` refuses **before** it launches when a variable the composition
/// references is unset, naming the variable and where it is written
/// (PRD §9.15).
///
/// The emitted project checks its own environment at process start, which is
/// what covers a project run by hand and what
/// `a_missing_env_ref_fails_at_process_start_naming_the_variable` decides. This
/// is the other half of the same sentence — "`run`/`serve` fail fast before
/// invoking the graph" — and it is a different check with a different message:
/// the compiler knows every reference statically, so it names all of them and
/// the site each is written at, without a runtime having been started at all.
#[test]
fn run_refuses_before_it_launches_when_a_variable_is_missing() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(SONNET, Outcome::text("never reached")));

    let environment: Vec<(String, String)> = harness::environment(&provider)
        .into_iter()
        .filter(|(name, _)| name != harness::API_KEY)
        .collect();
    let Some(project) = harness::scratch_project("run-unset") else {
        return;
    };
    let run = harness::run_into(
        &project,
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        None,
        &environment,
    );
    let failure = run.failed();
    assert!(
        failure.contains(harness::API_KEY) && failure.contains("provider.mock.api_key"),
        "the refusal names the variable and the site that references it: {failure}"
    );
    assert!(
        provider.requests().is_empty(),
        "nothing was launched, so nothing was called"
    );
    // …and the project was still **built**: a build reads no environment
    // (PRD 5.9), so the refusal is about starting rather than about compiling.
    assert!(
        project.join("src/graph.ts").is_file(),
        "the build happened and the launch did not"
    );
}

/// `run` says what is missing when the pinned dependency set is not installed.
///
/// An emitted project is source rather than a bundle: it imports LangGraph, Zod
/// and the rest of the pinned set by name. A launch into a directory with no
/// `node_modules` above it would otherwise fail inside the runtime with a
/// resolution error naming a package, which tells a reader nothing about the
/// step they skipped.
#[test]
fn run_says_what_is_missing_when_the_dependency_set_is_not_installed() {
    // The runtime and not the install: this run is meant to stop *before* it
    // resolves a package, and the message it stops with is only this one when
    // there was a runtime to launch. Same skip-locally, fail-in-CI rule as
    // everything else here.
    if harness::bun_command().is_none() {
        return;
    }
    let provider = MockProvider::start().expect("a loopback port");
    let elsewhere = harness::Scratch::new("uninstalled");

    let run = harness::run_into(
        &elsewhere.path().join("project"),
        "agent-anthropic",
        "flow.review",
        &[("goal", "ship it"), ("draft", "a draft")],
        None,
        &harness::environment(&provider),
    );
    let failure = run.failed();
    assert!(
        failure.contains("bun install") && failure.contains("not installed"),
        "the refusal names the step rather than a package: {failure}"
    );
    assert!(provider.requests().is_empty());
}

/// `agent-compose serve` exposes start and status for an `http` trigger.
///
/// Two of the criterion's three verbs, over the fixture's interrupt-free flow
/// (`flow.direct`): a start that answers with an execution id and a status route
/// that reports the run's terminal state and outputs. Split from the resume half
/// below because this half is decidable without anything ever pausing, and a
/// test that mixed the two would be asserting about the `human` runtime while
/// claiming to be about the route.
#[test]
fn serve_exposes_start_and_status_for_an_http_trigger() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
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
    assert_eq!(
        refused.json()["error"].as_str(),
        Some(
            "the request does not fit the `inputs:` of `flow.direct`: question: expected at least 1 character (found \"\")"
        ),
        "…and the body says which field and what was wrong with it, rather than \
         handing back a schema library's own dump (PRD G3): {:?}",
        refused.body
    );

    assert!(provider.snapshot().is_drained(), "the run used its script");
}

/// `respond: sync` blocks for the flow's outputs, and on timeout expiry the
/// response **upgrades to async** while the execution continues (grammar 13.3,
/// PRD §9.8).
///
/// Both halves, over one trigger, because the upgrade only means anything beside
/// the answer it replaces: a run that finishes inside the budget is a `200`
/// carrying the outputs, and one that does not is a `202` carrying the execution
/// id and a status URL — with nothing cancelled, which the status read after it
/// is what proves. The scripted delay is what spends the budget: the trigger
/// declares `timeout: 2s`, the second call's model answer is held longer than
/// that, and the run completes on its own afterwards.
#[test]
fn serve_answers_a_sync_trigger_and_upgrades_when_its_timeout_expires() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "answer": "at once" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "eventually" })).after(Duration::from_secs(4)),
        ),
    ]);

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let answered = app
        .post_json("/sync-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(answered.status, 200, "{:?}", answered.body);
    let body = answered.json();
    assert_eq!(body["status"], "completed");
    assert_eq!(body["outputs"]["answer"], "at once");

    let upgraded = app
        .post_json("/sync-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(
        upgraded.status, 202,
        "the budget is the *response's*, so its expiry changes the answer rather \
         than the run: {:?}",
        upgraded.body
    );
    let body = upgraded.json();
    let execution = body["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    assert_eq!(body["status_url"], format!("/executions/{execution}"));

    // Nothing was cancelled: the execution the upgrade handed back finishes.
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed");
    assert_eq!(finished["outputs"]["answer"], "eventually");
}

/// `resume` tells "no such execution" from "nothing is waiting", and disturbs
/// neither (PRD 5.11).
///
/// Every refusal on this route is about *which* pause the request means, and a
/// route that answered them all the same way would hide the one a caller can
/// act on. An id this process never started is a `404` — the caller is polling
/// the wrong app, or the process restarted. An id it did start, on a flow with
/// no `human` node anywhere in it, is a `409`: there is a run, and it was never
/// going to ask anyone anything.
#[test]
fn resume_tells_an_unknown_execution_from_one_that_is_not_waiting() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // An id this process never started is a resume of nothing, and is told apart
    // from a resume that arrived at the wrong moment.
    let unknown = app
        .post_json("/executions/exec_nothing/resume", &json!({}))
        .expect("the resume route answers");
    assert_eq!(unknown.status, 404, "{:?}", unknown.body);
    assert!(
        unknown.json()["error"]
            .as_str()
            .unwrap_or_default()
            .contains("no execution `exec_nothing` was started by this process"),
        "{:?}",
        unknown.body
    );

    let started = app
        .post_json("/direct-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert!(
        finished["interrupts"].is_null(),
        "`flow.direct` has no `human` node, so nothing was ever published: {finished}"
    );

    let refused = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(refused.status, 409, "{:?}", refused.body);
    assert_eq!(
        refused.json()["error"].as_str(),
        Some("this execution has already completed, so nothing is waiting for an answer"),
        "{:?}",
        refused.body
    );
}

/// An app that cannot start is a command that could not run: exit `2` with a
/// sentence, not a framework stack trace.
///
/// `main.rs`'s exit-code table gives `1` one meaning — "a run produced no
/// answer" — and an address already taken is not that: nothing ran. The port is
/// held by this test, which is the one way to make the failure deterministic.
#[test]
fn serve_answers_two_with_a_sentence_when_it_cannot_take_the_port() {
    let provider = MockProvider::start().expect("a loopback port");
    // Held for the whole test: the app is refused the port because this socket
    // still owns it.
    let held = std::net::TcpListener::bind("127.0.0.1:0").expect("a loopback port");
    let port = held.local_addr().expect("a bound address").port();

    let Some(output) = harness::serve_refused("http-trigger", &provider, port) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(
        output.status.code(),
        Some(2),
        "the command could not run at all: {stderr}"
    );
    assert!(
        stderr.contains(&format!("the app could not listen on 127.0.0.1:{port}")),
        "…and says which address it could not have: {stderr}"
    );
}

/// What the generated app does with a request **body** is grammar 13.3's rule,
/// not the framework's default (Decision D117).
///
/// One rule, both directions. A body-bearing method sent with an empty body
/// presents `payload.body = {}` and starts an execution, which is the case a
/// stock JSON parser refuses before a handler runs; the identical request sent
/// with no `content-type` at all must do the same thing, because whether an
/// execution starts cannot turn on a header the grammar gives no meaning to; and
/// a body that is **present** and is not a decodable JSON object is refused at
/// request time, starting nothing.
///
/// The refusals are staged as a family rather than as one case, because "not a
/// decodable JSON object" has two halves and only one of them is loud. A body
/// that does not parse cannot become an object by accident. A body that parses
/// to a JSON `null`, `false`, `0`, a string or an array **can**: every one of
/// them is a value a handler might quietly treat as "nothing was sent", and
/// `null` in particular is what a `?? {}` turns into the empty payload of a
/// request that carried none — which starts an execution D117 says starts none.
///
/// `/query-answers` is the route it is decided on because its bindings read the
/// query string: a trigger that read the body would fail the read instead, which
/// is D110's rule rather than this one, and would make the whole family answer
/// correctly for the wrong reason.
#[test]
fn a_request_with_an_empty_body_starts_an_execution_and_a_non_object_one_does_not() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "answer": "declared" }))),
        Script::new(SONNET, Outcome::structured(json!({ "answer": "bare" }))),
    ]);

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let declared = app
        .send(
            Request::post("/query-answers?q=what-is-it")
                .header("content-type", "application/json")
                .bytes(Vec::new()),
        )
        .expect("the trigger's route answers");
    assert_eq!(
        declared.status, 202,
        "an empty body presents `{{}}` and starts an execution: {:?}",
        declared.body
    );
    let execution = declared.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["outputs"]["answer"], "declared");

    let bare = app
        .send(Request::post("/query-answers?q=what-is-it"))
        .expect("the trigger's route answers");
    assert_eq!(
        bare.status, 202,
        "…and so does the same request with no content type: {:?}",
        bare.body
    );
    let second = bare.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    assert_eq!(
        harness::settled(&app, &second)["outputs"]["answer"],
        "bare",
        "both requests really ran the flow"
    );

    for staged in [
        // Undecodable.
        "{not json",
        // …and decodable, but not an object. `null` is the one a `?? {}` would
        // silently promote to the empty payload above.
        "null",
        "false",
        "0",
        "\"text\"",
        "[1]",
    ] {
        let refused = app
            .send(
                Request::post("/query-answers?q=what-is-it")
                    .header("content-type", "application/json")
                    .bytes(staged.as_bytes().to_vec()),
            )
            .expect("the trigger's route answers");
        assert_eq!(
            refused.status, 400,
            "a body of `{staged}` is not a JSON object, so it starts no execution: {:?}",
            refused.body
        );
        assert!(
            String::from_utf8_lossy(&refused.body).contains("not a JSON object"),
            "…and says so in the app's own words: {:?}",
            refused.body
        );
    }

    assert!(
        provider.snapshot().is_drained(),
        "two requests started two runs, and none of the refused bodies started a third"
    );
}

/// A trigger that cannot read a request says **which** key it looked for.
///
/// The generated app is the first surface where a CEL diagnostic is read by
/// somebody outside the composition: a caller who omitted a query parameter or a
/// header gets the evaluator's own sentence back as a `400` body, and an index
/// spelled `payload.query[…]` tells them everything about their mistake except
/// the part they can act on. PRD G3 makes error UX a product feature, and the
/// validator's column already spells a literal key — so the two interpreters
/// naming one sub-path differently would be exactly the drift the shared corpus
/// exists to prevent, in the half a corpus of values does not reach.
///
/// `/query-answers` is the route it is decided on because its binding is an
/// index over a payload member the request controls: `payload.query['q']`,
/// grammar 13.3's own spelling, sent with no `q` at all.
#[test]
fn a_trigger_that_cannot_read_a_request_names_the_key_it_looked_for() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let refused = app
        .send(Request::post("/query-answers"))
        .expect("the trigger's route answers");
    assert_eq!(
        refused.status, 400,
        "a binding that cannot be read is a bad request, not a failed run: {:?}",
        refused.body
    );
    assert_eq!(
        refused.json()["error"].as_str(),
        Some(
            "the trigger `on_query_request` could not read this request: `payload.query[\"q\"]` is not present: nothing has supplied it"
        ),
        "the refusal names the trigger and the key that was missing: {:?}",
        refused.body
    );

    assert!(
        provider.requests().is_empty(),
        "…and no execution started, so nothing reached the provider"
    );
}

/// The `callback:` completion webhook fires with the run's report, and only for
/// a request that asked for one (grammar 13.3, PRD 5.11).
///
/// Both halves, because the second is what the first's implementation costs: the
/// URL is read **after** the run rather than at the start, precisely so that
/// `callback: "payload.body.callback_url"` — the grammar's own spelling — does
/// not refuse every caller who did not want a webhook. A change that read it
/// earlier would turn those callers into failed requests, and a test that only
/// watched the delivery arrive would stay green through it.
#[test]
fn a_completion_webhook_fires_with_the_runs_report_and_only_when_a_url_was_given() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "answer": "notified" }))),
        Script::new(SONNET, Outcome::structured(json!({ "answer": "quiet" }))),
    ]);

    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json(
            "/callback-answers",
            &json!({
                "question": "what is it?",
                "callback_url": format!("{}/done", receiver.base_url),
            }),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    assert_eq!(harness::settled(&app, &execution)["status"], "completed");

    let delivered = receiver.wait_for(1, Duration::from_secs(30));
    let report = &delivered[0].body;
    assert_eq!(delivered[0].event(), "settled", "{report}");
    assert_eq!(report["execution_id"], execution, "{report}");
    assert_eq!(report["status"], "completed", "{report}");
    assert_eq!(report["outputs"]["answer"], "notified", "{report}");
    assert!(
        report["trace"]
            .as_array()
            .is_some_and(|trace| !trace.is_empty()),
        "the delivery carries the whole report the status route holds: {report}"
    );

    // The same trigger, from a caller who named no URL: a request with no
    // webhook rather than a request that could not be read.
    let quiet = app
        .post_json("/callback-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(
        quiet.status, 202,
        "an absent `callback_url` is not a bad request: {:?}",
        quiet.body
    );
    let second = quiet.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let finished = harness::settled(&app, &second);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["outputs"]["answer"], "quiet", "{finished}");
    // The run has already settled, and a webhook fires immediately after: this
    // is the window in which a delivery that should not happen would.
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(
        receiver.delivered().len(),
        1,
        "two executions, one webhook: {:?}",
        receiver.delivered()
    );
}

/// The third verb: `resume` against the interrupting `human` node's schema.
///
/// Kept apart from start/status because it is the one verb that needs the
/// `human` node runtime: the execution has to really stop at the pause, the
/// status route has to say so, and the payload has to be held to the node's own
/// `output:` before anything is delivered (grammar 8.7, PRD 5.11).
#[test]
fn serve_resumes_an_interrupted_execution_against_the_human_nodes_schema() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
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

    // …and the report is enough to *ask* the question rather than only to
    // notice there is one: what the human is shown, the schema their answer has
    // to fit, and where to send it (PRD 5.11).
    let interrupts = status["interrupts"].as_array().unwrap_or_else(|| {
        panic!("an interrupted report names the pauses it is holding: {status}")
    });
    assert_eq!(interrupts.len(), 1, "{status}");
    let waiting = &interrupts[0];
    assert_eq!(waiting["wait_id"], "approve/0", "{waiting}");
    assert_eq!(waiting["flow"], "flow.assisted", "{waiting}");
    assert_eq!(waiting["node"], "approve", "{waiting}");
    assert_eq!(
        waiting["input"],
        json!({ "answer": "an answer" }),
        "the node's own `input:`, evaluated — what the human is shown: {waiting}"
    );
    assert_eq!(
        waiting["output_schema"]["properties"]["decision"]["enum"],
        json!(["approve", "reject"]),
        "…and the published schema an answer is held to: {waiting}"
    );
    assert_eq!(
        waiting["resume_url"],
        "/executions/{execution}/resume?wait=approve%2F0".replace("{execution}", &execution),
        "{waiting}"
    );
    assert!(
        waiting["expires_at"].is_string(),
        "this node declares `timeout: 24h`, so the wait has a deadline to publish: {waiting}"
    );
    assert!(
        status["trace"].is_null() && status["trace_version"].is_null(),
        "a run that has not stopped carries no trace (`docs/trace.md` §1.3): {status}"
    );

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

    // …and it did not consume the wait: an answer that does not fit is a
    // request to send a better one, not a turn spent (PRD 5.11).
    let still = harness::settled(&app, &execution);
    assert_eq!(
        still["status"], "interrupted",
        "the refused payload left the execution waiting: {still}"
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
    assert!(
        finished["interrupts"].is_null(),
        "a finished execution is holding nothing: {finished}"
    );

    // The pause is trace data (PRD 5.3, `docs/trace.md` §3): when it began, that
    // it resumed, and when the answer arrived. What the human *said* is not
    // there and is not meant to be — the same posture the format takes to a
    // model's completion (§11) — so the entry names no `decision` anywhere.
    let entry = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"))
        .iter()
        .find(|entry| entry["node"] == "approve")
        .unwrap_or_else(|| panic!("the `human` node has an entry: {finished}"))
        .clone();
    assert_eq!(entry["outcome"], "completed", "{entry}");
    assert_eq!(entry["human"]["settled"], "resumed", "{entry}");
    assert!(entry["human"]["pausedAt"].is_string(), "{entry}");
    assert!(entry["human"]["settledAt"].is_string(), "{entry}");
    assert!(entry["human"]["expiresAt"].is_string(), "{entry}");
    assert_eq!(
        entry["writes"],
        json!(["decision"]),
        "the answer reaches the run through the node's writes, by channel name: {entry}"
    );
    assert!(
        !entry.to_string().contains("looks right"),
        "no field of the format carries what the human answered: {entry}"
    );

    // A resume for a run that has stopped is refused rather than delivered.
    let late = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "reject", "note": "too late" }),
        )
        .expect("the resume route answers");
    assert_eq!(late.status, 409, "{:?}", late.body);
    assert_eq!(
        late.json()["error"].as_str(),
        Some("this execution has already completed, so nothing is waiting for an answer"),
        "{:?}",
        late.body
    );

    // …and an id this process never started is the other mistake, told apart.
    let unknown = app
        .post_json(
            "/executions/exec_nobody/resume",
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(unknown.status, 404, "{:?}", unknown.body);
}

/// A wait that runs out its budget takes its `on_timeout:` route **instead of**
/// the node's own edges, and the answer that arrives after it is refused as the
/// expiry it is (grammar 8.7, 9.2).
///
/// Three claims in one run, because the second and third are only decidable
/// against the first: the route was taken (a channel only that route writes has
/// a value, and the node's own edges did not fire), the trace says which of the
/// two ways the wait ended, and a resume sent into the window *after* the expiry
/// and *before* the run finished is answered by the wait's own state rather than
/// by "this execution is over" — which is why the fixture's route sleeps.
#[test]
fn an_expired_wait_takes_its_route_and_refuses_the_answer_that_arrives_after_it() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/expiring-answers", &json!({ "question": "what is it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let waiting = harness::settled(&app, &execution);
    assert_eq!(waiting["status"], "interrupted", "{waiting}");
    assert_eq!(
        waiting["interrupts"][0]["wait_id"], "sign_off/0",
        "{waiting}"
    );

    // The budget runs out and the `on_timeout:` route starts; the execution is
    // still going, which is the window the late answer lands in.
    let running = poll_until(&app, &execution, "running");
    assert!(
        running["interrupts"].is_null(),
        "the wait is over, so there is nothing to publish: {running}"
    );
    let late = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=sign_off/0"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(late.status, 409, "{:?}", late.body);
    let said = late.json()["error"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        said.contains("expired before this answer arrived")
            && said.contains("on_timeout")
            && said.contains("sign_off/0"),
        "the refusal names what happened to the wait rather than only refusing: {said}"
    );

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"]["escalated"], "the wait ran out",
        "only the `on_timeout:` route writes this channel: {finished}"
    );

    let entry = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"))
        .iter()
        .find(|entry| entry["node"] == "sign_off")
        .unwrap_or_else(|| panic!("the `human` node has an entry: {finished}"))
        .clone();
    assert_eq!(
        entry["outcome"], "failed",
        "the node left no result for the run to carry on from: {entry}"
    );
    assert_eq!(
        entry["fallback"], "give_up",
        "…and control transferred to the declared route instead of its edges: {entry}"
    );
    assert!(
        entry["routing"].is_null(),
        "a node whose edges were not evaluated records no routing decision: {entry}"
    );
    assert_eq!(entry["human"]["settled"], "expired", "{entry}");
    assert!(entry["human"]["settledAt"].is_string(), "{entry}");
}

/// Several pauses in one execution are told apart by the instance path that
/// addresses them (grammar 9.4, 8.6).
///
/// A `map` dispatching a flow with a `human` node in it is the reachable way one
/// execution holds more than one wait at a time, and it is the case a resume
/// route with only an execution id could not serve. `?wait=` is what names one,
/// the ids are derived rather than handed out — `fan/0/<index>/sign/0` — and a
/// resume that names none while two are pending is refused rather than guessed.
/// So is one that names *two*: a repeated `?wait=` arrives as an array, and the
/// refusal says that rather than reporting a comma-joined id no client sent.
#[test]
fn two_pauses_in_one_execution_are_addressed_by_their_instance_paths() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json(
            "/batch-answers",
            &json!({ "questions": ["ship it?", "revert it?"] }),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let waiting = wait_for_pauses(&app, &execution, 2);
    let ids: Vec<String> = waiting["interrupts"]
        .as_array()
        .expect("the pauses")
        .iter()
        .map(|one| one["wait_id"].as_str().unwrap_or_default().to_string())
        .collect();
    // Ordered by id rather than by which instance parked first: two instances
    // doing identical work under `max_concurrency: 4` are interleaved by the
    // scheduler, and a report ordered on that would differ between runs of one
    // composition. `humanWaits` sorts, which is what makes this an assertion
    // about addressing rather than about scheduling.
    assert_eq!(
        ids,
        vec!["fan/0/0/sign/0".to_string(), "fan/0/1/sign/0".to_string()],
        "each pause is addressed by its own instance path: {waiting}"
    );
    // Each carries the question *its* instance was dispatched with, which is
    // what makes two pauses two questions rather than one asked twice.
    assert_eq!(waiting["interrupts"][0]["input"]["question"], "ship it?");
    assert_eq!(waiting["interrupts"][1]["input"]["question"], "revert it?");

    // An unaddressed resume is refused rather than applied to whichever pause
    // happened to be first, and it lists what it could have meant.
    let ambiguous = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(ambiguous.status, 409, "{:?}", ambiguous.body);
    assert_eq!(
        ambiguous.json()["pending"],
        json!(["fan/0/0/sign/0", "fan/0/1/sign/0"]),
        "{:?}",
        ambiguous.body
    );

    // An id that is not one of them is the other refusal, told apart.
    let nowhere = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=sign/0"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(nowhere.status, 409, "{:?}", nowhere.body);
    assert!(
        nowhere.json()["error"]
            .as_str()
            .unwrap_or_default()
            .contains("holding no pause `sign/0`"),
        "{:?}",
        nowhere.body
    );

    // And a request carrying `wait` **twice** names two pauses, which is not
    // what one resume answers. Refused as that — rather than joined into a
    // comma-spliced id no client ever sent, or applied to whichever of the two
    // the parser happened to keep — and it consumes neither, which the resumes
    // below prove by still being taken.
    let doubled = app
        .post_json(
            &format!(
                "/executions/{execution}/resume?wait=fan%2F0%2F0%2Fsign%2F0&wait=fan%2F0%2F1%2Fsign%2F0"
            ),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(doubled.status, 400, "{:?}", doubled.body);
    assert_eq!(
        doubled.json()["wait"],
        json!(["fan/0/0/sign/0", "fan/0/1/sign/0"]),
        "the refusal echoes what the request actually named: {:?}",
        doubled.body
    );
    assert_eq!(
        doubled.json()["pending"],
        json!(["fan/0/0/sign/0", "fan/0/1/sign/0"]),
        "{:?}",
        doubled.body
    );
    assert!(
        doubled.json()["error"]
            .as_str()
            .unwrap_or_default()
            .contains("carried `wait` 2 times"),
        "{:?}",
        doubled.body
    );

    // Answered in the reverse of source-item order, because what the fan-out
    // writes is ordered by **index** rather than by who answered first
    // (grammar 8.6 rule 5).
    for (id, decision) in [("fan/0/1/sign/0", "reject"), ("fan/0/0/sign/0", "approve")] {
        let resumed = app
            .post_json(
                &format!(
                    "/executions/{execution}/resume?wait={}",
                    id.replace('/', "%2F")
                ),
                &json!({ "decision": decision }),
            )
            .expect("the resume route answers");
        assert_eq!(resumed.status, 202, "{:?}", resumed.body);
        assert_eq!(resumed.json()["wait"], id, "{:?}", resumed.body);
    }

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"]["decisions"],
        json!(["approve", "reject"]),
        "the join is ordered by source-item index, not by who answered first: {finished}"
    );
}

/// Two interrupted executions hold their pauses — and their answers — apart
/// (PRD 5.11).
///
/// The wait id is an instance path *inside one run*, so two executions of one
/// composition pause at the same `approve/0`. What tells them apart is the
/// execution the wait belongs to and nothing else, which makes this the case a
/// board flattened into one id-keyed table would pass every other test and still
/// break: the answer meant for one execution would settle the other's pause.
/// Each report must carry only its own pause, answering one must leave the other
/// waiting, and each run must end holding the decision that was sent to *it*.
#[test]
fn two_interrupted_executions_hold_their_pauses_and_answers_apart() {
    let provider = MockProvider::start().expect("a loopback port");
    // One per execution, and deliberately identical: what distinguishes the two
    // runs here is the answer a human gives, not the one the model gave.
    for _ in 0..2 {
        provider.enqueue(Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "an answer" })),
        ));
    }

    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let start = |question: &str| -> String {
        let started = app
            .post_json("/answers", &json!({ "question": question }))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{:?}", started.body);
        started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string()
    };
    let first = start("what is it?");
    let second = start("and this one?");
    assert_ne!(first, second, "two starts are two executions");

    for execution in [&first, &second] {
        let waiting = wait_for_pauses(&app, execution, 1);
        assert_eq!(waiting["status"], "interrupted", "{waiting}");
        assert_eq!(
            waiting["execution_id"].as_str(),
            Some(execution.as_str()),
            "a report is about the execution it was asked for: {waiting}"
        );
        assert_eq!(
            waiting["interrupts"][0]["wait_id"], "approve/0",
            "both executions pause at the same instance path: {waiting}"
        );
    }

    // Answering one settles that one's pause…
    let resumed = app
        .post_json(
            &format!("/executions/{first}/resume"),
            &json!({ "decision": "approve", "note": "the first" }),
        )
        .expect("the resume route answers");
    assert_eq!(resumed.status, 202, "{:?}", resumed.body);
    let finished = harness::settled(&app, &first);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["outputs"]["decision"], "approve", "{finished}");

    // …and leaves the other holding the pause it started with, un-addressed
    // resume and all: "this execution is holding exactly one" is a question
    // asked per execution.
    let still = app
        .get(&format!("/executions/{second}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        still["status"], "interrupted",
        "the first execution's answer is not this one's: {still}"
    );
    assert_eq!(still["interrupts"][0]["wait_id"], "approve/0", "{still}");

    let resumed = app
        .post_json(
            &format!("/executions/{second}/resume"),
            &json!({ "decision": "reject", "note": "the second" }),
        )
        .expect("the resume route answers");
    assert_eq!(resumed.status, 202, "{:?}", resumed.body);
    let finished = harness::settled(&app, &second);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"]["decision"], "reject",
        "each run ends holding the decision that was sent to it: {finished}"
    );
}

/// A `timeout:` on the node **above** a pause does not cut the wait short
/// (Decision D102, grammar 9.2).
///
/// D102 withholds `timeout` and `retry` from a `human` node at every level of
/// grammar 9.3's chain so that a composition-wide budget can never end a wait —
/// and the node that *dispatches* a pause is where that promise is easiest to
/// break, because it is not a `human` node and resolves `defaults:` like any
/// other. `flow.patient`'s `wrap` node declares `timeout: 2s` over a subflow
/// whose only node is a `human` one; this waits four seconds — twice the budget —
/// and then answers. A budget that ran while the human was thinking would have
/// failed the node before the resume was sent, and the execution would be
/// `failed` rather than holding the same pause it started with.
#[test]
fn an_enclosing_nodes_budget_does_not_run_while_a_pause_below_it_is_open() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/patient-answers", &json!({ "question": "ship it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let waiting = wait_for_pauses(&app, &execution, 1);
    assert_eq!(
        waiting["interrupts"][0]["wait_id"], "wrap/0/sign/0",
        "the pause is addressed through the node that dispatched it: {waiting}"
    );

    // Twice the enclosing node's budget, spent doing nothing — which is exactly
    // what a human takes.
    std::thread::sleep(Duration::from_secs(4));

    let still = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        still["status"], "interrupted",
        "`wrap`'s 2s budget is not the wait's: {still}"
    );
    assert_eq!(
        still["interrupts"][0]["wait_id"], "wrap/0/sign/0",
        "…and it is the same pause, not a new one: {still}"
    );
    assert_eq!(
        still["interrupts"][0]["expires_at"],
        Value::Null,
        "the wait declares no `timeout:`, so it has no expiry to report: {still}"
    );

    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=wrap%2F0%2Fsign%2F0"),
            &json!({ "decision": "reject" }),
        )
        .expect("the resume route answers");
    assert_eq!(resumed.status, 202, "{:?}", resumed.body);

    let finished = harness::settled(&app, &execution);
    assert_eq!(
        finished["status"], "completed",
        "the budget was held still, not cancelled — the instance finished inside \
         what was left of it: {finished}"
    );
    assert_eq!(finished["outputs"]["decision"], "reject", "{finished}");
}

/// A wait whose `on_timeout:` names **`end`** retires the branch that was
/// holding it, and the execution completes (grammar 8.7, 7.6.3).
///
/// `on_timeout:` takes §9.2's two target shapes and
/// [`an_expired_wait_takes_its_route_and_refuses_the_answer_that_arrives_after_it`]
/// reaches only the first: a flow-local node id, which schedules that node. This
/// is the other, and it is the one whose runtime spelling is a pseudo-node —
/// `goto: ["__end__"]` — so the two readings a compiled graph could produce are
/// both a plausible bug. A branch that retired but was still counted as live
/// leaves the run waiting on nothing until the superstep ceiling ends it, and a
/// `failed` node outcome carried up as the *run's* outcome reports a composition
/// that did exactly what it declared as a failure.
///
/// Three claims, in the one run that can hold them: nothing on the far side of
/// the pause ran (`signed` has only one writer and it is the node the wait's own
/// edge goes to), the execution ended `completed`, and the trace says which of
/// the two ways the wait ended and where control went — `"__end__"`, which
/// `docs/trace.md` §3 names as `fallback`'s spelling for the terminal
/// pseudo-node.
#[test]
fn a_wait_that_expires_into_end_retires_its_branch_and_completes_the_execution() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/lapsing-answers", &json!({ "question": "ship it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let waiting = wait_for_pauses(&app, &execution, 1);
    assert_eq!(
        waiting["interrupts"][0]["wait_id"], "sign_off/0",
        "{waiting}"
    );
    assert!(
        waiting["interrupts"][0]["expires_at"].is_string(),
        "this node declares `timeout: 3s`, so the wait has a deadline to publish: {waiting}"
    );

    // Nobody answers, so the only thing that ends this run is the budget.
    // `poll_until` rather than `harness::settled`, which counts `interrupted` as
    // a state a run has stopped in: the pause this test is about is exactly that
    // state, so waiting for "settled" would answer with the report above.
    let finished = poll_until(&app, &execution, "completed");
    assert_eq!(
        finished["status"], "completed",
        "a branch that retired at `end` is a run that finished, not one that \
         failed: {finished}"
    );
    assert!(
        finished["interrupts"].is_null(),
        "the wait is over, so there is nothing left to publish: {finished}"
    );
    assert_eq!(
        finished["outputs"]["signed"], "",
        "`record` is the only writer of this channel and it is what the wait's \
         own edge goes to, so a value here would mean the branch carried on: \
         {finished}"
    );

    let trace = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"))
        .clone();
    assert!(
        !trace.iter().any(|entry| entry["node"] == "record"),
        "nothing on the far side of the pause ran: {finished}"
    );
    let entry = trace
        .iter()
        .find(|entry| entry["node"] == "sign_off")
        .unwrap_or_else(|| panic!("the `human` node has an entry: {finished}"))
        .clone();
    assert_eq!(
        entry["outcome"], "failed",
        "the node left no result for the run to carry on from: {entry}"
    );
    assert_eq!(
        entry["fallback"], "__end__",
        "…and control transferred to the terminal pseudo-node rather than to \
         the node its own edge names: {entry}"
    );
    assert!(
        entry["routing"].is_null(),
        "a node whose edges were not evaluated records no routing decision: {entry}"
    );
    assert_eq!(entry["human"]["settled"], "expired", "{entry}");
    assert!(entry["human"]["settledAt"].is_string(), "{entry}");
    assert!(entry["human"]["expiresAt"].is_string(), "{entry}");
}

/// Two pauses at **one node**, told apart by the traversal ordinal in their ids
/// (grammar 9.4, 7.4).
///
/// The other axis a wait id is built on, and the one a fan-out cannot reach:
/// [`two_pauses_in_one_execution_are_addressed_by_their_instance_paths`] holds
/// several pauses at once, at *different* sites, while this holds them one after
/// the other at the same site. `flow.revisiting` is a bounded cycle whose
/// back-edge is guarded on the `human` node's own answer, so a `reject` sends
/// the flow round to that node and a second pause opens there — and what tells
/// the second question from the first is the ordinal alone: `sign/0`, then
/// `sign/1`.
///
/// A wait id built from the node id would make the two indistinguishable, which
/// is decidable here and nowhere else: the second resume would be refused as an
/// answer already given, and the run would sit on a pause nothing could address.
/// So the first id is sent again *after* the second pause opened — it must be
/// refused as the settled wait it is, rather than answer the live one — and the
/// `append` channel the node writes carries both answers in pass order, which is
/// what says both traversals really ran.
#[test]
fn a_second_traversals_pause_is_addressed_apart_from_the_first() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/revisited-answers", &json!({ "question": "ship it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let first = wait_for_pauses(&app, &execution, 1);
    assert_eq!(
        first["interrupts"][0]["wait_id"], "sign/0",
        "the first traversal's pause is the node at ordinal `0`: {first}"
    );
    assert_eq!(
        first["interrupts"][0]["resume_url"],
        "/executions/{execution}/resume?wait=sign%2F0".replace("{execution}", &execution),
        "{first}"
    );

    // `reject` is what the back-edge's guard is written over, so this answer is
    // also the routing decision that sends the flow round again.
    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=sign%2F0"),
            &json!({ "decision": "reject" }),
        )
        .expect("the resume route answers");
    assert_eq!(resumed.status, 202, "{:?}", resumed.body);
    assert_eq!(resumed.json()["wait"], "sign/0", "{:?}", resumed.body);

    let second = wait_for_pause_at(&app, &execution, "sign/1");
    assert_eq!(
        second["interrupts"].as_array().map(Vec::len),
        Some(1),
        "one question at a time: the first is answered and off the board: {second}"
    );
    assert_eq!(
        second["interrupts"][0]["input"]["question"], "ship it?",
        "the second traversal asks the node's own `input:` again: {second}"
    );

    // The first id, sent again now that a *live* pause exists at the same node.
    // A board keyed on the node rather than on the instance path answers this
    // `202` and settles the second question with it.
    let stale = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=sign%2F0"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(stale.status, 409, "{:?}", stale.body);
    assert!(
        stale.json()["error"]
            .as_str()
            .unwrap_or_default()
            .contains("`sign/0` has already been answered"),
        "the refusal names the wait that is settled rather than the one that is \
         open: {:?}",
        stale.body
    );

    // …and the live one is still live, which is the other half of the same
    // claim: the stale answer consumed nothing.
    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=sign%2F1"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(resumed.status, 202, "{:?}", resumed.body);
    assert_eq!(resumed.json()["wait"], "sign/1", "{:?}", resumed.body);

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"]["decisions"],
        json!(["reject", "approve"]),
        "the `append` channel holds one answer per pass, in pass order: \
         {finished}"
    );

    // Two entries for one node, and the trace numbers them the way the wait ids
    // do (`docs/trace.md` §3's `traversal` is grammar 9.4's ordinal).
    let entries: Vec<Value> = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"))
        .iter()
        .filter(|entry| entry["node"] == "sign")
        .cloned()
        .collect();
    assert_eq!(entries.len(), 2, "one entry per traversal: {finished}");
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry["traversal"].clone())
            .collect::<Vec<Value>>(),
        vec![json!(0), json!(1)],
        "{finished}"
    );
    for entry in &entries {
        assert_eq!(entry["outcome"], "completed", "{entry}");
        assert_eq!(entry["human"]["settled"], "resumed", "{entry}");
        assert!(
            entry["human"]["expiresAt"].is_null(),
            "this node declares no `timeout:`, so its wait has no deadline: {entry}"
        );
        assert_eq!(
            entry["writes"],
            json!(["decisions"]),
            "each answer reaches the run through the node's `writes:`: {entry}"
        );
    }
}

/// A `human` node inside a flow a **model** called is answerable like any other.
///
/// The two runtimes compose rather than special-case each other, and this is
/// where that is decided. Grammar 8.7 names the two constructs that may not
/// reach a pause — a `respond: sync` trigger and a detached dispatch — and
/// grammar 7.7 clause 4 walks an agent's `tools:` for both, so a flow-as-tool
/// call reached from an `async` trigger is neither and has to *work*.
///
/// Three things make it work, and each is somebody else's mechanism reaching
/// this composition unchanged: the pause belongs to the **child instance**, so
/// its wait id is that instance's own path with the `human` node's frame on the
/// end (grammar 9.4, PRD resolved q19) — `draft/0/sign/0/approve/0` reads as
/// "the `approve` node, inside the first `sign` call agent node `draft` made";
/// the wait board is keyed by the execution, which the instance shares, so the
/// status route publishes the question and the resume route addresses it; and
/// the agent node's own budget is held still while the pause is open
/// (Decision D102), which is the property `defaults: { timeout: 60s }` here
/// would otherwise break — the node holding the timer is now also the node
/// dividing it across a model route.
#[test]
fn a_pause_inside_a_flow_a_model_called_is_published_and_answered_like_any_other() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The loop asks for sign-off, and the flow it calls stops at its `human`
        // node. The two calls after it are the loop ending and the pinned call,
        // and neither is served until somebody answers.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "sign",
                json!({ "draft": "a drafted answer" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("It is signed off.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "a drafted answer" })),
        ),
    ]);

    let Some(served) = harness::serve("flow-as-tool", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/decisions", &json!({ "question": "ship it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let status = harness::settled(&app, &execution);
    assert_eq!(
        status["status"], "interrupted",
        "the run stopped inside the flow the model called: {status}"
    );
    let interrupts = status["interrupts"]
        .as_array()
        .unwrap_or_else(|| panic!("an interrupted report names its pauses: {status}"));
    assert_eq!(interrupts.len(), 1, "{status}");
    let waiting = &interrupts[0];
    assert_eq!(
        waiting["wait_id"], "draft/0/sign/0/approve/0",
        "the pause is addressed through the tool call that reached it, which is \
         grammar 9.4's frame with the `human` node's on the end: {waiting}"
    );
    assert_eq!(waiting["flow"], "flow.sign", "{waiting}");
    assert_eq!(waiting["node"], "approve", "{waiting}");
    assert_eq!(
        waiting["input"],
        json!({ "draft": "a drafted answer" }),
        "what the human is shown is the child node's own `input:`, built from \
         the arguments the model sent: {waiting}"
    );

    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=draft%2F0%2Fsign%2F0%2Fapprove%2F0"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert!(
        resumed.status == 200 || resumed.status == 202,
        "{resumed:?}"
    );

    let finished = harness::settled(&app, &execution);
    assert_eq!(
        finished["status"], "completed",
        "the answer went back into the instance, the instance answered the tool \
         call, and the loop carried on: {finished}"
    );
    assert_eq!(
        finished["outputs"]["answer"], "a drafted answer",
        "{finished}"
    );

    // The pause is on the entry of the node that held it — inside the instance,
    // reached through the dispatch record the call filed (`docs/trace.md` §3,
    // §5) — and on no node above it.
    let trace = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"));
    let agent = trace
        .iter()
        .find(|entry| entry["node"] == "draft")
        .unwrap_or_else(|| panic!("the agent node has an entry: {finished}"));
    assert!(
        agent["human"].is_null(),
        "the wait was not this node's: {agent}"
    );
    let record = &agent["toolDispatches"][0];
    assert_eq!(record["target"], "flow.sign", "{agent}");
    assert_eq!(record["outcome"], "completed", "{agent}");
    let inner = record["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("the instance's trace is on its record: {agent}"));
    let paused = inner
        .iter()
        .find(|entry| entry["node"] == "approve")
        .unwrap_or_else(|| panic!("the `human` node has an entry: {agent}"));
    assert_eq!(paused["human"]["settled"], "resumed", "{paused}");
    assert!(paused["human"]["settledAt"].is_string(), "{paused}");
}

/// An **agent** node's own `timeout:` does not run while a pause below its tool
/// call is open (Decision D102, grammar 8.7, 9.2).
///
/// [`an_enclosing_nodes_budget_does_not_run_while_a_pause_below_it_is_open`]
/// decides this for a `flow:` node, which is one of the three constructs that
/// can have a pause beneath them; a `map` is the second and an `agent:` node
/// whose `tools:` names a `flow.*` holding a `human` node is the third
/// (grammar 5.4, 7.7 clause 4). The third is not a variation on the first two,
/// because it is the only one where the node holding the timer is also the node
/// **dividing** the budget: a model route bounds each request from
/// `RunContext.deadline`, and that reading is the same budget the hold is
/// freezing. The two halves have to agree, or the first model call after an
/// hour-long wait is refused for a budget the wait spent.
///
/// `flow.hold`'s `draft` node declares `timeout: 2s`; this waits four seconds —
/// twice the budget — before answering. A budget that ran while the human was
/// thinking would have failed the node before the resume was sent, and the
/// execution would be `failed` rather than still holding the pause it started
/// with; a budget that was *cancelled* rather than held would leave the two
/// calls after the wait unbounded, which the node's single execution and its
/// three model calls are what pin.
#[test]
fn an_agent_nodes_budget_does_not_run_while_a_pause_below_its_tool_call_is_open() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "sign",
                json!({ "draft": "a drafted answer" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("It is signed off.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "a drafted answer" })),
        ),
    ]);

    let Some(served) = harness::serve("flow-as-tool", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json("/held-decisions", &json!({ "question": "ship it?" }))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{:?}", started.body);
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let waiting = wait_for_pauses(&app, &execution, 1);
    assert_eq!(
        waiting["interrupts"][0]["wait_id"], "draft/0/sign/0/approve/0",
        "the pause is the child instance's, addressed through the call that \
         reached it: {waiting}"
    );

    // Twice the agent node's budget, spent doing nothing — which is exactly what
    // a human takes.
    std::thread::sleep(Duration::from_secs(4));

    let still = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        still["status"], "interrupted",
        "`draft`'s 2s budget is not the wait's: {still}"
    );
    assert_eq!(
        still["interrupts"][0]["wait_id"], "draft/0/sign/0/approve/0",
        "…and it is the same pause, not a new one a retried node opened: {still}"
    );

    let resumed = app
        .post_json(
            &format!("/executions/{execution}/resume?wait=draft%2F0%2Fsign%2F0%2Fapprove%2F0"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert!(
        resumed.status == 200 || resumed.status == 202,
        "{resumed:?}"
    );

    let finished = harness::settled(&app, &execution);
    assert_eq!(
        finished["status"], "completed",
        "the budget was held still, not cancelled — the loop finished inside \
         what was left of it: {finished}"
    );
    assert_eq!(
        finished["outputs"]["answer"], "a drafted answer",
        "{finished}"
    );

    // One node execution, not a retried one, and the whole loop on its entry:
    // the call that asked for sign-off, the one that ended the loop, and the
    // pinned call — the last two made after the wait, inside the same budget.
    let trace = finished["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("a finished report carries its trace: {finished}"));
    let entry = trace
        .iter()
        .find(|held| held["node"] == "draft")
        .unwrap_or_else(|| panic!("the agent node has an entry: {finished}"));
    assert_eq!(entry["outcome"], "completed", "{entry}");
    assert_eq!(
        entry["attempts"], 1,
        "the node made one attempt: the deadline never fired, so nothing \
         restarted it: {entry}"
    );
    assert_eq!(
        entry["models"].as_array().map(Vec::len),
        Some(3),
        "two loop calls and the pinned one, all on one node execution: {entry}"
    );
    assert!(
        entry["human"].is_null(),
        "the wait was not this node's: {entry}"
    );
    let record = &entry["toolDispatches"][0];
    assert_eq!(record["target"], "flow.sign", "{entry}");
    assert_eq!(
        record["outcome"], "completed",
        "the instance answered the tool call after the pause settled: {entry}"
    );
}

/// `agent-compose run` reports the pause it cannot answer, and exits on a code
/// of its own (grammar 8.7, PRD 5.11).
///
/// Resume is an invocation the generated **app** exposes, so a one-shot CLI run
/// that reaches a `human` node cannot finish. What it must not do is either of
/// the two things that would be easier: wait forever, or carry on with a
/// decision nobody made. It stops, says where the answer goes, writes the trace
/// document with a status of its own, and exits `3` — beside `1` for a run that
/// produced no answer and `2` for a command that could not be run.
#[test]
fn run_reports_the_pause_it_cannot_answer_and_exits_on_its_own_code() {
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
    let said = run.failed();
    assert_eq!(
        run.output.status.code(),
        Some(3),
        "a pause is neither a failed run nor a command that could not be run: {said}"
    );
    assert!(
        said.contains("is waiting for a human and this run has no way to answer")
            && said.contains("POST /executions/:id/resume"),
        "…and the message says where the answer goes: {said}"
    );

    let document = run.trace_document();
    assert_eq!(
        document["status"], "interrupted",
        "the document says which of the three ways the run ended: {document}"
    );
    assert!(
        document["error"]
            .as_str()
            .unwrap_or_default()
            .contains("HumanInterrupt"),
        "{document}"
    );
    let entry = run
        .entries("approve")
        .pop()
        .unwrap_or_else(|| panic!("the `human` node has an entry: {document}"));
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert!(
        entry["human"]["pausedAt"].is_string(),
        "the pause is recorded even though nothing settled it: {entry}"
    );
    assert!(
        entry["human"]["settled"].is_null() && entry["human"]["settledAt"].is_null(),
        "…and a wait the run ended holding has no settlement to record: {entry}"
    );
    assert!(
        entry["fallback"].is_null(),
        "an interrupt transfers control nowhere: {entry}"
    );
}

/// A pause a `run` cannot answer leaves the tool loop's whole story on the
/// agent node's entry.
///
/// The pause the previous test reports arrives at a `human` node the flow's own
/// graph reaches. This one arrives inside a flow a **model** called, which is
/// the composition `serve` answers in
/// `a_pause_inside_a_flow_a_model_called_is_published_and_answered_like_any_other`
/// and which a one-shot run has no way to answer — so the agent node's entry is
/// written by the interrupt path rather than by any of the outcomes
/// `on_error:` decides (grammar 8.7).
///
/// What that entry owes is PRD §9.20's invariant, and it is the reason this is a
/// test rather than a variation: the loop's story is complete inside the
/// `ModelCall`, at the cost of one indirection. A `toolDispatches` record whose
/// `ModelCall` was dropped on the way out would leave the instance findable and
/// the call that started it nowhere — a dispatch record no tool call names, on
/// an entry that also says the agent never called a provider. `docs/trace.md` §9
/// states the same thing as a presence rule: `stores`, `models`, `dispatches`,
/// `toolDispatches` and `inner` reach an aborting entry like any other.
#[test]
fn a_pause_a_run_cannot_answer_leaves_the_loops_calls_on_the_agent_nodes_entry() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "sign",
            json!({ "draft": "a drafted answer" }),
        )]),
    ));

    let Some(run) = harness::run(
        "flow-as-tool",
        "flow.decide",
        &[("question", "ship it?")],
        &provider,
    ) else {
        return;
    };
    let said = run.failed();
    assert_eq!(
        run.output.status.code(),
        Some(3),
        "a pause below an agent node is still a pause: {said}"
    );

    let document = run.trace_document();
    assert_eq!(document["status"], "interrupted", "{document}");
    let entries = run.entries("draft");
    let [entry] = entries.as_slice() else {
        panic!("`draft` ran once: {document}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");
    assert!(
        entry["human"].is_null(),
        "the wait is the child instance's, not this node's: {entry}"
    );

    // Half one: the instance is findable, with the whole of what it did before
    // it asked.
    let dispatched = entry["toolDispatches"]
        .as_array()
        .unwrap_or_else(|| panic!("the instance the model started is reported: {entry}"));
    let [record] = dispatched.as_slice() else {
        panic!("one call, one record: {entry}");
    };
    assert_eq!(record["target"], "flow.sign", "{record}");
    assert_eq!(
        record["outcome"], "failed",
        "the call handed the model nothing back, which is what `failed` says \
         here; the document's `status` is what says the instance is parked \
         rather than broken (`docs/trace.md` §5.1): {record}"
    );
    let key = record["idempotencyKey"]
        .as_str()
        .unwrap_or_else(|| panic!("a dispatch record names its instance: {record}"));
    assert!(
        key.ends_with("/draft/0/sign/0"),
        "…under the frame grammar 9.4 gives the call: {record}"
    );
    let paused = record["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("the instance's trace is on its record: {record}"))
        .iter()
        .find(|one| one["node"] == "approve")
        .cloned()
        .unwrap_or_else(|| panic!("the `human` node has an entry: {record}"));
    assert!(paused["human"]["pausedAt"].is_string(), "{paused}");
    assert!(
        paused["human"]["settled"].is_null(),
        "nothing answered it: {paused}"
    );

    // Half two — the half an interrupt used to drop: the call that started it.
    let models = entry["models"]
        .as_array()
        .unwrap_or_else(|| panic!("the loop's model call is on the entry: {entry}"));
    let [call] = models.as_slice() else {
        panic!("the loop made one call and the pause ended it: {entry}");
    };
    let asked = call["toolCalls"]
        .as_array()
        .unwrap_or_else(|| panic!("…and it records what it asked for: {call}"));
    let [tool_call] = asked.as_slice() else {
        panic!("one tool call: {call}");
    };
    assert_eq!(tool_call["name"], "sign", "{tool_call}");
    assert_eq!(tool_call["target"], "flow.sign", "{tool_call}");
    assert_eq!(tool_call["outcome"], "failed", "{tool_call}");
    assert_eq!(
        tool_call["instance"], record["idempotencyKey"],
        "the indirection PRD §9.20 promises: the call links to the record by \
         string equality, on the entry of a run that ended holding a question: \
         {entry}"
    );
    assert!(
        tool_call["result"].is_null(),
        "the model saw no result: {tool_call}"
    );
}

/// A `run` at a terminal asks the question, takes the answer, and finishes the
/// flow (grammar 8.7, PRD 5.11).
///
/// The gap this closes: a flow with a `human` node in it and no `http` trigger
/// on it was **un-completable** before this. `run` reached the pause and exited
/// `3` pointing at `serve`'s resume route, and a wait lives in the serving
/// process — so the route that answers it belongs to a process this one never
/// started. The composition here is exactly that shape: `flow.assisted` has a
/// pause in the middle and nobody is serving.
///
/// What is asserted is the whole loop and the fact that nothing else moved.
/// stdout is still the run's answer and stderr is where the question went, so a
/// caller parsing one stream is unaffected by a run that had to ask. The pause
/// records what a **resumed** one records, because it was one: the same wait
/// board, the same schema check, the same `deliverHumanAnswer` (grammar 8.7).
#[test]
fn a_run_at_a_terminal_asks_the_pause_it_reaches_and_finishes_the_flow() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "an answer" })),
    ));

    let Some(out) = harness::scratch_project("terminal-answer") else {
        return;
    };
    let run = harness::run_answering(harness::Answering {
        out: &out,
        fixture: "http-trigger",
        flow: "flow.assisted",
        inputs: &[("question", "what is it?")],
        format: None,
        environment: &harness::environment(&provider),
        answers: &[r#"{"decision":"approve","note":"reads right"}"#],
        afterwards: harness::Answers::Closed,
    });
    let said = run.stderr();
    run.succeeded();

    // The question, as a person is given it: which pause, where it is, what they
    // are shown, what their answer has to fit, and the deadline the node
    // declares. It is on **stderr**.
    for part in [
        "pause `approve/0` — flow.assisted node `approve`",
        "\"answer\": \"an answer\"",
        "answer: { decision: \"approve\" | \"reject\", note: string }",
        "answer `approve/0` with one line of JSON: ",
        "  expires: ",
    ] {
        assert!(
            said.contains(part),
            "the prompt is missing `{part}`:\n{said}"
        );
    }

    // …and stdout is still only the flow's outputs, which is what a caller
    // reading one stream depends on.
    assert_eq!(
        run.outputs(),
        json!({ "answer": "an answer", "decision": "approve" }),
        "the answer reached the flow's `outputs:` through the node's `writes:`"
    );

    let document = run.trace_document();
    assert_eq!(
        document["status"], "completed",
        "a run that asked and was answered is a run that finished: {document}"
    );
    assert!(document["error"].is_null(), "{document}");
    let entry = run
        .entries("approve")
        .pop()
        .unwrap_or_else(|| panic!("the `human` node has an entry: {document}"));
    assert_eq!(
        entry["outcome"], "completed",
        "a terminal-answered pause records exactly like a resumed one: {entry}"
    );
    assert_eq!(entry["human"]["settled"], "resumed", "{entry}");
    assert!(entry["human"]["pausedAt"].is_string(), "{entry}");
    assert!(entry["human"]["settledAt"].is_string(), "{entry}");
    assert!(entry["human"]["expiresAt"].is_string(), "{entry}");
    assert!(
        !serde_json::to_string(&entry)
            .expect("the entry serializes")
            .contains("reads right"),
        "no field of the format carries what the human answered (docs/trace.md §11): {entry}"
    );
}

/// One question per pause, asked one at a time and in wait-id order — and the
/// refusals that do not consume a wait.
///
/// A `map` over a flow that pauses is the reachable way one execution holds more
/// than one question at once (grammar 9.4), and it is where a terminal has to
/// decide something a resume route does not: what order to ask in. Each question
/// is the **lowest wait id open when it is asked**, which is what the status
/// route publishes in — so the two instances here, which park together and are
/// both waiting when the first question goes out, are asked in the
/// composition's order rather than the one the scheduler parked them in.
///
/// The guarantee is that and not a total order over the run's pauses; a pause
/// that opens while a question is on the screen is asked after it, which gate 20
/// pins directly (`interactive-pause.mjs`, the `later` section) because it needs
/// a pause opened at an instant a composition cannot ask for.
///
/// The refusals are here rather than in a test of their own because they are the
/// same claim from the other side: two lines that do not answer the first
/// question leave it waiting, so the answers that follow land on the pauses they
/// were typed for and the outputs come back in source-item order.
///
/// `--format json` rides along, because the run *did* have to ask: the document
/// on stdout is the one a completed run always prints, and every prompt went to
/// stderr.
#[test]
fn a_terminal_asks_one_question_per_pause_and_a_refused_answer_asks_again() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(out) = harness::scratch_project("terminal-batch") else {
        return;
    };
    let run = harness::run_answering(harness::Answering {
        out: &out,
        fixture: "http-trigger",
        flow: "flow.batch",
        inputs: &[("questions", r#"["first","second"]"#)],
        format: Some("json"),
        environment: &harness::environment(&provider),
        answers: &[
            "not JSON at all",
            r#"{"decision":"maybe"}"#,
            r#"{"decision":"approve"}"#,
            r#"{"decision":"reject"}"#,
        ],
        afterwards: harness::Answers::Closed,
    });
    let said = run.stderr();
    run.succeeded();

    let asked: Vec<&str> = said
        .lines()
        .filter_map(|line| line.strip_prefix("pause `"))
        .filter_map(|line| line.split('`').next())
        .collect();
    assert_eq!(
        asked,
        ["fan/0/0/sign/0", "fan/0/1/sign/0"],
        "one question per pause, in wait-id order:\n{said}"
    );
    assert!(
        said.contains("an answer is one line of JSON, and this line is not one: "),
        "a line that does not parse is refused as one:\n{said}"
    );
    assert!(
        said.contains(
            "that answer does not fit the `human` node's `output:`, so `fan/0/0/sign/0` is still \
             waiting for one that does: "
        ),
        "…and one the node's `output:` refuses is refused as that, naming the wait it did \
         not consume:\n{said}"
    );

    // Neither refusal spent a turn: the two answers after them are the two the
    // two pauses took, in source-item order (grammar 8.6 rule 5).
    let record = run.outputs();
    assert_eq!(record["status"], "completed", "{record}");
    assert_eq!(
        record["outputs"],
        json!({ "decisions": ["approve", "reject"] }),
        "{record}"
    );
    assert_eq!(
        record["trace_version"], 4,
        "the JSON document a run prints is unchanged in shape by having asked: {record}"
    );
    assert!(
        record["trace_path"].is_string(),
        "…and it still names where the whole trace was written: {record}"
    );
}

/// A wait that runs out while the terminal is asking routes exactly as it would
/// under `serve`, and the withdrawn question says so (grammar 8.7, 9.2).
///
/// The budget is the composition's and it keeps running while a person thinks:
/// nothing about being asked at a terminal holds a `timeout:` still. So this run
/// is given a terminal nobody types at — standard input stays open and empty —
/// and what has to happen is what happens under a resume route: the wait
/// expires, `on_timeout: end` retires the branch (grammar 7.6.3), the run
/// **completes**, and the entry records the expiry.
///
/// What is this surface's own is the last part: the question was on the screen
/// when it stopped being answerable, so the prompt is withdrawn with the
/// sentence a late answer would have been refused with rather than left standing
/// over a wait nothing is holding.
#[test]
fn a_wait_that_expires_while_the_terminal_is_asking_withdraws_the_question() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(out) = harness::scratch_project("terminal-expiry") else {
        return;
    };
    let run = harness::run_answering(harness::Answering {
        out: &out,
        fixture: "http-trigger",
        flow: "flow.lapsing",
        inputs: &[("question", "ship it?")],
        format: None,
        environment: &harness::environment(&provider),
        answers: &[],
        afterwards: harness::Answers::Held,
    });
    let said = run.stderr();
    run.succeeded();

    assert!(
        said.contains("pause `sign_off/0` — flow.lapsing node `sign_off`"),
        "the question was asked:\n{said}"
    );
    assert!(
        said.contains(
            "this question is withdrawn: the wait at `sign_off/0` expired, and `on_timeout` has \
             already routed the execution on (grammar 8.7)"
        ),
        "…and taken away saying what happened to it:\n{said}"
    );

    let document = run.trace_document();
    assert_eq!(
        document["status"], "completed",
        "a wait that ran out is not a failed run — `on_timeout: end` retires the \
         branch (grammar 7.6.3): {document}"
    );
    assert_eq!(
        run.outputs(),
        json!({ "signed": "" }),
        "nothing ran after the sign-off, which is what `end` means in a control-transfer \
         position"
    );
    let entry = run
        .entries("sign_off")
        .pop()
        .unwrap_or_else(|| panic!("the `human` node has an entry: {document}"));
    assert_eq!(entry["human"]["settled"], "expired", "{entry}");
    assert!(entry["human"]["settledAt"].is_string(), "{entry}");
}

/// Standard input ending is the answer surface going away, and a run that loses
/// it ends where a run that never had one ends (grammar 8.7).
///
/// Two questions and one answer: the script said everything it had to say and
/// the second pause has nothing that can reach it. The alternatives are both
/// worse than stopping — waiting for ever is a command that never returns, and
/// carrying on is a graph that routed on a decision nobody made — so this is the
/// exit-`3` path, reached from the *other* direction: not "there was never a
/// surface" but "there is no longer one".
///
/// Which is why the whole report is asserted rather than only the code: the
/// document's `status`, the entry of the node it stopped at, and the absence of
/// a settlement on the pause. A reader cannot tell this run from a headless one,
/// and that is the point.
///
/// It also asserts what the terminal **did not** print. The pipe held one line
/// and its EOF from the moment the run started, so by the time the second pause
/// comes up the surface is already gone — and a question that cannot be answered
/// is not asked. What a reader sees is the first block, `taken.`, and the
/// sentence saying the surface went away; a second block printed in full and
/// withdrawn on the line under it would be a prompt that never existed.
#[test]
fn a_terminal_that_runs_out_of_answers_leaves_the_run_interrupted() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(out) = harness::scratch_project("terminal-eof") else {
        return;
    };
    let run = harness::run_answering(harness::Answering {
        out: &out,
        fixture: "http-trigger",
        flow: "flow.batch",
        inputs: &[("questions", r#"["first","second"]"#)],
        format: None,
        environment: &harness::environment(&provider),
        answers: &[r#"{"decision":"approve"}"#],
        afterwards: harness::Answers::Closed,
    });
    let said = run.failed();
    assert_eq!(
        run.output.status.code(),
        Some(3),
        "a run that stopped holding a question is neither a failed run nor a command that \
         could not be run: {said}"
    );
    assert!(
        said.contains("standard input ended, so nothing can answer this run's pauses any more."),
        "…and it says which surface went away:\n{said}"
    );
    assert!(
        said.contains("is waiting for a human and this run has no way to answer"),
        "{said}"
    );

    // What ended the run is the second question rather than anything about the
    // first: the first was asked in full and its answer taken. The second was
    // never put on the screen at all — the pipe carried its EOF from the start,
    // so the surface was gone before that pause came up, and the loop says so
    // instead of printing a block it would have to withdraw.
    let asked: Vec<&str> = said
        .match_indices("pause `")
        .map(|(at, _)| {
            let rest = &said[at + "pause `".len()..];
            &rest[..rest.find('`').expect("a rendered pause names its wait id")]
        })
        .collect();
    assert_eq!(
        asked,
        ["fan/0/0/sign/0"],
        "one question was asked, and it is the one the script had an answer for:\n{said}"
    );
    assert!(
        said.contains("answer `fan/0/0/sign/0` with one line of JSON: taken.\n"),
        "…and the line the script piped in answered it:\n{said}"
    );

    let document = run.trace_document();
    assert_eq!(document["status"], "interrupted", "{document}");
    let entries = run.entries("fan");
    let [entry] = entries.as_slice() else {
        panic!("the `map` node has one entry: {document}");
    };
    assert_eq!(entry["outcome"], "failed", "{entry}");

    // The pause that stopped the run rides out on the failure, in the aborting
    // entry's `inner` (docs/trace.md §9): it is recorded, and it has no
    // settlement to record.
    //
    // Nothing is asserted about the *answered* instance's dispatch record,
    // because how far it got is not this run's guarantee: a `map` fails as soon
    // as an item does, an answer delivered and an instance run to quiescence are
    // not the same moment, and the surface here goes away in the moment after
    // the answer — so the fan-out is abandoned with that instance somewhere in
    // it. What the answer did is asserted where it is decided, on the screen
    // above. A test that read a record out of that race would be pinning the
    // scheduler rather than the behaviour.
    let unanswered = entry["inner"]
        .as_array()
        .unwrap_or_else(|| panic!("the failing instance's trace is on the entry: {entry}"))
        .iter()
        .find(|inner| inner["node"] == "sign")
        .unwrap_or_else(|| panic!("the parked instance's `human` node has an entry: {entry}"));
    assert!(unanswered["human"]["pausedAt"].is_string(), "{unanswered}");
    assert!(
        unanswered["human"]["settled"].is_null() && unanswered["human"]["settledAt"].is_null(),
        "a wait the run ended holding has no settlement to record: {unanswered}"
    );
}

/// `AGENT_COMPOSE_INTERACTIVE=0` keeps a run to the headless path, standard
/// input or no standard input.
///
/// The variable is what decides the surface where a terminal cannot, and it
/// decides it **both ways**: a supervisor that reads exit `3` and hands the
/// execution to a person needs a `run` that does not quietly start asking
/// because it was given a pipe with something in it. The same invocation as the
/// answering tests above, with the one variable turned off, reaches the pause
/// and exits `3` with the answer unread.
#[test]
fn a_run_told_not_to_ask_reports_the_pause_instead_of_reading_the_answer() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(out) = harness::scratch_project("terminal-refused") else {
        return;
    };
    let mut environment = harness::environment(&provider);
    environment.push((harness::INTERACTIVE.to_string(), "0".to_string()));
    let run = harness::run_answering(harness::Answering {
        out: &out,
        fixture: "http-trigger",
        flow: "flow.batch",
        inputs: &[("questions", r#"["first"]"#)],
        format: None,
        environment: &environment,
        answers: &[r#"{"decision":"approve"}"#],
        afterwards: harness::Answers::Closed,
    });
    let said = run.failed();
    assert_eq!(run.output.status.code(), Some(3), "{said}");
    assert!(
        !said.contains("pause `fan/0/0/sign/0`"),
        "nothing was asked, so nothing was shown:\n{said}"
    );
    assert_eq!(run.trace_document()["status"], "interrupted", "{said}");

    // …and a value the variable does not admit is a command that could not run
    // rather than a setting nobody read (Decision D50).
    let mut mistyped = harness::environment(&provider);
    mistyped.push((harness::INTERACTIVE.to_string(), "yes".to_string()));
    let refused = harness::run_formatted(
        &out,
        "http-trigger",
        "flow.batch",
        &[("questions", r#"["first"]"#)],
        None,
        None,
        &mistyped,
    );
    let complaint = refused.failed();
    assert_eq!(
        refused.output.status.code(),
        Some(2),
        "an environment variable nothing can read is an invocation to fix: {complaint}"
    );
    assert!(
        complaint.contains("`AGENT_COMPOSE_INTERACTIVE=yes` is not one of `1`"),
        "{complaint}"
    );
}

/// Poll the status route until it reports `status`, and answer with that report.
///
/// [`harness::settled`] stops at the three states a run can rest in; this is for
/// the transitions between them — a wait that expires while the execution
/// carries on is a `running` report that only exists for as long as the route it
/// took takes.
fn poll_until(app: &Client, execution: &str, status: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let last: Value = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        if last["status"] == status {
            return last;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "execution `{execution}` never reached `{status}`: {last}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Poll until the execution is holding exactly `count` pauses.
///
/// A fan-out's instances start together and park one at a time, so a report read
/// the instant after `start` answered can hold fewer than the dispatch will.
fn wait_for_pauses(app: &Client, execution: &str, count: usize) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let last: Value = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        if last["interrupts"]
            .as_array()
            .is_some_and(|held| held.len() == count)
        {
            return last;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "execution `{execution}` never held {count} pauses: {last}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Poll until the execution is holding a pause with this **id**.
///
/// The counterpart of [`wait_for_pauses`] for a run whose pauses arrive one
/// after the other rather than together: answering one and waiting for the next
/// passes through a moment where the board holds none at all, so a count is not
/// what says the next question is open.
fn wait_for_pause_at(app: &Client, execution: &str, wait: &str) -> Value {
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let last: Value = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        if last["interrupts"]
            .as_array()
            .is_some_and(|held| held.iter().any(|one| one["wait_id"] == wait))
        {
            return last;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "execution `{execution}` never held a pause at `{wait}`: {last}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// A route collision the **compiler cannot decide** is reported by the app as
/// the collision it is, rather than as a failure to take the address.
///
/// `check/triggers.rs::routes` refuses every pair it can decide — two triggers
/// on one `method:`/`path:`, and a trigger claiming one of the app's own
/// `/executions/…` routes — but grammar 13.3 has the router read a path's
/// parameters "unexamined", so `/reviews/:id` beside `/reviews/:name` is two
/// distinct strings to the compiler and one route to the router. That residue is
/// the app's to report, and what it says is the point: the address is fine,
/// nothing about `127.0.0.1` failed, and a reader sent to look at a port would
/// find nothing wrong with it.
///
/// Written here rather than as a fixture because the composition it needs is one
/// no app can mount: a fixture of it would be a project the rest of this suite
/// builds and can never serve. It still reaches the real compiler — `validate`
/// accepts it, which is half the claim — and the real emitted app.
#[test]
fn serve_names_the_route_collision_the_compiler_could_not_see() {
    let provider = MockProvider::start().expect("a loopback port");
    let scratch = harness::Scratch::new("route-collision");
    let entrypoint = scratch.path().join("main.yml");
    std::fs::write(
        &entrypoint,
        r#"version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${MOCK_API_KEY}
  base_url: ${MOCK_BASE_URL}
model.m:
  provider: provider.p
  id: claude-sonnet-4-6
agent.a:
  model: model.m
  prompt: Do the thing.
  input:
    text: { type: string }
  output:
    result: { type: string }
triggers:
  by_id:
    type: http
    flow: flow.f
    method: GET
    path: /reviews/:id
    input:
      goal: "payload.query['goal']"
  by_name:
    type: http
    flow: flow.f
    method: GET
    path: /reviews/:name
    input:
      goal: "payload.query['goal']"
flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "input.goal" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
"#,
    )
    .expect("the scratch area is writable");

    // Half the claim: the compiler has nothing to say about this composition —
    // two different strings, and the check is exact-pairs-only by design.
    let validated = harness::validate_entrypoint(&entrypoint, "local");
    assert!(
        validated.status.success(),
        "two paths that differ only in a parameter's name are two paths to the compiler: {}",
        String::from_utf8_lossy(&validated.stderr)
    );

    let Some(output) = harness::serve_refused_entrypoint(&entrypoint, &provider, 0) else {
        return;
    };
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    assert_eq!(
        output.status.code(),
        Some(2),
        "the command could not run at all: {stderr}"
    );
    assert!(
        stderr.contains("the app could not mount its routes"),
        "…and says the routes are what failed: {stderr}"
    );
    assert!(
        !stderr.contains("could not listen on"),
        "…and not that the address was refused, which it was not: {stderr}"
    );
}

/// Stopping `agent-compose serve` stops the app it started.
///
/// The command is not the server: it launches the emitted project, which is the
/// process that holds the listening socket, and then waits on it. So a `SIGTERM`
/// delivered to the command alone decides whether stopping the command means
/// anything — a command that only ended itself would leave the app listening on
/// the same port, reparented, answering requests an operator believes they
/// stopped, and holding the data directory of a project they believe is gone.
///
/// Asked as three observations rather than by reading the process table: the app
/// answers, the command is signalled and exits, and the address stops answering.
/// The third is the claim; the first is what makes it about the signal rather
/// than about an app that never started.
///
/// `SIGTERM` is sent to the command's **pid**, not to its process group — the
/// harness puts each served command in a group of its own precisely so this can
/// be a pid-directed question. A group-directed signal would reach the app on
/// its own and decide nothing.
#[test]
#[cfg(unix)]
fn stopping_serve_stops_the_app_it_started() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(mut served) = harness::serve("http-trigger", &provider) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // The app is up: an id nothing started is a `404` from the status route,
    // which is an answer and is what this needs.
    let answered = app
        .get("/executions/exec_nothing-started-this")
        .expect("the app answers before it is stopped");
    assert_eq!(answered.status, 404, "{:?}", answered.body);

    served.signal(libc::SIGTERM);
    let status = served.wait();
    // `0`, and that is the whole chain working rather than an accident: the
    // command forwards the signal, the emitted app's own handler closes the app
    // and exits `0` (`serve()` in `src/serve.ts`), and the command answers with
    // the child's code unchanged — which is the exit-code table `run` and
    // `serve` share. A stop that was asked for is not a failure to report.
    assert_eq!(
        status.code(),
        Some(0),
        "the command answers with the app's own code: {status:?}"
    );

    // …and the address stops answering. Polled rather than asserted once,
    // because the app closes its socket on its own clock — what is being pinned
    // is that it closes it at all, not how fast.
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        match app.get("/executions/exec_nothing-started-this") {
            Err(_) => break,
            Ok(answer) => assert!(
                std::time::Instant::now() < deadline,
                "the app was still listening {}s after the command it was launched by exited, \
                 answering {}: nothing forwarded the signal to it",
                10,
                answer.status
            ),
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// Every generated project type-checks and constructs its graph under the pinned
/// LangGraph version (CLAUDE.md, *Generated-code checks*).
///
/// Two things about this test changed when `build` landed, and both are
/// interface assumptions the harness header says a codegen PR may fix here:
///
/// * the module is `src/graph.ts`, not `./graph.js`. The emitted project has **no
///   build step** — Bun, which PRD §9.18 makes the default runtime and which is
///   what this test launches, runs the TypeScript in `src/` as it is written, and
///   so does the Node fallback since 22.18 — and a `.js` at the root would have
///   had to come from a `tsc` emit `--noEmit` never performs. See the generated
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

        let typecheck = harness::bun()
            .args(["run", "typecheck"])
            .current_dir(&project)
            .output()
            .expect("bun runs");
        assert!(
            typecheck.status.success(),
            "`{name}` does not type-check:\n{}",
            String::from_utf8_lossy(&typecheck.stdout)
        );

        // Constructing every flow's graph is a stronger check than compiling it:
        // an edge to a node that is not registered, or a node reachable only
        // through a control-transfer position that `ends` did not declare, is a
        // runtime error at construction and a green `tsc` either way.
        let construct = harness::bun()
            .args([
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
            .expect("bun runs");
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

// ---------------------------------------------------------------------------
// PRD §7 M3, second bullet — "Built-in tools", the runtime half (resolved q54).
// ---------------------------------------------------------------------------

/// A scratch directory a `builtin-tools` run is bounded to, and the environment
/// that points the fixture at it.
///
/// The `workspace:` every configured entry declares is `${BUILTIN_ROOT}` — a
/// class 2 surface (grammar 4.3) resolved at process start — so the bound is a
/// property of the machine running the graph. Each test gets its own directory,
/// which is what lets the suite run in parallel and what makes "the file is
/// there afterwards" an assertion about *this* run.
///
/// `PATH` travels with it, because the fixture's `env:` declares one: a built-in
/// child is scrubbed (PRD resolved q54 ruling b), so a shell that is to run
/// anything but its own builtins is given the `PATH` this process has.
fn bounded_root(
    provider: &MockProvider,
    purpose: &str,
) -> (harness::Scratch, Vec<(String, String)>) {
    let scratch = harness::Scratch::new(purpose);
    let root = scratch.path().join("root");
    std::fs::create_dir_all(&root).expect("the scratch area is writable");
    let mut environment = harness::environment(provider);
    environment.push(("BUILTIN_ROOT".to_string(), root.display().to_string()));
    environment.push((
        "PATH".to_string(),
        std::env::var("PATH").unwrap_or_else(|_| "/usr/bin:/bin".to_string()),
    ));
    (scratch, environment)
}

/// Where a `builtin-tools` run's tools are bounded to, under its scratch.
fn root_of(scratch: &harness::Scratch) -> std::path::PathBuf {
    scratch.path().join("root")
}

/// The built-in shell, called by a real model loop, doing real work
/// (grammar 5.5, 6.1, Decision D135, PRD resolved q54).
///
/// A built-in is the one binding whose program the **model** writes, so what it
/// is asserted on is the thing only a real call could produce: the stdout of a
/// command the model composed, and — the only one visible outside the transcript
/// — a file that exists on disk afterwards because that command wrote it.
///
/// And what the *trace* has that no other tool call does: the program, which is
/// PRD resolved q54 ruling c's carve-out (`docs/trace.md` §7.4). A run that
/// recorded three `bash` calls and nothing about what they ran would be the
/// record this feature is not allowed to leave behind.
#[test]
fn the_builtin_shell_runs_inside_its_workspace_and_answers_the_model() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-run");
    let root = root_of(&scratch);
    std::fs::write(root.join("notes.txt"), "the note this file held")
        .expect("the root is writable");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new("bash", json!({ "command": "cat notes.txt" })),
                ToolCall::new(
                    "bash",
                    json!({ "command": "printf 'what the graph wrote' > written.txt" }),
                ),
                ToolCall::new(
                    "bash",
                    json!({ "command": "printf 'ran in %s' \"$(basename \"$PWD\")\"" }),
                ),
            ]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "read, wrote and ran" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "do the three things" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "read, wrote and ran");

    // The one effect that outlives the process: the command really wrote.
    assert_eq!(
        std::fs::read_to_string(root.join("written.txt")).expect("the file was written"),
        "what the graph wrote"
    );

    // …and the two that are only visible in what the model was handed back,
    // which is the request that replays this turn's results.
    let asked = provider.requests();
    assert_eq!(asked.len(), 3, "the loop turned twice and then pinned");
    let handed = asked[1].body().to_string();
    for expected in [
        // The first command read a file that was on disk before the run.
        "the note this file held",
        // …and the last ran with the workspace as its working directory, which
        // is what the command printed the basename of.
        "ran in root",
        // …and each answer carries the status the model reasons about.
        "exit_code",
    ] {
        assert!(
            handed.contains(expected),
            "the model was not handed `{expected}` back: {handed}"
        );
    }

    // The trace records each call as a `ToolCallRecord`, under the **tool's**
    // address rather than the built-in's, because a configured built-in is a
    // `tool.*` like any other (grammar 6.1) — and each carries the **program**
    // the model wrote, which is the carve-out of `docs/trace.md` §7.4.
    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| (
                call["name"].as_str().unwrap_or_default().to_string(),
                call["target"].as_str().unwrap_or_default().to_string(),
                call["outcome"].as_str().unwrap_or_default().to_string(),
                call.get("result").is_some(),
            ))
            .collect::<Vec<_>>(),
        vec![
            (
                "bash".to_string(),
                "tool.shell".to_string(),
                "completed".to_string(),
                false
            ),
            (
                "bash".to_string(),
                "tool.shell".to_string(),
                "completed".to_string(),
                false
            ),
            (
                "bash".to_string(),
                "tool.shell".to_string(),
                "completed".to_string(),
                false
            ),
        ],
        "each call is recorded under the tool's address, with no result beside it"
    );
    assert_eq!(
        calls
            .iter()
            .map(|call| call["program"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!({ "tool": "bash", "command": "cat notes.txt", "exitCode": 0 }),
            json!({
                "tool": "bash",
                "command": "printf 'what the graph wrote' > written.txt",
                "exitCode": 0
            }),
            json!({
                "tool": "bash",
                "command": "printf 'ran in %s' \"$(basename \"$PWD\")\"",
                "exitCode": 0
            }),
        ],
        "the trace carries what the model ran and what it exited with, and nothing the \
         command printed (`docs/trace.md` §7.4, §11): {calls:?}"
    );
    assert!(
        !run.trace()
            .iter()
            .any(|entry| entry.to_string().contains("the note this file held")),
        "a command's stdout is a tool's *answer* and stays out of the trace with every \
         other tool's (`docs/trace.md` §11)"
    );
    assert!(provider.snapshot().is_drained());
}

/// One shell per node **activity**: what a command did is still true for the
/// command after it (PRD resolved q54).
///
/// The claim that makes `bash` a tool a model can drive rather than one it has to
/// re-explain itself to: `cd build` and then `make` is one thought. A runtime
/// that forked per call would answer the second command from the workspace root
/// with the model's own account of the first as the only evidence, so what is
/// asserted here is the thing only a live session produces — a working directory
/// and a shell variable that outlived the call that set them.
#[test]
fn a_shells_state_carries_across_the_calls_of_one_node_activity() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-session");
    std::fs::create_dir_all(root_of(&scratch).join("inner")).expect("the root is writable");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "cd inner; kept=carried-over" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "printf 'in %s with %s' \"$(basename \"$PWD\")\" \"$kept\"" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("The shell remembered.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the session carried" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "change directory, then look around" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();

    // The second command's answer, as the model was handed it: the directory the
    // *first* call moved to, and the variable it set.
    let handed = provider.requests()[2].body().to_string();
    assert!(
        handed.contains("in inner with carried-over"),
        "the second command ran in a shell that still held the first's working \
         directory and variables: {handed}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A command that reads **standard input** gets one of its own, rather than the
/// pipe this runtime types its commands into (PRD resolved q54).
///
/// The one place the runtime's own protocol and the model's program would share
/// a channel. `builtin.bash` is a session, so the shell reads its script from
/// standard input and each command is typed into that pipe followed by the marker
/// lines that close it — which is exactly what a command reading standard input
/// would read. Both failures are silent, which is why this is asserted rather
/// than left to the other tests:
///
///  * `read` swallows the line carrying `$?`, so the call settles with **no
///    status** — a completed command recorded as one that did not finish, which
///    `docs/trace.md` §7.4 says is the difference between "the command failed"
///    and "the command did not finish";
///  * `cat` swallows both `printf`s, spends the whole `timeout:` and answers the
///    model with this runtime's marker text as the command's own output.
///
/// So both are run, and what is asserted is a status on each and a session still
/// usable after them.
#[test]
fn a_command_that_reads_standard_input_does_not_eat_the_marker_protocol() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-stdin");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new(
                    "bash",
                    json!({
                        "command":
                            "read -r line; printf 'read exited %s and the line is [%s]' \"$?\" \"$line\""
                    }),
                ),
                ToolCall::new("bash", json!({ "command": "cat" })),
                ToolCall::new(
                    "bash",
                    json!({ "command": "printf 'still in %s' \"$(basename \"$PWD\")\"" }),
                ),
            ]),
        ),
        Script::new(SONNET, Outcome::text("The shell answered all three.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "nothing ate the protocol" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "run three commands, two of which read standard input" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "nothing ate the protocol");

    // Each call completed with a status, which is the assertion: a `read` that
    // had eaten the status line would be recorded with no `exitCode` at all, and
    // a `cat` that had eaten the markers would be recorded `timedOut`.
    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["program"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!({
                "tool": "bash",
                "command":
                    "read -r line; printf 'read exited %s and the line is [%s]' \"$?\" \"$line\"",
                "exitCode": 0
            }),
            json!({ "tool": "bash", "command": "cat", "exitCode": 0 }),
            json!({
                "tool": "bash",
                "command": "printf 'still in %s' \"$(basename \"$PWD\")\"",
                "exitCode": 0
            }),
        ],
        "a command that reads standard input reads its own, so each of the three completed \
         with the status `docs/trace.md` §7.4 records where one did: {calls:?}"
    );

    // …and what the model was handed: the `read` reported the end of an input
    // with nothing in it rather than a line of this runtime's protocol, and the
    // third command proves the session survived the two.
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("read exited 1 and the line is []"),
        "the `read` found the end of an empty input rather than the marker line: {handed}"
    );
    assert!(
        !handed.contains("__agent_compose_"),
        "no answer carries this runtime's own marker protocol back to the model: {handed}"
    );
    assert!(
        handed.contains("still in root"),
        "the session outlived both of them, in the workspace it started in: {handed}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A command that exits nonzero **comes back to the model** with its status
/// (PRD resolved q54, Decision D119).
///
/// The split this feature moves: for every other binding an exit status outside
/// `expect_exit:` is the composition's own contract failing, and for a built-in
/// there is no such contract — the model wrote the command, so the status is a
/// *fact it asked for* and it is the one that decides what to do about it. A
/// runtime that failed the node here would end a run over a `grep` that matched
/// nothing.
#[test]
fn a_command_that_exits_nonzero_comes_back_to_the_model_with_its_status() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-nonzero");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "echo 'the command said why' >&2; (exit 3)" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "printf 'recovered'" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Handled it.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the second command worked" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "run something that fails, then recover" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "the second command worked");

    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("\\\"exit_code\\\":3") && handed.contains("the command said why"),
        "the model was handed the status and what the command said on stderr: {handed}"
    );

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["outcome"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["completed", "completed"],
        "a command that exited nonzero is a call that **completed**: the tool ran and \
         answered, and what it answered is the model's to act on: {calls:?}"
    );
    assert_eq!(
        calls[0]["program"]["exitCode"], 3,
        "…and the status is what the trace records of it (`docs/trace.md` §7.4): {calls:?}"
    );
}

/// A command that outruns its binding's `timeout:` is **killed and reported**
/// (grammar 6.1, PRD resolved q54).
///
/// `agent.impatient` binds the same built-in at one second where `agent.worker`
/// binds it at thirty, so what this decides is the *binding's* bound rather than
/// a constant: the command asks for far longer than either. The deadline answers
/// the model — the loop goes on, the model is told what it hit and that its shell
/// state went with it, and the run completes — which is the same D119 split the
/// exit status is on.
#[test]
fn a_command_that_outruns_its_timeout_comes_back_as_a_timeout_result() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-timeout");

    // Written to leave a **grandchild** holding the output pipes after the shell
    // is killed, which is the shape a deadline is silently lost in: a runtime
    // that waited for the child's streams to close would wait for the `sleep`
    // rather than for its own timer, and the bound would become the command's to
    // honour rather than the composition's.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "sleep 30 & wait" }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::text("It did not finish, so I will report that."),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the command was cut short" })),
        ),
    ]);

    let started = std::time::Instant::now();
    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.deadline",
        &json!({ "goal": "wait forever" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "the command was cut short");
    // Comfortably under the `sleep`, and comfortably over the deadline plus a
    // build: what this rules out is the run having waited for the grandchild.
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "the command was killed at its deadline rather than waited out"
    );

    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("timed_out") && handed.contains("`1s`"),
        "the model was told the bound it hit, in the units the composition wrote: {handed}"
    );
    assert!(
        handed.contains("fresh shell"),
        "…and that the session went with it, which is the part it has to know: {handed}"
    );

    let calls = tool_calls_of(&run, "do");
    assert_eq!(calls[0]["outcome"], "completed");
    assert_eq!(
        calls[0]["program"],
        json!({ "tool": "bash", "command": "sleep 30 & wait", "timedOut": true }),
        "the trace records the command and that it was cut short, with no exit status \
         because none completed (`docs/trace.md` §7.4): {calls:?}"
    );
}

/// Arguments a built-in's own contract refuses go **back to the model**, which is
/// the other side of the split (Decision D119, PRD resolved q22 and q54).
///
/// The first answer calls `bash` with no `command` and no `restart` — the one
/// shape the schema admits and the tool cannot, since the provider-defined tool
/// takes either. The loop hands the refusal back, the model corrects itself, and
/// the run completes. A runtime that failed the node on a refusal would end the
/// run here, and one that answered the bad call would run a command nobody wrote.
#[test]
fn arguments_a_builtin_refuses_bounce_back_to_the_model() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-refusal");
    std::fs::write(
        root_of(&scratch).join("notes.txt"),
        "the note this file held",
    )
    .expect("the root is writable");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("bash", json!({}))]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "cat notes.txt" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Now I have it.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "read it on the second try" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "read the notes" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "read it on the second try");

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["outcome"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["refused", "completed"],
        "the refusal came back to the model and the corrected call ran: {calls:?}"
    );
    let refusal = calls[0]["error"].as_str().unwrap_or_default();
    assert!(
        refusal.starts_with("ToolCallRefused: ") && refusal.contains("`restart`"),
        "the refusal is recorded in the shape `docs/trace.md` §3 gives one, and says which \
         of the two the model may send: {refusal}"
    );
    // …and the model was handed the same sentence, which is what it corrects on.
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("no `command` and no `restart`"),
        "the refusal reached the model: {handed}"
    );
    assert!(provider.snapshot().is_drained());
}

/// A key the built-in's argument set does not carry (grammar 5.5, Decision
/// D119, PRD resolved q54).
///
/// The set is **closed**, and it is the compiler's rather than the model's: on
/// the Messages wire `builtin.files` goes out as `text_editor_20250728` with no
/// schema at all, so what arrives is whatever the model was trained to fill —
/// the declared names for the most part, and now and then a key from some other
/// editor it has seen, or one it invented for the call it had in mind. The
/// compiler's Zod is `.strict()`, so that call is refused before the tool is
/// reached.
///
/// What has to hold for that to be a *cost* rather than a dead end is the whole
/// of D119: the refusal names the key it refused, the model corrects, and the
/// run completes — spending one turn of `max_tool_iterations`, which is what
/// `agent-compose docs tools` tells an author to budget for. The vendor's own
/// `view_range` is **not** an example of this any more: it is implemented, and
/// `the_file_tool_creates_edits_and_views_inside_its_workspace` drives it.
///
/// And the presence rule the other half of the campaign wrote down: this call is
/// refused **before** it reaches the tool, so its record carries no `program`
/// (`docs/trace.md` §7.3) — the arguments the model chose are in the `error`,
/// where a refusal is allowed to quote them, and nowhere else.
#[test]
fn an_argument_outside_a_builtins_schema_is_refused_naming_the_key() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-unknown-key");
    std::fs::write(
        root_of(&scratch).join("notes.txt"),
        "one\ntwo\nthree\nfour\n",
    )
    .expect("the root is writable");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({ "command": "view", "path": "notes.txt", "line_numbers": false }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({ "command": "view", "path": "notes.txt" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("The whole file will do.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "read the whole file instead" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.edit",
        &json!({ "goal": "read the notes" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "read the whole file instead");

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["outcome"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["refused", "completed"],
        "the unknown key bounced and the corrected call ran: {calls:?}"
    );
    let refusal = calls[0]["error"].as_str().unwrap_or_default();
    assert!(
        refusal.starts_with("ToolCallRefused: ")
            && refusal.contains("Unrecognized key")
            && refusal.contains("line_numbers"),
        "the refusal **names the key it refused**, which is the difference between a model \
         that corrects itself and one that guesses (PRD G3): {refusal}"
    );
    assert!(
        calls[0].get("program").is_none(),
        "a call refused before it reached the tool records no program — nothing ran, and \
         `docs/trace.md` §7.3 says the record carries one only where the built-in was \
         reached: {calls:?}"
    );
    // …and the model was handed the same sentence, which is what it corrects on.
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("line_numbers"),
        "the refusal reached the model with the offending key in it: {handed}"
    );
    assert!(provider.snapshot().is_drained());
}

/// The file tool, round-tripped by a real model loop: create, edit, view
/// (grammar 5.5, 6.1, PRD resolved q54).
///
/// The operations and their parameter names are the provider-defined text
/// editor's, because on the Messages wire this *is* that tool — including the
/// `view_range` a fourth call reads a window with, which is how a file longer
/// than one answer is read. What is asserted is the file system afterwards and
/// the two views the model was handed, which together are the only proof the
/// calls were one file's story rather than four separate answers.
#[test]
fn the_file_tool_creates_edits_and_views_inside_its_workspace() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-editing");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "create",
                    "path": "notes/plan.md",
                    "file_text": "one\ntwo\nthree\n",
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "str_replace",
                    "path": "notes/plan.md",
                    "old_str": "two",
                    "new_str": "the second line",
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({ "command": "view", "path": "notes/plan.md" }),
            )]),
        ),
        // …and the same file read as a **window**, which is the provider-defined
        // editor's own `view_range` and the way a file longer than one answer is
        // read at all. `-1` as the last line reads to the end of the file.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({ "command": "view", "path": "notes/plan.md", "view_range": [2, -1] }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("The file reads the way it should.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "wrote it, edited it, read it back" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.edit",
        &json!({ "goal": "write a plan and fix its second line" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(
        run.outputs()["summary"],
        "wrote it, edited it, read it back"
    );

    // On disk: the file the `create` wrote, with the edit the `str_replace`
    // made, in a directory neither of them had to make first.
    assert_eq!(
        std::fs::read_to_string(root_of(&scratch).join("notes/plan.md"))
            .expect("the file was created inside the workspace"),
        "one\nthe second line\nthree\n"
    );

    // …and what the model was handed for the `view`: the file, numbered, which
    // is what makes an `insert_line` a line the model can count to.
    let viewed = provider.requests()[3].body().to_string();
    assert!(
        // Two backslashes, because the tool result is a JSON string *inside* a
        // JSON body: the tab is escaped once by `JSON.stringify` and again by
        // the reading here.
        viewed.contains(r"     1\\tone") && viewed.contains(r"     2\\tthe second line"),
        "the view came back numbered: {viewed}"
    );
    // …and what it was handed for the **window**: those lines and no others,
    // under the file's own numbers, which is what makes a `str_replace`
    // composed out of one window match what the next one shows.
    let answering = provider.requests()[4].body()["messages"]
        .as_array()
        .and_then(|turns| turns.last())
        .map(|turn| turn["content"][0]["content"].clone())
        .unwrap_or_default();
    let ranged: serde_json::Value = serde_json::from_str(
        answering
            .as_str()
            .expect("the tool result is the JSON the runtime answered with"),
    )
    .expect("…and it parses");
    assert_eq!(
        ranged["content"], "     2\tthe second line\n     3\tthree",
        "`view_range: [2, -1]` answered with line 2 to the end of the file, numbered 2 and 3 — \
         a window, not a re-numbered copy of one: {ranged}"
    );

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["program"].clone())
            .collect::<Vec<_>>(),
        vec![
            json!({
                "tool": "files",
                "operation": "create",
                "path": "notes/plan.md",
                "change": "wrote 14 bytes"
            }),
            json!({
                "tool": "files",
                "operation": "str_replace",
                "path": "notes/plan.md",
                "change": "replaced one occurrence at line 2"
            }),
            json!({ "tool": "files", "operation": "view", "path": "notes/plan.md" }),
            json!({ "tool": "files", "operation": "view", "path": "notes/plan.md" }),
        ],
        "the trace carries the operation, the path and a sentence about the edit — never \
         the file's contents (`docs/trace.md` §7.4, §11), and a windowed read records the \
         same two fields as any other, because the window bounds the *answer*: {calls:?}"
    );
    assert!(
        !run.trace()
            .iter()
            .any(|entry| entry.to_string().contains("the second line")),
        "…and what the file holds is a tool's answer, which stays out of the format"
    );
}

/// A path that leaves the workspace is **refused to the model**, which corrects
/// itself and finishes the run (PRD resolved q54, Decision D119).
///
/// The bound is the workspace and the refusal is the shape of it: the model chose
/// the path, so it is the one that can choose another — and a runtime that failed
/// the node instead would end a run over a `..` the next call would have fixed.
/// What makes this an assertion about a *bound* rather than about a message is
/// the file outside the workspace, which is read back afterwards.
#[test]
fn a_file_path_that_leaves_the_workspace_bounces_back_to_the_model() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-escape");
    let outside = scratch.path().join("outside.txt");
    std::fs::write(&outside, "the file outside the workspace").expect("the scratch is writable");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "create",
                    "path": "../outside.txt",
                    "file_text": "clobbered",
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "create",
                    "path": "inside.txt",
                    "file_text": "written where it belongs",
                }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Wrote it in the workspace instead.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "stayed inside" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.edit",
        &json!({ "goal": "write the file" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "stayed inside");

    // The bound, as the file system saw it.
    assert_eq!(
        std::fs::read_to_string(&outside).expect("the file outside is still there"),
        "the file outside the workspace",
        "a path that resolves outside the workspace is refused rather than written"
    );
    assert_eq!(
        std::fs::read_to_string(root_of(&scratch).join("inside.txt"))
            .expect("the corrected call wrote inside the workspace"),
        "written where it belongs"
    );

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["outcome"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["refused", "completed"],
        "{calls:?}"
    );
    assert_eq!(
        calls[0]["program"],
        json!({ "tool": "files", "operation": "create", "path": "../outside.txt" }),
        "a refused call still records what the model asked for: {calls:?}"
    );
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("resolves outside this tool's workspace"),
        "the model was told what was wrong with the path it chose: {handed}"
    );
}

/// A file inside the workspace that carries a **second name** is refused for
/// writing (PRD resolved q54, Decision D119).
///
/// The one escape the resolution check cannot see, and the reason the refusal
/// lives at the write instead. `targetWithinWorkspace` answers "where is this
/// path really", which is the whole answer for a symbolic link — a link has a
/// target — and no answer at all for a hard one: a second directory entry for one
/// inode has nothing to resolve, so the path is inside the workspace and the
/// bytes it names are also outside it. A `str_replace` through the name inside
/// would rewrite what the name outside reads, with every path check passed.
///
/// Reachable through a **configured** `workspace:` that something else populated
/// — a checkout, a package manager that links rather than copies — which is the
/// shape `tool.editor` has. What is asserted is the file outside, read back after
/// the run: a refusal message alone would pass against a runtime that wrote the
/// file and then said something about it.
#[test]
fn a_write_to_a_file_with_a_second_name_is_refused_to_the_model() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-hard-link");
    let outside = scratch.path().join("outside.txt");
    std::fs::write(&outside, "the file outside the workspace").expect("the scratch is writable");
    // The second name, inside the workspace. Not a symlink: there is no target
    // for a resolution to follow, which is the whole point of the case.
    std::fs::hard_link(&outside, root_of(&scratch).join("linked.txt"))
        .expect("the scratch and its root are one filesystem");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "str_replace",
                    "path": "linked.txt",
                    "old_str": "outside the workspace",
                    "new_str": "clobbered",
                }),
            )]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "str_replace_based_edit_tool",
                json!({
                    "command": "create",
                    "path": "copy.txt",
                    "file_text": "written where it belongs",
                }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("I worked on a copy instead.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "left the linked file alone" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.edit",
        &json!({ "goal": "edit the file" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "left the linked file alone");

    // The bound, as the file system saw it: the edit did not land, on either
    // name.
    assert_eq!(
        std::fs::read_to_string(&outside).expect("the file outside is still there"),
        "the file outside the workspace",
        "a write through a second name is refused rather than made"
    );
    assert_eq!(
        std::fs::read_to_string(root_of(&scratch).join("copy.txt"))
            .expect("the corrected call wrote inside the workspace"),
        "written where it belongs"
    );

    let calls = tool_calls_of(&run, "do");
    assert_eq!(
        calls
            .iter()
            .map(|call| call["outcome"].as_str().unwrap_or_default().to_string())
            .collect::<Vec<_>>(),
        ["refused", "completed"],
        "the refusal came back to the model and the corrected call ran: {calls:?}"
    );
    assert_eq!(
        calls[0]["program"],
        json!({ "tool": "files", "operation": "str_replace", "path": "linked.txt" }),
        "a refused call still records what the model asked for, and no `change`, because \
         nothing changed (`docs/trace.md` §7.4): {calls:?}"
    );
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("names point at that one file"),
        "the model was told what it was about to write through: {handed}"
    );

    // …and the read is deliberately left alone: a path inside the workspace is
    // inside the bound whatever else names it, so refusing the `view` would buy
    // nothing a `bash` could not undo.
    assert!(provider.snapshot().is_drained());
}

/// A built-in's children see **what the binding declared and nothing else**
/// (PRD resolved q54 ruling b).
///
/// Both directions in one test, because either alone passes against a runtime
/// that is wrong the other way: a variable this harness holds is invisible to a
/// scrubbed shell and visible to one whose binding wrote `inherit_env: true`,
/// while the variable the binding *declared* is there for the first and not for
/// the second, which declares none of its own.
///
/// The scrubbed direction is what keeps a placement's environment manifest the
/// whole answer to what a machine is asked for (resolved q41): a command a model
/// wrote cannot read a credential nobody handed it.
#[test]
fn a_shells_environment_holds_what_its_binding_declared_and_nothing_else() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, mut environment) = bounded_root(&provider, "builtins-env");
    environment.push((
        "HARNESS_SECRET".to_string(),
        "the value this process holds".to_string(),
    ));

    let look = json!({
        "command": "printf '[%s][%s]' \"${HARNESS_SECRET-unset}\" \"${DECLARED-unset}\"",
    });
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("bash", look.clone())]),
        ),
        Script::new(SONNET, Outcome::text("That is what I can see.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "looked at the environment" })),
        ),
    ]);

    let Some(scrubbed) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "print what you can see" }),
        &environment,
    ) else {
        return;
    };
    scrubbed.succeeded();
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("[unset][the value the binding wrote]"),
        "a scrubbed child sees the binding's declared variables and not this process's: \
         {handed}"
    );

    // …and the same command under a binding that opted into inheriting, which is
    // the explicit widening for the machines where that is the point.
    provider.reset();
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("bash", look)]),
        ),
        Script::new(SONNET, Outcome::text("That is what I can see.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "looked again" })),
        ),
    ]);
    let Some(inherited) = harness::invoke_with(
        "builtin-tools",
        "flow.inherited",
        &json!({ "goal": "print what you can see" }),
        &environment,
    ) else {
        return;
    };
    inherited.succeeded();
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("[the value this process holds][unset]"),
        "`inherit_env: true` is the opt-in, and it inherits rather than adding to a \
         declared set this binding does not have: {handed}"
    );
}

/// A binding that wrote no `workspace:` works in one **fresh directory per
/// execution**, removed when the run settles (grammar 6.1, PRD resolved q54).
///
/// The default is the shape an author reaches for first, and it is the one whose
/// lifetime nothing else in the composition states: the directory is the
/// execution's, under the project's own data directory, and it goes when the run
/// does — `scope: execution` read for a directory. The run prints where it is,
/// which is the only way a test outside the process can find out; the assertion
/// is that the path is where this format says and that nothing is there
/// afterwards.
///
/// The configured half is the contrast, and it is asserted by every other test
/// in this section: `${BUILTIN_ROOT}` is the composition's directory rather than
/// the run's, so what a run wrote there is still there when the run has gone —
/// which is what
/// `the_builtin_shell_runs_inside_its_workspace_and_answers_the_model` reads
/// `written.txt` back out of, after its run has ended.
#[test]
fn the_default_workspace_is_the_executions_own_and_goes_when_the_run_settles() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-workspace");

    provider.enqueue_all([
        Script::new(
            GPT41,
            Outcome::tool_calls(vec![
                ToolCall::new(
                    "str_replace_based_edit_tool",
                    json!({
                        "command": "create",
                        "path": "made.txt",
                        "file_text": "by the file tool\n",
                    }),
                ),
                ToolCall::new("bash", json!({ "command": "cat made.txt; pwd" })),
            ]),
        ),
        Script::new(GPT41, Outcome::text("Both tools saw one directory.")),
        Script::new(
            GPT41,
            Outcome::structured(json!({ "summary": "one workspace, two tools" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.elsewhere",
        &json!({ "goal": "write a file and read it back" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();

    // What the shell printed: the file the *file tool* wrote — so both built-ins
    // of this execution took one directory — and where that directory is.
    let handed = provider.requests()[1].body().to_string();
    assert!(
        handed.contains("by the file tool"),
        "the shell read the file the file tool wrote, which is what \"shared by every \
         built-in of that execution\" means: {handed}"
    );
    let printed = handed
        .split(".agent-compose/workspaces/")
        .nth(1)
        .and_then(|rest| rest.split(['\\', '"', ' ']).next())
        .expect("the shell printed its working directory");
    let workspace = run
        .project
        .join(".agent-compose")
        .join("workspaces")
        .join(printed);
    assert!(
        printed.starts_with("exec_"),
        "the default workspace is named for the execution it belongs to: {printed}"
    );
    assert!(
        !workspace.exists(),
        "the execution settled and its workspace is still at `{}`: a directory nobody \
         configured belongs to the run (grammar 6.1)",
        workspace.display()
    );
}

/// Both built-ins are declared on the Messages wire as the **provider-defined
/// tool types** (PRD resolved q54 ruling d).
///
/// The half of that ruling the PRD calls the point rather than the incident: a
/// model is *trained* against `bash_20250124` and `text_editor_20250728`, and a
/// request that offered the same two tools as ordinary custom tools carrying
/// this compiler's own `input_schema` would still work — the arguments come
/// back the same, which is what lets one set of handlers serve every wire — and
/// engage none of that training. Nothing about the graph's own behaviour
/// distinguishes the two, so the assertion is on the **request**, entry for
/// entry: the dated type, the name that type dictates, and nothing beside them.
///
/// The mock is the other half of the check and cannot be skipped past: it holds
/// a provider-defined entry to `{type, name, cache_control}` and to the name its
/// dated type requires (`crates/mock-provider` `WIRE-NOTES` (25)), so a request
/// naming `text_editor_20250728` `files` is a 400 rather than a run.
#[test]
fn the_builtins_go_out_as_provider_defined_tools_on_the_messages_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-messages");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "printf 'ran on the trained wire'" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Done.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the trained wire ran it" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.both",
        &json!({ "goal": "run one command" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "the trained wire ran it");

    let asked = provider.requests();
    let declared = asked[0].body();
    assert_eq!(
        declared["tools"],
        json!([
            { "type": "bash_20250124", "name": "bash" },
            { "type": "text_editor_20250728", "name": "str_replace_based_edit_tool" },
        ]),
        "a built-in reaches the Messages wire as its dated provider-defined type and the \
         name that type dictates — an `input_schema` beside it, or a `description`, is this \
         compiler declaring an ordinary custom tool and losing the trained behaviour the \
         ruling is for: {declared}"
    );
    assert!(
        asked[1]
            .body()
            .to_string()
            .contains("ran on the trained wire"),
        "and the loop really ran it: {}",
        asked[1].body()
    );
    assert!(provider.snapshot().is_drained());
}

/// Both built-ins drive a loop on a **non-Anthropic wire** (PRD resolved q54
/// ruling d).
///
/// The Messages wire declares these as the provider-defined tool types, and this
/// is the other half of that ruling: on Chat Completions there are no such types,
/// so the same two tools go out as ordinary function tools carrying this
/// compiler's own schemas — same names, same argument names, same handlers. What
/// the test reads is the request, because the *point* of the ruling is what the
/// wire carries.
#[test]
fn the_builtins_go_out_as_function_tools_on_the_openai_wire() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-openai");

    provider.enqueue_all([
        Script::new(
            GPT41,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "printf 'ran on the other wire'" }),
            )]),
        ),
        Script::new(GPT41, Outcome::text("Done.")),
        Script::new(
            GPT41,
            Outcome::structured(json!({ "summary": "the other wire ran it" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.elsewhere",
        &json!({ "goal": "run one command" }),
        &environment,
    ) else {
        return;
    };
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "the other wire ran it");

    let asked = provider.requests();
    let declared = asked[0].body();
    let tools = declared["tools"]
        .as_array()
        .expect("the request declares the agent's tools");
    assert_eq!(
        tools
            .iter()
            .map(|tool| (
                tool["type"].as_str().unwrap_or_default().to_string(),
                tool["function"]["name"]
                    .as_str()
                    .unwrap_or_default()
                    .to_string(),
            ))
            .collect::<Vec<_>>(),
        vec![
            ("function".to_string(), "bash".to_string()),
            (
                "function".to_string(),
                "str_replace_based_edit_tool".to_string()
            ),
        ],
        "on this wire a built-in is a function tool under the name the other wire's \
         provider-defined type dictates: {declared}"
    );
    assert!(
        tools[0]["function"]["parameters"]["properties"]["command"].is_object(),
        "…carrying this compiler's own schema, which is what makes one set of handlers \
         serve both wires: {declared}"
    );
    assert!(
        provider.requests()[1]
            .body()
            .to_string()
            .contains("ran on the other wire"),
        "and the loop really ran it"
    );
}

/// A run killed mid-graph resumes **without running its recorded `bash` again**
/// (PRD resolved q54, `docs/durability.md` §3.2).
///
/// The command appends a line to a file outside the graph's own state, so
/// "it ran twice" is a fact a test can count rather than one it has to infer:
/// after the crash the file holds one line, and after the resume it still holds
/// one. A built-in that reached the journal like every other tool execution is
/// the only way that happens.
#[test]
fn a_resumed_run_consumes_a_recorded_builtin_instead_of_running_it_again() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-resume");
    let ledger = root_of(&scratch).join("ledger.txt");

    provider.enqueue_all([
        // The first node's loop: one command with a side effect, then the pinned
        // answer that ends the node.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "echo ran >> ledger.txt" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Recorded.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the first pass" })),
        ),
    ]);
    // The second node's first call — a loop call, since the agent carries tools
    // — held past the kill, so the frontier lands with the whole of the first
    // node recorded and none of the second.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::text("Nothing more to do.").after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("builtin-tools", "resume-builtin")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the builtin-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "goal=append a line"],
        &environment,
        |_| provider.snapshot().requests >= 4,
    );
    assert_eq!(
        harness::lines_in(&ledger),
        1,
        "the first generation ran the command once"
    );

    provider.reset();
    // What a correct replay still owes the provider: the second node's loop and
    // its pinned answer. The first node's three calls — and the `bash` inside
    // them — are behind the frontier.
    provider.enqueue_all([
        Script::new(SONNET, Outcome::text("Nothing more to do.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the second pass" })),
        ),
    ]);

    let resumed = harness::resume(
        &project,
        "builtin-tools",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["relayed"], "the first pass",
        "the replayed node's answer reaches the resumed run's state: {answered}"
    );
    assert_eq!(
        answered["outputs"]["final"], "the second pass",
        "{answered}"
    );

    assert_eq!(
        harness::lines_in(&ledger),
        1,
        "the resumed generation was handed the recorded answer and ran no command \
         (`docs/durability.md` §3.2): the ledger would hold two lines if it had"
    );
}

/// The deadline kills the command's **process group**, so work the command
/// forked stops with it (grammar 5.5, Decision D124).
///
/// The shell is almost never where the work is: `npm run build`, `a | b`,
/// `(cd sub && make)` are all `bash` *forking*, and a `SIGKILL` aimed at the
/// shell's own pid ends the shell while every one of those children keeps
/// running — inside the very workspace the binding bounded them to, with the
/// model already told the command was cut short. Under `retry:` that is two
/// generations of one command writing one workspace; with a loop that carries on
/// it is the *next* command reading files a killed one is still producing. Either
/// way the `timeout:` would be a message rather than a bound.
///
/// So the command forks a **foreground** subshell — not a backgrounded job,
/// which is the only shape the grammar ever set aside — that writes nothing
/// until long after the one-second deadline, and the file system is asked
/// afterwards. `; echo done` is load-bearing: without a command after it, `bash`
/// may `exec` the subshell in place of itself, which is the one shape where
/// killing the shell would have killed the work by accident and this test would
/// pass against a runtime that never fixed anything.
#[test]
fn a_killed_command_takes_the_work_it_forked_with_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-forked");
    let escaped = root_of(&scratch).join("escaped.txt");

    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "( sleep 4; printf escaped > escaped.txt ); echo done" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Cut short.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the command was cut short" })),
        ),
    ]);

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.deadline",
        &json!({ "goal": "fork something that would outlive the deadline" }),
        &environment,
    ) else {
        return;
    };

    // The command was cut short at the binding's bound and the run went on.
    run.succeeded();
    let calls = tool_calls_of(&run, "do");
    assert_eq!(calls[0]["program"]["timedOut"], json!(true), "{calls:?}");

    // Past the moment the forked work would have written, had anything been left
    // running to write. The run is long over; this is the file system being asked
    // whether the bound was a fact.
    std::thread::sleep(Duration::from_secs(8));
    assert!(
        !escaped.exists(),
        "the subshell the killed command forked wrote `{}` after the deadline had \
         already answered the model: the kill ended the shell and left the work running \
         (grammar 5.5, D124)",
        escaped.display()
    );
}

/// A run **asked to stop** takes the shell it was running with it, and still
/// stops (grammar 5.5, Decision D124).
///
/// Detaching the shell is what puts its process group within the deadline's
/// reach, and the same move puts it outside the *terminal's*: its group is no
/// longer the foreground one, so the `SIGINT` a person types reaches this process
/// and nothing below it. Left there, `Ctrl-C` would have traded the deadline's
/// orphan for its own — a build still writing into the workspace with the graph
/// gone — and done it where an `exec:` tool's child dies. So the runtime sweeps
/// the live groups itself.
///
/// The sweep is a signal handler, which is the other half of what this asserts
/// and the half that would hurt a person rather than a file: a handler that ate
/// the signal instead of handing it back would leave a terminal wedged on a run
/// that will not stop. So both are checked — the process **ended**, promptly, and
/// the command it was running did not outlive it.
#[cfg(unix)]
#[test]
fn a_run_asked_to_stop_takes_its_command_with_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, environment) = bounded_root(&provider, "builtins-stopped");
    let root = root_of(&scratch);
    let started = root.join("started.txt");
    let escaped = root.join("escaped.txt");

    // The marker is written by the **subshell**, not by the shell above it, so
    // the moment the test sees it the fork it is about has certainly happened.
    // Thirty seconds is `agent.worker`'s bound, which nothing here reaches: what
    // ends this command is the signal, not the deadline.
    //
    // **The subshell's own sleep is the window the signal has to arrive in**, and
    // it is twenty seconds rather than a handful because the thing being measured
    // is not a delay this test controls: the marker is polled, the signal is
    // delivered, and the runtime's handler then sweeps the group — three steps
    // that take milliseconds on an idle machine and are all schedulable under a
    // loaded one. A window narrow enough for load to close is a test that fails
    // for a reason that is not the defect, and the escape is checked from the
    // fork rather than from here, so widening it costs no margin.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "bash",
            json!({
                "command":
                    "( printf started > started.txt; sleep 20; printf escaped > escaped.txt ); \
                     echo done"
            }),
        )]),
    ));

    let Some((project, built)) = harness::build_under_toolchain("builtin-tools", "stop-builtin")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the builtin-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    // When the fork happened, on this process's clock. The escape is due a fixed
    // number of seconds *after that instant*, so it is the instant the check
    // below waits from: measuring from the run's end instead would make the
    // margin depend on how long the signal took to be delivered, which is the
    // one quantity this test has no control over.
    let marked = std::cell::Cell::new(None::<std::time::Instant>);
    let stopped = harness::stop_run(
        &project,
        &["run", "flow.work", "--input", "goal=start something long"],
        &environment,
        libc::SIGINT,
        || {
            if !started.exists() {
                return false;
            }
            if marked.get().is_none() {
                marked.set(Some(std::time::Instant::now()));
            }
            true
        },
    );

    let status = stopped.status.expect(
        "the run never ended: the sweep installed a handler and did not hand the signal back",
    );
    assert!(
        !status.success(),
        "a run stopped by a signal does not report success: {status:?}"
    );
    assert!(
        stopped.took < Duration::from_secs(10),
        "the run ended when it was asked to rather than whenever it got round to \
         it (took {:?})",
        stopped.took
    );

    // Past the instant the escape was due — the fork, plus its own sleep, plus a
    // margin — rather than a flat wait from here, so what the file system is
    // asked is "did the work outlive the run" and not "did it outlive the run by
    // more than this test happened to wait".
    let due = marked.get().expect("the marker was seen") + Duration::from_secs(23);
    let now = std::time::Instant::now();
    if due > now {
        std::thread::sleep(due - now);
    }
    assert!(
        !escaped.exists(),
        "the command the stopped run was in the middle of wrote `{}` after the \
         run had gone: `Ctrl-C` left the work behind (grammar 5.5, D124)",
        escaped.display()
    );
}

/// A killed command's **escapee** does not keep this runtime alive, and the
/// output it goes on writing is not still being read (grammar 5.5, resolved
/// q54).
///
/// `builtin.bash` settles its call at the deadline rather than at the shell's
/// `close`, because a process the group kill cannot reach can still be holding
/// the output pipes. Settling is only half of ending the call: the pipes and
/// their `data` handlers are still this runtime's, and a session that has been
/// killed must not keep them. Kept, they are an event-loop handle that holds the
/// process open for as long as that process lives and a buffer that goes on
/// growing for a result nobody will ever read — in `serve`, or any host embedding
/// a compiled graph, unbounded memory minutes after the command was cut short.
///
/// `set -m` is what makes this a test rather than a tautology. The deadline kills
/// the command's process group (Decision D124, asserted above), which is where an
/// ordinary background job lives — so a plain `&` would now be swept and the
/// pipes would close on their own, proving nothing about the runtime. Job control
/// puts the job in a **group of its own**, which is the portable spelling of the
/// escape `setsid` and a double-forking daemon reach the same way, and no
/// external program is needed to write it.
///
/// The two are one defect and this is the observable half: the command leaves a
/// process holding the pipes for a minute, the deadline answers the model at one
/// second, the run finishes — and the **process** is asked when it is done.
#[test]
fn a_killed_commands_grandchild_does_not_hold_the_runtime_open() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, environment) = bounded_root(&provider, "builtins-outlived");

    // The escapee writes once and then holds both pipes open for a minute, long
    // after the shell above it is killed at one second.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new(
                "bash",
                json!({ "command": "set -m; { printf 'still here'; sleep 60; } & wait" }),
            )]),
        ),
        Script::new(SONNET, Outcome::text("Cut short.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "summary": "the command was cut short" })),
        ),
    ]);

    let started = std::time::Instant::now();
    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.deadline",
        &json!({ "goal": "leave something behind" }),
        &environment,
    ) else {
        return;
    };
    let elapsed = started.elapsed();

    run.succeeded();
    let calls = tool_calls_of(&run, "do");
    assert_eq!(calls[0]["program"]["timedOut"], json!(true), "{calls:?}");

    // …and the process was done when the graph was. Everything here — the build,
    // the run, the deadline — is seconds; the grandchild is a minute. A runtime
    // still holding its pipes would be sitting in the event loop for the rest of
    // that minute with the flow long since finished.
    assert!(
        elapsed < Duration::from_secs(45),
        "the run exited when the graph finished rather than when the grandchild \
         let go of the pipes (took {elapsed:?})"
    );
}

/// A `workspace:` that resolves to **nothing** fails the call, rather than
/// quietly bounding the tool to whatever directory the process was started in
/// (grammar 6.1, Decision D135).
///
/// The parser refuses an empty `workspace:` as written, so the only way to reach
/// this is the environment: `${BUILTIN_ROOT}` is present — the presence check of
/// PRD 5.9 is satisfied — and empty. `path.resolve("")` is the process's working
/// directory, so a runtime that skipped this check would answer the call from
/// wherever it happened to be started, which is the ambient capability every
/// bound in q54 is written out to refuse. The file the model asks for is one that
/// really exists beside the running test, so a runtime with the hole
/// **succeeds** at reading it and this test fails on the missing failure.
#[test]
fn a_workspace_that_resolves_to_nothing_fails_the_call() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_scratch, mut environment) = bounded_root(&provider, "builtins-empty-root");
    environment.push(("BUILTIN_ROOT".to_string(), String::new()));

    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "bash",
            json!({ "command": "cat Cargo.toml" }),
        )]),
    ));

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "read whatever is around" }),
        &environment,
    ) else {
        return;
    };
    let said = run.failed();
    assert!(
        said.contains("resolved to nothing") && said.contains("${BUILTIN_ROOT}"),
        "the failure names the bound that came back empty: {said}"
    );
    assert!(
        !said.contains("exit_code"),
        "the call failed on the workspace rather than reaching the shell at all: {said}"
    );
}

/// A `workspace:` naming a directory that does not exist fails the call, as
/// itself (grammar 6.1).
///
/// The workspace is resolved once per call precisely so that this is reported as
/// the bound being wrong rather than as every path inside it failing to resolve,
/// and it is an *execution* failure, so `retry:`/`on_error:` decide the run
/// (Decision D119): a model cannot make a directory exist by calling differently.
/// Nothing else in the suite holds that message, and the degradation it guards
/// against is quiet: a `workspace:` pointing at a **file**, or at a path with a
/// typo, would otherwise surface as a confusing per-path error from inside the
/// tool.
#[test]
fn a_workspace_that_names_no_directory_fails_the_call() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, mut environment) = bounded_root(&provider, "builtins-absent-root");
    let absent = scratch.path().join("not-a-directory");
    environment.push(("BUILTIN_ROOT".to_string(), absent.display().to_string()));

    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "bash",
            json!({ "command": "cat notes.txt" }),
        )]),
    ));

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.tolerant",
        &json!({ "goal": "read a note" }),
        &environment,
    ) else {
        return;
    };
    // The run **completes**, which is only possible because the node failed and
    // `on_error: skip` absorbed it: the value in `summary` is the one the node
    // after it wrote, not the agent's. That is the half of the split a built-in
    // still has — the *tool* being unusable is the world's, not the model's.
    run.succeeded();
    assert_eq!(run.outputs()["summary"], "the node was skipped");
    let entries = run.entries("attempt");
    assert_eq!(entries.len(), 1, "{entries:?}");
    assert_eq!(entries[0]["outcome"], "skipped");
    let said = entries[0]["error"].as_str().unwrap_or_default();
    assert!(
        said.contains("is not a directory that exists") && said.contains("${BUILTIN_ROOT}"),
        "the failure is the workspace's, named as the workspace: {said}"
    );
    let calls = tool_calls_of(&run, "attempt");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0]["outcome"], "failed");
    assert_eq!(
        calls[0]["program"],
        json!({ "tool": "bash", "command": "cat notes.txt" }),
        "a call that failed still records what the model asked it to run — which is the \
         record a reader opens the trace for (`docs/trace.md` §7.4): {calls:?}"
    );
}

/// A host with **no `bash` on `PATH`** fails the call with an error naming the
/// requirement (PRD resolved q54, grammar 5.5).
///
/// q54 keeps q31's posture: the shell is resolved from `PATH` at the call, and a
/// host with no bash fails the call as an execution failure with an error naming
/// the requirement — the compiler does not decide at build time what a deployment
/// machine has. So the `PATH` the graph runs under is the subject here, and it
/// carries the JavaScript runtime and nothing else: an empty one would decide the
/// test before the graph started.
#[cfg(unix)]
#[test]
fn a_host_with_no_bash_on_path_fails_the_call_naming_the_requirement() {
    let provider = MockProvider::start().expect("a loopback port");
    let (scratch, mut environment) = bounded_root(&provider, "builtins-no-bash");
    let Some(path) = a_path_without_bash(&scratch) else {
        return;
    };
    environment.push(("PATH".to_string(), path));

    provider.enqueue(Script::new(
        SONNET,
        Outcome::tool_calls(vec![ToolCall::new(
            "bash",
            json!({ "command": "printf 'ran'" }),
        )]),
    ));

    let Some(run) = harness::invoke_with(
        "builtin-tools",
        "flow.work",
        &json!({ "goal": "run something" }),
        &environment,
    ) else {
        return;
    };
    let said = run.failed();
    assert!(
        said.contains("this host has no `bash` on `PATH`"),
        "a missing shell is reported as the requirement it is, not as an errno: {said}"
    );
    let calls = tool_calls_of(&run, "do");
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert_eq!(calls[0]["outcome"], "failed");
}

/// A `PATH` holding the JavaScript runtime and no `bash`, or `None` when the
/// toolchain is absent and the caller has nothing to run.
///
/// The harness starts an emitted project with `bun`, which it may resolve from
/// `PATH` by name — so the directory is built rather than emptied: one link to
/// the runtime, and nothing else on it.
#[cfg(unix)]
fn a_path_without_bash(scratch: &harness::Scratch) -> Option<String> {
    let runtime = harness::bun_command()?;
    let program = std::path::PathBuf::from(runtime.get_program());
    let executable = if program.is_absolute() {
        program
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|directory| directory.join(&program))
            .find(|candidate| candidate.is_file())
            .expect("the runtime was found on `PATH`, so it is on it")
    };
    let bin = scratch.path().join("bin");
    std::fs::create_dir_all(&bin).expect("the scratch area is writable");
    std::os::unix::fs::symlink(executable, bin.join("bun")).expect("the scratch area is writable");
    assert!(
        !bin.join("bash").exists(),
        "the point of this directory is the shell it does not hold"
    );
    Some(bin.display().to_string())
}

/// Every `ToolCallRecord` one agent node's model calls filed, in call order
/// (`docs/trace.md` §7.3).
fn tool_calls_of(run: &harness::Invocation, node: &str) -> Vec<Value> {
    run.entries(node)
        .iter()
        .flat_map(|entry| {
            entry["models"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .flat_map(|call| {
            call["toolCalls"]
                .as_array()
                .cloned()
                .unwrap_or_default()
                .into_iter()
        })
        .collect()
}

// ---------------------------------------------------------------------------
// PRD §7 M3, first bullet — "Durable execution" (resolved q26–q29). Live.
// ---------------------------------------------------------------------------

/// The core claim of resolved q29: **replay is read-only up to the frontier.**
///
/// `flow.relay` makes two model calls in a row. The run is killed while the
/// second is in flight, so the journal holds exactly one recorded answer — and
/// a resumed generation has to consume that answer rather than ask for it
/// again. The mock provider's own request log is what decides it: its queue and
/// its record are cleared before the resume, and only the **second** call is
/// scripted. A resumed run that re-issued the first would find nothing scripted
/// for it and fail; one that issued it and was answered would show two requests
/// here instead of one.
///
/// The kill is a real `SIGKILL` to the process running the graph, with nothing
/// flushed that had not already been written (see `harness::crash_run`) — which
/// is the event durability exists for.
#[test]
fn a_resumed_run_consumes_its_recorded_model_answers_instead_of_asking_again() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the first note" })),
    ));
    // Held long enough that the kill lands while this call is outstanding: the
    // frontier is then exactly one effect in, which is the shape being tested.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-model")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    // Everything the crashed generation asked for is behind it. What is scripted
    // from here is the one call a correct replay still owes the provider.
    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(
        answered["execution_id"], killed.execution,
        "a resumed generation is the same execution, not a new one: {answered}"
    );
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["relayed"], "the first note",
        "the replayed call's answer reaches the resumed run's state: {answered}"
    );
    assert_eq!(
        answered["outputs"]["final"], "the second note",
        "{answered}"
    );

    let asked = provider.requests();
    assert_eq!(
        asked.len(),
        1,
        "a replay is read-only up to the frontier (PRD resolved q29): the recorded \
         call must not reach the provider a second time, and {} did",
        asked.len()
    );
    // …and the one call it did make is the *second* node's, whose input is what
    // the replayed answer wrote. So the replay did not merely skip a request —
    // it handed the graph the value the first generation got.
    assert!(
        asked[0].body_text.contains("the first note"),
        "the live call is the second node's, asked about what the replayed answer \
         wrote: {}",
        asked[0].body_text
    );
}

/// The same claim over a composition whose provider runs **server tools**:
/// replay is untouched, because the use happened inside the recorded model call
/// (Decision D122, `docs/durability.md` §3.1).
///
/// This is the one durability question q30 raises and answers: a server tool is
/// not an effect of its own, so it gets no journal record and no key — what is
/// recorded is the model call it happened inside, whose answer already carries
/// the `server_tool_use` block and the result the provider paired with it. A
/// resumed generation therefore replays the whole turn, search and all, and the
/// provider is asked once for the one call still ahead of the frontier.
///
/// It also exercises the half that *did* move: the recorded call's **request
/// identity** now carries the provider's suite, so this is a resume where the
/// suite matched. The other side — a suite that changed between generations —
/// is a divergence by construction, and `docs/durability.md` §7 says so.
#[test]
fn a_resumed_run_replays_a_server_tools_answer_without_asking_again() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the first finding" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search_20250305",
                json!({ "query": "the question" }),
                json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
            ),
        ]),
    ));
    // Held past the kill, so the frontier lands exactly one effect in.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the second finding" }))
            .after(Duration::from_secs(120)),
    ));

    let Some((project, built)) =
        harness::build_under_toolchain("server-tools", "resume-server-tools")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the server-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "question=does it?"],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the second finding" })),
    ));

    let resumed = harness::resume(
        &project,
        "server-tools",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["relayed"], "the first finding",
        "the replayed call's answer — the one the search was woven into — reaches \
         the resumed run's state: {answered}"
    );
    assert_eq!(
        answered["outputs"]["final"], "the second finding",
        "{answered}"
    );

    let asked = provider.requests();
    assert_eq!(
        asked.len(),
        1,
        "the recorded call is replayed rather than re-issued, server tool and all"
    );
    assert_eq!(
        asked[0].server_tools,
        ["web_search_20250305"],
        "and the live call past the frontier still carries the provider's suite"
    );
    assert!(
        asked[0].body_text.contains("the first finding"),
        "the live call is the second node's, asked about what the replayed answer \
         wrote: {}",
        asked[0].body_text
    );
}

/// …and the same claim on the **Responses** wire, which is the one the
/// durability argument was otherwise only made about (Decision D122).
///
/// The wire matters to replay for a reason the Messages one does not raise:
/// this surface answers with a list of items carrying service-minted ids — the
/// `web_search_call`, the `message`, the `function_call` — and the runtime
/// replays that list into the next request verbatim. Those ids are therefore
/// inside the next call's **request identity** (`docs/durability.md` §11.1), so
/// a resumed generation only derives the same identity if it went on with the
/// recorded answer rather than a re-read of it. That is exactly what
/// [`callModel`]'s "the kept answer, not the live one" rule buys, and it is
/// unfalsifiable without a run: the composition is two calls in a row on this
/// wire, killed between them.
///
/// Nothing Responses-specific is *added* to the identity beyond what the answer
/// carries — no `previous_response_id`, no continuity token — which is why the
/// journal's version does not move for this wire either.
#[test]
fn a_resumed_run_replays_a_responses_wire_answer_without_asking_again() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        GPT5,
        Outcome::structured(json!({ "answer": "the first finding" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search",
                json!({ "type": "search", "query": "the question" }),
                json!([{ "url": "https://docs.example.com/a" }]),
            ),
        ]),
    ));
    // Held past the kill, so the frontier lands exactly one effect in.
    provider.enqueue(Script::new(
        GPT5,
        Outcome::structured(json!({ "answer": "the second finding" }))
            .after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("server-tools", "resume-responses")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the server-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &[
            "run",
            "flow.relay_responses",
            "--input",
            "question=does it?",
        ],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    provider.reset();
    provider.enqueue(Script::new(
        GPT5,
        Outcome::structured(json!({ "answer": "the second finding" })),
    ));

    let resumed = harness::resume(
        &project,
        "server-tools",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["relayed"], "the first finding",
        "the replayed Responses answer reaches the resumed run's state: {answered}"
    );
    assert_eq!(
        answered["outputs"]["final"], "the second finding",
        "{answered}"
    );

    let asked = provider.requests();
    assert_eq!(
        asked.len(),
        1,
        "the recorded call is replayed rather than re-issued, on this wire too"
    );
    assert_eq!(
        (asked[0].surface, asked[0].path.as_str()),
        (Surface::Responses, "/v1/responses"),
        "…and the one live call is still this connection's wire"
    );
    assert_eq!(
        asked[0].server_tools,
        ["web_search"],
        "…still carrying the provider's suite"
    );
    assert!(
        asked[0].body_text.contains("the first finding"),
        "the live call is the second node's, asked about what the replayed answer \
         wrote: {}",
        asked[0].body_text
    );
}

/// The other side of the two above, and the one `docs/durability.md` §7 promises
/// outright: **a resume whose provider lost a server tool diverges at the first
/// model call.**
///
/// The suite is in the request identity because it is part of what the model was
/// offered (Decision D122), and the consequence is the one resolved q29 intends:
/// the recorded answer was produced by a model with a different tool surface, so
/// handing it to a graph that would now ask differently is the silent re-keying
/// q29 refuses. The resume fails, naming the step.
///
/// Both halves of the conditionality that makes that true are decided here, and
/// only a run can decide either:
///
/// * the key is **present** when a member declares a suite — asserted off the
///   journal's own bytes before the composition moves, since a `serverTools`
///   that never reached the identity would make the divergence below happen for
///   some other reason and this test pass on a build that had stopped keying on
///   the suite at all;
/// * and the identity really **turns on it** — the composition that moves under
///   the journal moves in exactly one respect, the `server_tools:` array of the
///   provider `flow.relay`'s two calls resolve to. Every other input to the
///   identity (the `model.*` addressed, the system prompt, the conversation, the
///   tool names, the pinned tool) is byte-for-byte what the killed generation
///   recorded.
///
/// A regression that stopped emitting the key would pass every other test in
/// this file — the two above included, since a suite that matches replays either
/// way — and would replay an answer produced under a tool surface this build no
/// longer offers.
#[test]
fn a_resume_whose_provider_lost_its_server_tools_diverges_at_the_first_model_call() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the first finding" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search_20250305",
                json!({ "query": "the question" }),
                json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
            ),
        ]),
    ));
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "answer": "the second finding" }))
            .after(Duration::from_secs(120)),
    ));

    let Some((project, built)) =
        harness::build_under_toolchain("server-tools", "resume-suite-moved")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the server-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "question=does it?"],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    // The first half: the recorded identity carries the suite. Read off the
    // journal's committed bytes, the way this harness reads every key.
    assert!(
        harness::journal_holds(&project, &["first/0#model/0"]),
        "the killed generation committed the first node's model record"
    );
    assert!(
        harness::journal_holds(&project, &["serverTools"]),
        "a connection that declares a suite puts it in the request identity \
         (`docs/durability.md` §7)"
    );

    // The composition moves, in the one respect this test is about: the
    // provider behind `flow.relay` stops declaring a suite.
    let moved = harness::Scratch::new("suite-moved");
    let source = std::fs::read_to_string(harness::fixture("server-tools")).expect("the fixture");
    let edited = source.replace(
        "  base_url: ${MOCK_BASE_URL}\n  server_tools:\n    - type: web_search_20250305\n      \
         name: web_search\n      max_uses: 5\n      allowed_domains: [\"docs.example.com\"]\n",
        "  base_url: ${MOCK_BASE_URL}\n",
    );
    assert_ne!(
        edited, source,
        "the Messages provider's suite is the block being removed"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a resume of an answer produced under another tool surface is a divergence: {said}"
    );
    assert!(
        said.contains("first/0#model/0"),
        "…naming the step that disagreed, which is the *first* model call: {said}"
    );

    assert_eq!(
        provider.snapshot().requests,
        0,
        "a divergence re-executes nothing (PRD resolved q29)"
    );
}

/// The arm the conditionality above buys, and the reason `JOURNAL_VERSION` did
/// not move for any of it: a composition that declares **no** suite derives the
/// identity it already derived.
///
/// `callModel` omits the `serverTools` key entirely where no ladder member
/// declares one, rather than writing an empty array — and the difference is
/// every journal written before the key existed. `canonical()` sorts keys and
/// hashes the whole object, so an extra `"serverTools":[[]]` would be a
/// different string and every pre-existing record would fail to match its own
/// replay (`docs/durability.md` §11.2, §11.3).
///
/// This holds it off the journal's committed bytes, on the fixture whose
/// providers declare nothing: the run above is the same shape and the same
/// two-call frontier, so what differs between the two assertions is only the
/// suite.
#[test]
fn a_composition_with_no_server_tools_keeps_the_identity_it_always_derived() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the first note" })),
    ));
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "identity-no-suite")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let ran = harness::run_into(
        &project,
        "durability",
        "flow.relay",
        &[("topic", "durability")],
        None,
        &environment,
    );
    ran.succeeded();

    assert!(
        harness::journal_holds(&project, &["first/0#model/0", "second/0#model/0"]),
        "both model records are committed, so the journal really is being read"
    );
    assert!(
        !harness::journal_holds(&project, &["serverTools"]),
        "no ladder member declares a suite, so the key is omitted from the request \
         identity rather than written empty — which is what keeps a journal from \
         before the key existed replaying, and `JOURNAL_VERSION` where it is"
    );
}

/// The **second** key `JOURNAL_VERSION` did not move for, on the same terms as
/// the one above: a conversation turn off the **Messages** wire is written
/// untagged, so a tool loop on that wire derives the identity it always derived.
///
/// `docs/durability.md` §3.1: an assistant turn records which wire wrote its
/// blocks, and the key "is written **only** for a Responses turn: the Messages
/// wire was the only surface that ever produced blocks, so leaving its turns
/// untagged keeps every identity an earlier build derived". One line of the
/// emitted runtime makes that true — `replayed`'s `...(answer.wire === undefined
/// ? {} : { wire: answer.wire })`, resting on `callMessages` returning no `wire`
/// field — and losing it is invisible to every other test in this file. A
/// `wire: "messages"` added to `callMessages` for symmetry would stamp the key
/// onto every replayed Messages turn; `turns` is part of a model call's request
/// identity, so every second-and-later call of every Messages tool loop would
/// re-key, and `canonical()` hashes the whole object. Each resume test here
/// resumes a journal its own build wrote, so both sides would carry the new key
/// and agree — while an `agent-compose resume` of an execution begun under an
/// earlier build would die with a `ReplayDivergence` at the loop's second model
/// call, with no `JOURNAL_VERSION` bump to signal a migration.
///
/// Both halves of the conditionality are decided, for the reason the pair above
/// decides both of its: a negative assertion on its own would also pass on a
/// build whose identity had stopped carrying the conversation at all, or whose
/// journal had stopped holding a request as text this scan can read.
///
/// * `flow.search_loop` is the Messages wire with a real loop — three model
///   calls, the second and third of which replay an assistant turn — and its
///   journal must hold no `wire` key anywhere;
/// * `flow.respond` is the same shape on the wire that **does** tag its turns,
///   and its journal must hold one, which is what makes the scan above a scan
///   that can see the key at all.
#[test]
fn a_messages_wire_tool_loop_keeps_the_untagged_turns_it_always_derived() {
    let provider = MockProvider::start().expect("a loopback port");
    // The Messages loop: a server search and a client call in one turn, prose to
    // end the loop, then the pinned answer.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search_20250305",
                    json!({ "query": "what" }),
                    json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
                )]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "answer": "a looked-up snippet" })),
        ),
    ]);

    let Some((messages, built)) =
        harness::build_under_toolchain("server-tools", "identity-messages-untagged")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the server-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    harness::run_into(
        &messages,
        "server-tools",
        "flow.search_loop",
        &[("question", "what?")],
        None,
        &environment,
    )
    .succeeded();

    assert!(
        harness::journal_holds(&messages, &["ask/0#model/0", "ask/0#model/2"]),
        "the loop's model records are committed, so the journal really is being read"
    );
    assert!(
        !harness::journal_holds(&messages, &["\"wire\""]),
        "a Messages turn is replayed untagged, so the key reaches neither the \
         recorded answer nor the request identity — which is what keeps a journal \
         from before the second wire existed replaying, and `JOURNAL_VERSION` \
         where it is (`docs/durability.md` §3.1, §11.2)"
    );
    assert!(provider.snapshot().is_drained());

    // The positive half, on the wire whose turns *are* tagged: the same loop
    // shape, three calls, replaying assistant turns the Responses wire wrote.
    provider.reset();
    provider.enqueue_all([
        Script::new(
            GPT5,
            Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "what" }))])
                .with_server_tools(vec![ServerToolUse::new(
                    "web_search",
                    json!({ "type": "search", "query": "what" }),
                    json!([{ "url": "https://docs.example.com/a" }]),
                )]),
        ),
        Script::new(GPT5, Outcome::text("I have what I need.")),
        Script::new(
            GPT5,
            Outcome::structured(json!({ "answer": "a looked-up snippet" })),
        ),
    ]);

    let Some((responses, built)) =
        harness::build_under_toolchain("server-tools", "identity-responses-tagged")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the server-tools fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    harness::run_into(
        &responses,
        "server-tools",
        "flow.respond",
        &[("question", "what?")],
        None,
        &environment,
    )
    .succeeded();

    assert!(
        harness::journal_holds(&responses, &["ask/0#model/0", "ask/0#model/2", "\"wire\""]),
        "the same three-call loop on the Responses wire commits the key, so the \
         assertion above is an absence this scan can tell from a presence"
    );
    assert!(provider.snapshot().is_drained());
}

/// A pause survives the process that opened it, and comes back under the **same
/// wait id** (resolved q28).
///
/// The run is killed while it is holding the question — the terminal has been
/// shown the prompt, and standard input is held open so the pause is really
/// parked rather than withdrawn. Nothing about the wait is journaled, because
/// nobody answered it; what is journaled is the model call before it. So a
/// resumed generation replays that call, re-parks, and asks the same question —
/// and the id it asks under is the node's instance path (grammar 9.4), which no
/// process generation is part of.
#[test]
fn a_pause_killed_with_its_process_is_asked_again_under_the_same_wait_id() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-pause")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let mut environment = harness::environment(&provider);
    environment.push((harness::INTERACTIVE.to_string(), "1".to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.gate", "--input", "topic=durability"],
        &environment,
        |said| said.iter().any(|line| line.contains("pause `sign_off/0`")),
    );
    assert!(
        killed
            .stderr
            .contains("answer `sign_off/0` with one line of JSON"),
        "the first generation asked the question: {}",
        killed.stderr
    );

    // The model answered once and its answer is in the journal; the wait it
    // opened is in no journal at all, because nobody settled it.
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the first generation made its one model call"
    );

    let resumed = harness::resume_answering(
        &project,
        "durability",
        &killed.execution,
        &environment,
        &["{\"decision\": \"approve\"}"],
        harness::Answers::Closed,
    );
    resumed.succeeded();
    assert_eq!(
        resumed.outputs(),
        json!({ "note": "worth signing off", "decision": "approve" }),
        "{}",
        resumed.stderr()
    );
    assert!(
        resumed
            .stderr()
            .contains("answer `sign_off/0` with one line of JSON"),
        "the resumed generation asks the same pause under the same id: {}",
        resumed.stderr()
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the model call ahead of the pause was replayed, not re-issued"
    );
}

/// A lock the killed writer never gave back does not take the journal with it
/// (`docs/durability.md` §2).
///
/// The driver takes SQLite's exclusive lock by creating `<file>.lock` as a
/// directory and gives it back by removing it, so a process that dies **inside**
/// a write leaves one nothing else will ever remove. Left standing it refuses
/// every later open of that journal — not only the resume of the execution the
/// crash interrupted, but every future run of the project, for ever. A
/// durability story with a file a crash can permanently seal is not one.
///
/// The stale lock is planted rather than raced for, which is the only way to
/// decide this: the window a real crash has to land in is one statement wide, so
/// a test that tried to hit it would be a test that usually did not.
#[test]
fn a_lock_a_killed_writer_left_behind_does_not_seal_the_journal() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "the first note" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "the second note" })),
        ),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "stale-lock") else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let environment = harness::environment(&provider);
    let lock = project.join(".agent-compose").join("journal.sqlite.lock");
    std::fs::create_dir_all(&lock).expect("the project's data directory is writable");

    let ran = harness::run_into(
        &project,
        "durability",
        "flow.relay",
        &[("topic", "durability")],
        None,
        &environment,
    );
    ran.succeeded();
    assert!(
        !lock.exists(),
        "the lock its owner never gave back is gone: {}",
        lock.display()
    );
    assert_eq!(
        provider.snapshot().requests,
        2,
        "…and the run it was blocking did everything it was going to do"
    );
}

/// A pause somebody **answered** is not asked again, and the entry it replays as
/// is dated by the generation that held it.
///
/// The other half of the test above, and the promise durability makes that a
/// person can see: `flow.gate` cannot decide it, because nothing follows its
/// pause — the answer and the end of the run are the same moment there, so the
/// journalled answer is never read back. `flow.signed` puts a node after the
/// wait, the run is answered and then killed while *that* node's model call is
/// in flight, and the resumed generation has to hand the graph what the person
/// said.
///
/// The resume is **not** interactive, which is the sharpest form of the
/// assertion available: a resumed execution that re-parked would have no answer
/// surface at all and would end `3` holding the question (grammar 8.7).
///
/// It also decides the entry's two instants. A replayed wait takes `settledAt`
/// from the record; if it took `pausedAt` from *this* process's clock, the trace
/// of a resumed execution would say the answer arrived before the question — a
/// negative duration for any reader that subtracts them, and the opposite of
/// what `docs/durability.md` §9 promises.
#[test]
fn an_answered_pause_is_replayed_rather_than_put_to_the_person_twice() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));
    // Held, so the kill lands while the node **after** the pause is calling: the
    // answer is journaled and the run has not finished with it.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "signed and relayed" }))
            .after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-answered")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let mut answering = harness::environment(&provider);
    answering.push((harness::INTERACTIVE.to_string(), "1".to_string()));

    let killed = harness::crash_run_answering(
        &project,
        &["run", "flow.signed", "--input", "topic=durability"],
        &answering,
        &["{\"decision\": \"approve\"}"],
        |_| provider.snapshot().requests >= 2,
    );
    assert!(
        killed
            .stderr
            .contains("answer `sign_off/0` with one line of JSON"),
        "the first generation asked the question: {}",
        killed.stderr
    );

    // Everything ahead of the frontier is behind the crash: the model call, and
    // the answer a person gave.
    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "signed and relayed" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &harness::environment(&provider),
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["decision"], "approve",
        "the resumed run goes on with what the person answered, not with the \
         channel's default: {answered}"
    );
    assert_eq!(
        answered["outputs"]["final"], "signed and relayed",
        "…and reached the node past the pause: {answered}"
    );
    assert!(
        !resumed.stderr().contains("answer `sign_off/0`"),
        "a resumed execution does not put an answered question to a person a \
         second time: {}",
        resumed.stderr()
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "…and asked the provider only for the call the crash interrupted"
    );

    // The trace `--format json` carries, which is the resumed generation's own
    // whole document (`docs/durability.md` §9).
    let entries: Vec<&Value> = answered["trace"]
        .as_array()
        .unwrap_or_else(|| panic!("the report carries the run's trace: {answered}"))
        .iter()
        .filter(|entry| entry["node"] == "sign_off")
        .collect();
    let [signed] = entries.as_slice() else {
        panic!("the resumed generation filed one entry for the pause: {entries:?}");
    };
    let pause = &signed["human"];
    assert_eq!(pause["settled"], "resumed", "{signed}");
    let paused = pause["pausedAt"].as_str().expect("when the wait began");
    let settled = pause["settledAt"].as_str().expect("when it was answered");
    assert!(
        paused <= settled,
        "a replayed wait is dated by the generation that held it: this one was \
         answered at {settled} and says it began at {paused}"
    );
}

/// A contract that moved on the **first** of two tools at one effect site is a
/// divergence, not an ordinary mismatch a `retry:` absorbs.
///
/// `docs/durability.md` §7 lets one recorded mismatch through: the one the
/// recording generation's own ladder already retried past. What tells the two
/// apart is the next record at the site — and *next* has to mean "the same
/// request again", not merely "the next one". `agent.reading` calls `tool.alpha`
/// and then `tool.beta` in one loop, so the site holds `ask/0#tool/0` and
/// `ask/0#tool/1`; tightening only alpha's `output:` puts beta's record exactly
/// where a second attempt of alpha would have been.
///
/// The composition's own `retry: { max: 2 }` on the node is what makes getting
/// this wrong catastrophic rather than merely wrong: an ordinary `ResultMismatch`
/// is absorbed, and the second attempt claims ordinals past the frontier — a live
/// provider call and a live re-run of **both** subprocesses, which is the double
/// side effect resolved q29 exists to prevent.
#[test]
fn a_contract_that_moved_on_one_of_two_tools_at_a_site_is_not_read_as_a_retry() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // One answer, two calls: two `tool` effects at one site.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![
                ToolCall::new("alpha", json!({ "line": "the first source" })),
                ToolCall::new("beta", json!({ "line": "the second source" })),
            ]),
        ),
        // The loop offers the agent's tools while the model is calling them and
        // asks for the declared `output:` once it stops, so the turn after the
        // two results is a plain answer and the structured one is its own call.
        Script::new(SONNET, Outcome::text("both sources agree")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "both sources agree" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "relayed" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-two-tools")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let shims = harness::Scratch::new("two-tools");
    let log = shims.path().join("tally.log");
    harness::shim(
        shims.path(),
        "alpha",
        "printf 'alpha\\n' >> \"$TALLY_LOG\"\nprintf 'alpha said so'\n",
    );
    harness::shim(
        shims.path(),
        "beta",
        "printf 'beta\\n' >> \"$TALLY_LOG\"\nprintf 'beta said so'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.sourced", "--input", "topic=durability"],
        &environment,
        |_| {
            // `relay`'s call is the one in flight, and both of `ask`'s tool
            // effects are in the record rather than merely in the shim's log.
            provider.snapshot().requests >= 4
                && harness::journal_holds(&project, &["ask/0#tool/0", "ask/0#tool/1"])
        },
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the first generation read both sources once: {}",
        killed.stderr
    );

    // Exactly one line moves, and it is the **first** tool's contract. The
    // binding itself is untouched, so the request identity still matches and the
    // divergence is the one decided at the parse (`docs/durability.md` §7).
    let moved = harness::Scratch::new("two-tools-contract");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "    first: { type: string }",
        "    first: { type: string, min_length: 500 }",
    );
    assert_ne!(
        edited, source,
        "`tool.alpha`'s output is what this copy moves"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a recorded answer this build no longer accepts is a divergence whatever \
         else the site went on to do: {said}"
    );
    assert!(
        said.contains("`ask/0#tool/0`"),
        "…named at the effect whose contract moved, not at the one after it: {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "a divergence re-executes nothing: an absorbed mismatch would have spent \
         the node's second attempt on a live model call"
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "…and neither source was read a second time"
    );
}

/// The same site, the same tightened contract, and the next record repeating
/// this one's **request**: still a divergence, because a repeated call is not a
/// retried one (`docs/durability.md` §7).
///
/// A recorded answer this build refuses is not a divergence when the generation
/// that recorded it refused it too — its own ladder decided that, and a resume
/// has to do what it did. What tells the two apart cannot be read off the
/// records *around* the answer: the ordinals hold the sequence of effects at a
/// site, and "the site retried this call" and "the site made this call again"
/// are the same sequence. Here `agent.reading`'s loop calls `tool.alpha` twice
/// with identical arguments — the ordinary shape of a model looking something up
/// again — so `ask/0#tool/1` is byte-for-byte the request `ask/0#tool/0` was,
/// which is exactly what an inference would read as "the ladder already went
/// round".
///
/// Reading it that way is not a smaller mistake than missing the divergence: the
/// node carries `retry: { max: 2 }`, so the ordinary mismatch is absorbed and the
/// second attempt claims ordinals **past the frontier** — a live provider call
/// and a live re-run of the subprocess, which is the double side effect resolved
/// q29 exists to prevent. So the record says which it was, because the generation
/// that refused the answer wrote it down.
#[test]
fn a_contract_that_moved_on_a_repeated_call_at_a_site_is_not_read_as_a_retry() {
    let provider = MockProvider::start().expect("a loopback port");
    let looked_up = json!({ "line": "the first source" });
    provider.enqueue_all([
        // Two turns, one call each, with the **same** arguments: two `tool`
        // effects at one site whose recorded requests are identical.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("alpha", looked_up.clone())]),
        ),
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("alpha", looked_up.clone())]),
        ),
        // The loop stops when the model answers without calling anything, and
        // the declared `output:` is asked for in a call of its own.
        Script::new(SONNET, Outcome::text("the source agrees with itself")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "the source agrees with itself" })),
        ),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "relayed" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-twice-asked")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let shims = harness::Scratch::new("twice-asked");
    let log = shims.path().join("tally.log");
    harness::shim(
        shims.path(),
        "alpha",
        "printf 'alpha\\n' >> \"$TALLY_LOG\"\nprintf 'alpha said so'\n",
    );
    harness::shim(
        shims.path(),
        "beta",
        "printf 'beta\\n' >> \"$TALLY_LOG\"\nprintf 'beta said so'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.sourced", "--input", "topic=durability"],
        &environment,
        |_| {
            provider.snapshot().requests >= 5
                && harness::journal_holds(&project, &["ask/0#tool/0", "ask/0#tool/1"])
        },
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the first generation read the one source twice: {}",
        killed.stderr
    );

    // The same one line as the two-tools case: `tool.alpha`'s contract, and
    // nothing about the binding, so the request identity still matches.
    let moved = harness::Scratch::new("twice-asked-contract");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "    first: { type: string }",
        "    first: { type: string, min_length: 500 }",
    );
    assert_ne!(
        edited, source,
        "`tool.alpha`'s output is what this copy moves"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a record that repeats this one's request is the site calling again, not the \
         ladder going round: {said}"
    );
    assert!(
        said.contains("`ask/0#tool/0`"),
        "…named at the first of the two, which is the one this build refused: {said}"
    );
    assert!(
        said.contains("no longer satisfies this run's contract"),
        "…and said to be the contract divergence rather than a moved request: {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "an absorbed mismatch would have spent the node's second attempt on a live \
         model call"
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "…and re-read the source a third time"
    );
}

/// A store whose data **died with the process** is refused rather than answered
/// out of an empty database (`docs/durability.md` §5).
///
/// `flow.scratched` writes to a `scope: execution` store, calls a model, and then
/// reads the store back. The crash lands in the model call, so the write is in
/// the record and the read is past the frontier — and a replayed write is never
/// applied a second time, so the resumed process holds a scratch database that
/// was created empty a moment ago.
///
/// Nothing about that compares unequal: the read's request identity has not
/// moved and there is no record to compare it against. So without the refusal the
/// resume answers `found: false` about something the execution wrote, routes on
/// it, and reports `completed` — a wrong answer with no diagnostic, which is
/// worse than a failed resume.
#[test]
fn a_store_that_died_with_the_process_refuses_the_resume_it_cannot_answer() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a scratched note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-scratch")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.scratched", "--input", "topic=durability"],
        &environment,
        |_| {
            provider.snapshot().requests >= 1
                && harness::journal_holds(&project, &["stash/0#store/0"])
        },
    );

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a scratched note" })),
    ));
    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a live read of a store the record filled in a process that is gone is \
         not an answer this build can give: {said}"
    );
    assert!(
        said.contains("`recall/0`") && said.contains("`store.scratch`"),
        "…naming the op it stopped at and the store it cannot reconstruct: {said}"
    );
    assert!(
        said.contains("scope: session"),
        "…and what the composition would have to say instead: {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the frontier's own call is live and nothing past it ran"
    );
}

/// `serve` auto-recovers (resolved q28): "on process start it replays every
/// execution the journal holds open, including executions parked on `human`
/// waits, which re-park with their wait ids intact — the wait id is
/// deterministic […] so a resume request arriving after the restart still finds
/// its wait."
///
/// One app starts an execution through its `http` trigger and is killed while
/// the execution is holding a question. A second app is started against the same
/// project — and so the same journal — and the resume URL that was prepared
/// against the dead process answers the wait in the live one.
#[test]
fn a_restarted_serve_recovers_its_open_executions_and_their_waits() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let Some(project) = harness::scratch_project("serve-recovery") else {
        return;
    };
    let environment = harness::environment(&provider);

    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json("/gate", &json!({ "topic": "durability" }))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();

        let status = harness::settled(&app, &execution);
        assert_eq!(status["status"], "interrupted", "{status}");
        let waiting = &status["interrupts"].as_array().expect("the pauses")[0];
        assert_eq!(waiting["wait_id"], "sign_off/0", "{waiting}");
        resume_url = waiting["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
        // Dropping it signals the whole process group, so the app really goes
        // away holding the pause rather than being asked to finish it.
    }

    let Some(second) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");

    // The recovered execution is reported by the same status route, holding the
    // same wait, before anything is asked of it.
    let recovered = harness::settled(&app, &execution);
    assert_eq!(
        recovered["status"], "interrupted",
        "a restarted `serve` brings back the executions the journal holds open: {recovered}"
    );
    let waiting = &recovered["interrupts"].as_array().expect("the pauses")[0];
    assert_eq!(
        waiting["wait_id"], "sign_off/0",
        "…under the wait id the dead process published: {waiting}"
    );
    assert_eq!(
        waiting["input"],
        json!({ "note": "worth signing off" }),
        "…having replayed the model call that decided what the human is shown: {waiting}"
    );

    // …and the URL prepared against the process that died answers in this one.
    let answered = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert!(
        answered.status == 200 || answered.status == 202,
        "{}",
        answered.text()
    );

    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"],
        json!({ "note": "worth signing off", "decision": "approve" }),
        "{finished}"
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "recovery replayed the model call rather than re-issuing it, and re-fired \
         no trigger (resolved q28)"
    );
}

/// A replayed prefix re-issues **nothing**, and the two effect kinds a repeat
/// would be visible in say so.
///
/// `flow.staged` writes to a `scope: global` store, runs a subprocess that
/// counts its own executions, and then calls a model. The run is killed while
/// that call is in flight, so both effects ahead of it are recorded. The
/// resumed generation must:
///
///   * run the subprocess **no** further times — the shim's log is one line;
///   * apply the store write **no** further times — the write's own record says
///     `deduped: false`, which it could not if the write had reached a backend
///     that had already applied its idempotency key (grammar 9.4).
///
/// And once it has completed, the execution is closed: a second `resume` is
/// refused by name rather than re-running anything.
#[test]
fn a_replayed_prefix_re_issues_neither_its_store_write_nor_its_subprocess() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a staged note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-staged")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let shims = harness::Scratch::new("tally");
    let log = shims.path().join("tally.log");
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\nprintf 'tallied'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.staged", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 1,
    );
    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count(),
        1,
        "the first generation ran the subprocess once"
    );

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a staged note" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(answered["outputs"]["tally"], "tallied", "{answered}");
    assert_eq!(answered["outputs"]["note"], "a staged note", "{answered}");

    assert_eq!(
        std::fs::read_to_string(&log)
            .unwrap_or_default()
            .lines()
            .count(),
        1,
        "the replayed subprocess ran no second time (PRD resolved q29)"
    );

    let write = answered["trace"]
        .as_array()
        .expect("a trace")
        .iter()
        .find(|entry| entry["node"] == "save")
        .and_then(|entry| entry["stores"].as_array())
        .and_then(|records| records.first().cloned())
        .unwrap_or_else(|| panic!("the store node's write is on its entry: {answered}"));
    assert_eq!(write["effect"], "write", "{write}");
    assert_eq!(
        write["deduped"], false,
        "a replayed write is not applied a second time, so the backend never saw \
         its key twice: {write}"
    );

    // The execution is closed now, and a second resume says so rather than
    // re-running a graph whose effects have all happened.
    let again = harness::resume(
        &project,
        "durability",
        &killed.execution,
        None,
        &environment,
    );
    let refused = again.failed();
    assert!(
        refused.contains("has already completed"),
        "a completed execution is refused by name: {refused}"
    );
    assert!(
        refused.contains(&killed.execution),
        "…and the refusal names it: {refused}"
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the refused resume ran nothing"
    );
}

/// A journal that no longer describes the composition **fails the resume**,
/// naming the divergent step (resolved q29).
///
/// The way that happens in practice is the composition moving under a journal,
/// so that is what the test does: the same execution is resumed against a copy
/// of its own fixture with the agent's prompt changed. The first model call's
/// recorded request no longer matches the request this run makes, and the
/// resume stops there rather than re-issuing an effect the record claims to
/// hold — with nothing new reaching the provider.
#[test]
fn a_resume_whose_journal_no_longer_describes_the_run_names_the_divergent_step() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the first note" })),
    ));
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-diverged")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    // The composition moves: one line of the agent's prompt, which is part of
    // every request it makes and so part of every recorded request identity.
    let moved = harness::Scratch::new("diverged");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "You write one short note about the topic you are given, and nothing else.",
        "You write one long essay about the topic you are given, and nothing else.",
    );
    assert_ne!(edited, source, "the prompt line is the one being changed");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "the resume names what happened rather than the wrapper it arrived in: {said}"
    );
    assert!(
        said.contains("`first/0`"),
        "…naming the effect site's instance path: {said}"
    );
    assert!(
        said.contains("model effect #0"),
        "…the kind and the ordinal at that site: {said}"
    );
    assert!(
        said.contains("first/0#model/0"),
        "…and the key itself: {said}"
    );

    assert_eq!(
        provider.snapshot().requests,
        0,
        "a divergence re-executes nothing: the run stopped at the step that \
         disagreed (PRD resolved q29)"
    );
}

/// The two refusals a resume owes a caller who has the id wrong, each naming
/// what the reader has to look at rather than what went wrong inside (PRD G3).
///
/// A project that has never run has no journal at all, and saying "unknown
/// execution" there would send a reader to check an id when the answer is that
/// nothing has been journaled yet. Once one has run, an id the journal does not
/// hold is answered with the ones it does — which is also the only surface that
/// publishes them, so the refusal doubles as the listing.
///
/// Both are commands that could not run (exit `2`) rather than runs that
/// produced no answer: nothing was replayed, and the fix is the invocation.
#[test]
fn a_resume_that_cannot_find_its_execution_says_what_the_journal_holds() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-unknown")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );
    let environment = harness::environment(&provider);

    // Nothing has run in this directory, so there is no journal to look in.
    let empty = harness::resume(&project, "durability", "exec_nothing", None, &environment);
    let said = empty.failed();
    assert!(
        said.contains("has never journaled an execution"),
        "a project with no journal says so rather than blaming the id: {said}"
    );
    assert!(
        said.contains("journal.sqlite"),
        "…and names the file it looked for: {said}"
    );

    // One run later there is a journal, and it holds one execution — open,
    // because `flow.gate` parked at a question this run had nobody to answer.
    let mut headless = environment.clone();
    headless.push((harness::INTERACTIVE.to_string(), "0".to_string()));
    let parked = harness::run_into(
        &project,
        "durability",
        "flow.gate",
        &[("topic", "durability")],
        None,
        &headless,
    );
    assert_eq!(
        parked.output.status.code(),
        Some(3),
        "a run with nobody to ask stops at the pause: {}",
        parked.stderr()
    );
    let execution = parked
        .stderr()
        .lines()
        .find_map(|line| line.strip_prefix("execution: "))
        .expect("a run prints its execution id")
        .trim()
        .to_string();

    let unknown = harness::resume(&project, "durability", "exec_nothing", None, &environment);
    let said = unknown.failed();
    assert!(
        said.contains("is not an execution in"),
        "an id the journal does not hold is refused by name: {said}"
    );
    assert!(
        said.contains(&execution),
        "…and the executions it does hold open are named: {said}"
    );
}

/// A replayed value is the value the recording generation went on with
/// (`docs/durability.md` §11.1) — decided where it is observable: inside a
/// request identity built out of it.
///
/// `flow.consulted`'s agent reads `store.ledger` and then answers, so the row
/// the read produced is put into the *next* model call's turns verbatim. The
/// backend builds a `kv` `get` row as `{value, found}` and the journal holds it
/// as `{found, value}`, which is the same value and not the same bytes. A
/// generation that kept the first order and a generation that reads back the
/// second therefore compose two different requests out of one composition — and
/// because the crash lands in the node *after* the agent, every one of the
/// agent's calls is recorded, so the resume compares that request against the
/// record rather than treating it as the frontier.
///
/// The failure this pins is a resume that dies with a `ReplayDivergence` naming
/// `ask/0#model/1` against a composition nobody touched.
#[test]
fn a_replayed_store_read_is_the_row_the_recording_generation_went_on_with() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        // The agent's own loop: read the ledger, stop asking, answer.
        Script::new(
            SONNET,
            Outcome::tool_calls(vec![ToolCall::new("ledger_get", json!({ "key": "entry" }))]),
        ),
        Script::new(SONNET, Outcome::text("I have what I need.")),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "read it back" })),
        ),
        // …and the node the crash lands in.
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "relayed" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-consulted")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.consulted", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 4,
    );

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "relayed" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    let answered = resumed.outputs();
    assert_eq!(
        answered["status"], "completed",
        "the composition never moved, so nothing about this resume is a divergence — \
         a recorded value that reads back in a different order than it was written in \
         is the journal disagreeing with itself: {answered}"
    );
    resumed.succeeded();
    assert_eq!(answered["outputs"]["note"], "read it back", "{answered}");
    assert_eq!(answered["outputs"]["final"], "relayed", "{answered}");
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the three recorded calls were replayed and only the killed one was made again"
    );

    // …and the row really did travel through a request: the replayed store read
    // is what the agent's second call was told, so a replay that answered it
    // from the live store rather than from the record would have had to open one.
    let asked = provider.requests();
    assert!(
        asked[0].body_text.contains("read it back"),
        "the live call is the second node's, asked about what the agent answered: {}",
        asked[0].body_text
    );
}

/// resolved q29's **second** divergence: "a recorded answer [that] fails the
/// current contract".
///
/// `flow.guarded` runs a subprocess whose `output:` the resumed copy of the
/// composition tightens — the binding is untouched, so the recorded *request*
/// still matches and only the recorded *answer* is now refused. Two things must
/// happen and neither is the obvious one: the resume fails naming the step, and
/// the subprocess does **not** run again. The node carries `retry: { max: 2 }`
/// and `on_error: skip`, which are exactly the two policies that would otherwise
/// absorb it — the ladder by claiming an ordinal past the frontier and running
/// the command live, `skip` by carrying the run on past an effect the record
/// claims to hold.
#[test]
fn a_recorded_answer_that_fails_the_current_contract_is_a_divergence_not_a_retry() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a guarded note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-contract")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let shims = harness::Scratch::new("guarded");
    let log = shims.path().join("tally.log");
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\nprintf 'tallied'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.guarded", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 1,
    );
    assert_eq!(
        harness::lines_in(&log),
        1,
        "the first generation ran the subprocess once"
    );

    // The composition keeps the binding and tightens what it will accept back.
    let moved = harness::Scratch::new("contract");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "          stdout: { type: string }",
        "          stdout: { type: string, min_length: 500 }",
    );
    assert_ne!(
        edited, source,
        "the `output:` line is the one being changed"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a recorded answer this composition no longer accepts is a divergence, not a \
         node failure a `retry:` may re-run: {said}"
    );
    assert!(
        said.contains("count/0#tool/0"),
        "…naming the step whose recorded answer it is: {said}"
    );
    assert!(
        said.contains("no longer satisfies this run's contract"),
        "…and saying which of the two divergences it is: {said}"
    );

    assert_eq!(
        harness::lines_in(&log),
        1,
        "the retry ladder did not walk past the frontier and run the command again \
         (PRD resolved q29)"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "`on_error: skip` did not carry the run past the effect either"
    );
}

/// The other side of the same rule: a recorded answer that fails a contract
/// which has **not** moved is not a divergence, because it failed it on the
/// generation that recorded it too.
///
/// `flow.flaky`'s subprocess answers off-contract the first time it is run and
/// on-contract the second, and its `retry:` absorbed that on the generation that
/// crashed — so the journal holds two records at that site. A resumed generation
/// meets the first one, and has to do exactly what the first generation did:
/// spend an attempt and replay the second record. Reading it as a divergence
/// would break a composition nobody touched; reading it as a mismatch and then
/// re-running the command would be the double side effect. The journal is what
/// tells the two apart, and the log is what says which one happened.
#[test]
fn a_recorded_answer_the_original_retried_past_is_retried_past_again() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a flaky note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-flaky")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let shims = harness::Scratch::new("flaky");
    let log = shims.path().join("tally.log");
    // Empty the first time, which the node's `min_length: 3` refuses; the
    // ladder's second attempt is answered.
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\n\
         if [ \"$(wc -l < \"$TALLY_LOG\")\" -le 1 ]; then printf ''; else printf 'tallied'; fi\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &["run", "flow.flaky", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 1,
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the first generation spent an attempt on the off-contract answer"
    );

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "a flaky note" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    let answered = resumed.outputs();
    assert_eq!(
        answered["status"], "completed",
        "a mismatch the recording generation's own ladder absorbed is not a \
         divergence — the composition never moved: {answered}"
    );
    resumed.succeeded();
    assert_eq!(answered["outputs"]["tally"], "tallied", "{answered}");
    assert_eq!(
        harness::lines_in(&log),
        2,
        "…and the attempt it spent was a replay: the command did not run again"
    );
}

/// A divergence **two instance frames down** — inside a subflow a `map`
/// dispatched — arrives whole, keyed by the instance that made it
/// (`docs/durability.md` §4), and is not absorbed by `on_item_error: skip`.
///
/// `flow.parceled` dispatches `flow.child` per item, so the model call that
/// disagrees is at `parcel/0/<index>/write/0` rather than at any node the top
/// level declares. `skip` is the reading that would be worst: the fan-out would
/// record the item as absorbed and the run would carry on and **complete**, so a
/// resume that succeeds here is the whole failure.
#[test]
fn a_divergence_inside_a_dispatched_subflow_is_not_absorbed_by_on_item_error() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "note": "one note" }))),
        Script::new(SONNET, Outcome::structured(json!({ "note": "two note" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "wrapped" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, killed, environment, _shims)) =
        crash_a_parceled_run(&provider, "resume-parceled")
    else {
        return;
    };

    let moved = harness::Scratch::new("parceled-prompt");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, moved_prompt()).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "`on_item_error: skip` absorbs what an item did, not a journal that has \
         stopped describing this run: {said}"
    );
    assert!(
        said.contains("parcel/0/0/write/0"),
        "…keyed by the dispatched instance's own path, two frames down \
         (`docs/durability.md` §4): {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "a divergence re-executes nothing, at any depth"
    );
}

/// The same divergence under the *other* item policy: `on_item_error: retry`.
///
/// The composition the resume is pointed at moves the prompt **and** the policy,
/// which is legitimate — a resume replays a record against whatever composition
/// it is given, and that is what makes a divergence a divergence. What the item
/// retry must not do is retry: each attempt would claim the next ordinal at the
/// item's site, so a ladder of three would walk three recorded effects forward
/// and report a disagreement one step past the one that really happened — and
/// the attempt that ran out of records would issue its model call **live**.
#[test]
fn a_divergence_inside_a_dispatched_item_is_not_retried_by_its_item_policy() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "note": "one note" }))),
        Script::new(SONNET, Outcome::structured(json!({ "note": "two note" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "wrapped" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, killed, environment, _shims)) =
        crash_a_parceled_run(&provider, "resume-parceled-retry")
    else {
        return;
    };

    let moved = harness::Scratch::new("parceled-retry");
    let retried = moved_prompt().replace(
        "        on_item_error: skip",
        "        on_item_error:\n          retry: { max: 2, backoff: 1ms }",
    );
    assert!(
        retried.contains("retry: { max: 2, backoff: 1ms }"),
        "the item policy is the second thing this copy moves"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, retried).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.contains("model effect #0"),
        "the **first** disagreement is what is reported: a retried item would have \
         walked the record forward and named a later one: {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "…and no attempt of that item ran out of records and asked the provider"
    );
}

/// A divergence raised inside a **detached** delivery fails the resume, which is
/// the one nesting depth with nobody to throw to.
///
/// `detach: true` is a `map`-route policy (grammar 8.6 rule 7) and resolved q29
/// admits no policy at any depth, so the fire-and-forget `catch` that keeps a
/// delivery from failing the flow instance may not keep this from failing the
/// **execution**.
///
/// What the moved composition changes is the sink's `expect_exit:`, which is
/// deliberate on a second count: a request identity is the binding **whole**
/// (`docs/durability.md` §3.2), so a widened `expect_exit:` — which changes what
/// an answer means without changing the command that produced it — has to be a
/// divergence. An identity built from the command and its arguments alone would
/// hand this run the answer recorded under the old binding and complete.
#[test]
fn a_divergence_in_a_detached_delivery_fails_the_resume_it_cannot_be_thrown_out_of() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "note": "one note" }))),
        Script::new(SONNET, Outcome::structured(json!({ "note": "two note" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "wrapped" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, killed, environment, shims)) =
        crash_a_parceled_run(&provider, "resume-detached")
    else {
        return;
    };
    let log = shims.path().join("receipt.log");
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the first generation made both deliveries, so both are in the journal"
    );

    let moved = harness::Scratch::new("detached-binding");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace("    expect_exit: [0]", "    expect_exit: [0, 1]");
    assert_ne!(
        edited, source,
        "the sink's `expect_exit:` is what this copy moves"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "wrapped" })),
    ));
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "a delivery nothing waits for is still an effect the record describes: {said}"
    );
    assert!(
        said.contains("receipt/0/"),
        "…named by the dispatch that made it: {said}"
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "…and it delivered nothing: a divergence is raised before the effect"
    );
}

/// A **detached** delivery that called a model is *replayed*, and a resume of
/// the composition that recorded it is not a divergence.
///
/// A delivery runs with the node execution's collectors detached (Decision D94):
/// nothing joins it, so nothing may reach the node's trace entry through it, and
/// `RunContext.modelCalls` is `undefined` for the whole branch. The `ModelCall`
/// records a joined call files on its way to an answer are therefore not filed
/// at all here, and the record's list of them is empty **by construction** — so
/// a replay that read the answering route member off the tail of that list would
/// find nothing there and raise a `ReplayDivergence` about a composition nobody
/// had touched. Which member served the call is recorded in its own right for
/// exactly this shape (`docs/durability.md` §3.1).
///
/// `flow.parceled`'s detached sink is a subprocess, so the whole corpus around
/// it cannot decide this: `flow.fanned`'s is `agent.filing`. And the failure it
/// guards against is unrecoverable rather than merely wrong — a divergence
/// deliberately never closes the execution's row (§7), so every later `resume`,
/// and every `serve` start, would re-derive it for ever.
#[test]
fn a_detached_delivery_that_called_a_model_is_replayed_rather_than_reported_as_divergent() {
    let provider = MockProvider::start().expect("a loopback port");
    // Matched by item text rather than left to arrive in order: a detached
    // delivery and the node after it are dispatched together (rule 7), so which
    // of the three calls reaches the provider first is not this test's to say.
    provider.enqueue_all([
        Script::new(
            SONNET,
            Outcome::structured(json!({ "filed": "noted first" })),
        )
        .matching("alpha-item"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "filed": "noted second" })),
        )
        .matching("beta-item"),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "wrapped" })).after(Duration::from_secs(120)),
        )
        .matching("durability"),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-fanned")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &[
            "run",
            "flow.fanned",
            "--input",
            "topic=durability",
            "--input",
            "topics=[\"alpha-item\",\"beta-item\"]",
        ],
        &environment,
        |_| {
            // Both deliveries are in the **record** — not merely answered by the
            // provider — and `wrap`'s call is the one still in flight.
            provider.snapshot().requests >= 3
                && harness::journal_holds(&project, &["notify/0/0#model/0", "notify/0/1#model/0"])
        },
    );

    // The one call a correct replay still owes the provider, and nothing else.
    provider.reset();
    provider.enqueue(
        Script::new(SONNET, Outcome::structured(json!({ "note": "wrapped" })))
            .matching("durability"),
    );

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said = resumed.stderr();
    assert!(
        !said.contains("ReplayDivergence"),
        "the composition did not move: a delivery whose record holds no filed \
         call is still a delivery this run made:\n{said}"
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}\n{said}");
    assert_eq!(answered["outputs"]["note"], "wrapped", "{answered}");

    let asked = provider.requests();
    assert_eq!(
        asked.len(),
        1,
        "both deliveries were answered out of the journal: only the call the \
         crash interrupted reaches the provider, and {} did",
        asked.len()
    );
    assert!(
        asked[0].body_text.contains("durability"),
        "…and the one live call is `wrap`'s: {}",
        asked[0].body_text
    );
}

/// A diverged resume leaves the execution **open** (`docs/durability.md` §7).
///
/// The composition is what disagreed, so putting it back is the whole repair —
/// and it can only be a repair if the failed resume did not close the row. A
/// `failed` row is refused by name, so recording a divergence as the execution's
/// own outcome would make one moved prompt an unresumable execution for ever,
/// and `serve` — which replays *every* open execution at start — would burn a
/// project's whole backlog on a single deploy.
#[test]
fn a_diverged_resume_leaves_the_execution_open_for_the_composition_that_fits_it() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the first note" })),
    ));
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })).after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-reopened")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let environment = harness::environment(&provider);
    let killed = harness::crash_run(
        &project,
        &["run", "flow.relay", "--input", "topic=durability"],
        &environment,
        |_| provider.snapshot().requests >= 2,
    );

    let moved = harness::Scratch::new("reopened");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "    You write one short note about the topic you are given, and nothing else.",
        "    You write one long essay about the topic you are given, and nothing else.",
    );
    assert_ne!(edited, source, "the prompt line is the one being changed");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &environment,
    )
    .failed();

    // The composition comes back, and so does the execution: the record is
    // untouched and the frontier is where it always was.
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "the second note" })),
    ));
    let again = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    let said = again.stderr();
    assert!(
        !said.contains("has already failed"),
        "a divergence is this build disagreeing with a record, not the execution \
         failing — the row it closed is one nothing can reopen: {said}"
    );
    again.succeeded();
    let answered = again.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["relayed"], "the first note",
        "…and it replayed from the same record: {answered}"
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "the diverged resume issued nothing and the repaired one issued only the \
         call the crash interrupted"
    );
}

/// A run that **parked** keeps what it owns on disk (`docs/durability.md` §5).
///
/// `scope: execution` means "dies with the run" (grammar 11.1), and a run that
/// reached a `human` pause with nobody to answer it has not ended: it exits `3`
/// with its journal row open, which is the execution `resume` exists for. So the
/// one store whose contents really can outlive the process — a `blob` store,
/// which is a directory whatever its scope — has to still be there.
///
/// `flow.papered` writes a draft, parks, and reads the draft back on the far
/// side of the pause. Getting this wrong is silent: a replayed `put` is never
/// applied a second time, so a partition removed on the way out leaves the
/// live `get` past the frontier answering `found: false` about something the
/// record says the execution wrote. Nothing compares unequal, §7's divergences
/// never fire, and the resume reports `completed` carrying a wrong answer —
/// which is why the assertion is on the value read back rather than on an error.
#[test]
fn a_parked_runs_own_blob_store_is_there_for_the_generation_that_resumes() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(project) = harness::scratch_project("resume-papered") else {
        return;
    };
    let environment = harness::environment(&provider);

    // Nobody can answer this one: no `AGENT_COMPOSE_INTERACTIVE`, so the pause
    // ends the run rather than parking on a promise (grammar 8.7).
    let parked = harness::run_formatted(
        &project,
        "durability",
        "flow.papered",
        &[("topic", "durability")],
        None,
        Some("json"),
        &environment,
    );
    let said_on_stderr = parked.failed();
    let report = parked.outputs();
    assert_eq!(
        report["status"], "interrupted",
        "{report}\n{said_on_stderr}"
    );
    let execution = report["execution_id"]
        .as_str()
        .expect("a run names the execution it opened")
        .to_string();

    // The partition itself, where `src/stores.ts` puts it. Asserted directly
    // because the read below is what a *user* sees and this is what makes it
    // true: a directory the parked generation deleted is not something a later
    // assertion could tell from a store that was never written.
    let partition = project
        .join(".agent-compose")
        .join("blobs")
        .join("papers")
        .join("execution")
        .join(&execution);
    assert!(
        partition.is_dir(),
        "a parked run leaves its own `blob` partition where the resume can read \
         it: `{}` is not there",
        partition.display()
    );

    let resumed = harness::resume_answering(
        &project,
        "durability",
        &execution,
        &environment,
        &["{\"decision\": \"approve\"}"],
        harness::Answers::Closed,
    );
    resumed.succeeded();
    assert_eq!(
        resumed.outputs(),
        json!({ "decision": "approve", "recalled": true, "paper": "durability" }),
        "the live `get` past the frontier reads the world the parked generation \
         left behind: {}",
        resumed.stderr()
    );

    // …and the generation that *ended* the execution took the partition with it,
    // which is the lifetime grammar 11.1 declares.
    assert!(
        !partition.is_dir(),
        "the resumed generation ends the execution, so it removes what the \
         execution owned: `{}` is still there",
        partition.display()
    );
}

/// A detached delivery still in flight when the run stops is **recorded before
/// the run lets go** (`docs/durability.md` §3.2).
///
/// Grammar 8.6 rule 7 says nothing a detached delivery does may delay the
/// enclosing flow instance, and nothing does: the join returns at dispatch. But
/// a delivery is journaled when it *answers*, so a process that walked away from
/// one in flight would leave an effect with no row — and `flow.posted` parks at
/// a pause nobody can answer, which is `agent-compose run`'s own documented way
/// to stop rather than a crash. The resume would then deliver each receipt a
/// second time, systematically.
///
/// The shim writes its line **before** it answers, so the effect is made while
/// the run is still parking and the record can only be there if the run waited
/// for it. Two lines after the resume is the whole assertion: two deliveries,
/// two receipts, across two generations.
#[test]
fn a_detached_delivery_in_flight_when_a_run_parks_is_not_delivered_twice() {
    let provider = MockProvider::start().expect("a loopback port");
    let Some(project) = harness::scratch_project("resume-posted") else {
        return;
    };

    let shims = harness::Scratch::new("posted");
    let log = shims.path().join("receipt.log");
    // The line first, then a delay, then the answer: the effect has happened and
    // the record has not, which is the window the run has to close.
    harness::shim(
        shims.path(),
        "receipt",
        "printf 'filed\\n' >> \"$RECEIPT_LOG\"\nsleep 0.4\nprintf 'filed'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::RECEIPT_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::RECEIPT_LOG.to_string(), log.display().to_string()));

    let parked = harness::run_formatted(
        &project,
        "durability",
        "flow.posted",
        &[("topic", "durability"), ("topics", "[\"one\",\"two\"]")],
        None,
        Some("json"),
        &environment,
    );
    let said_on_stderr = parked.failed();
    let report = parked.outputs();
    assert_eq!(
        report["status"], "interrupted",
        "{report}\n{said_on_stderr}"
    );
    let execution = report["execution_id"]
        .as_str()
        .expect("a run names the execution it opened")
        .to_string();
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the first generation made both deliveries"
    );
    assert!(
        harness::journal_holds(&project, &["receipt/0/0#tool/0", "receipt/0/1#tool/0"]),
        "…and did not stop until both were recorded, because it is going to be \
         resumed: {}",
        said_on_stderr
    );

    let resumed = harness::resume_answering(
        &project,
        "durability",
        &execution,
        &environment,
        &["{\"decision\": \"approve\"}"],
        harness::Answers::Closed,
    );
    resumed.succeeded();
    assert_eq!(
        resumed.outputs(),
        json!({ "decision": "approve" }),
        "{}",
        resumed.stderr()
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the resumed generation consumed both records instead of delivering \
         again (`docs/durability.md` §3.2)"
    );
}

/// A **resumed** generation does not end with a detached delivery still in
/// flight, whether or not it is parked (`docs/durability.md` §3.2, §7).
///
/// The sibling test above is the **parked** half: a run that stops at a pause
/// waits, because a delivery with no record is one the resume makes again. This
/// is the half that decides the *outcome*, and it is not about the record at
/// all.
///
/// A delivery is the one place a `ReplayDivergence` has nothing to be thrown to
/// — nothing joins it (grammar 8.6 rule 7) — so it is latched against the
/// execution instead, and a latch is exactly what makes `runtime.staysOpen`
/// true: it keeps the row **open**, keeps the `scope: execution` blob partition
/// on disk, and fails the resume. A generation that read that predicate at the
/// instant the graph quiesced and then walked away would be reading it while the
/// answer was still being decided: a delivery that latches a moment later leaves
/// the row closed `completed` over a delivery the record describes and nobody
/// made, or the partition removed under the very resume §5 promises it to. Both
/// are unrecoverable, and neither compares unequal to anything.
///
/// So the observable claim is the one that holds the predicate still: **a
/// resumed run that has quiesced is not over until its deliveries are**. The
/// shim writes its line and then holds, so the effect has happened and its
/// record has not; `wrap`'s call is replayed in milliseconds while both
/// deliveries are still in the shim. A generation that let go there exits before
/// either append — the run's own report is written and `process.exit` is
/// immediate — and the records are simply absent.
#[test]
fn a_resumed_generation_does_not_end_with_a_detached_delivery_still_in_flight() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "note": "one note" }))),
        Script::new(SONNET, Outcome::structured(json!({ "note": "two note" }))),
        Script::new(
            SONNET,
            Outcome::structured(json!({ "note": "wrapped" })).after(Duration::from_secs(120)),
        ),
    ]);

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-inflight")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let shims = harness::Scratch::new("inflight");
    let log = shims.path().join("receipt.log");
    // The line first, then a hold, then the answer: the delivery has happened
    // and its record has not, for as long as this test needs both to be true.
    harness::shim(
        shims.path(),
        "receipt",
        "printf 'filed\\n' >> \"$RECEIPT_LOG\"\nsleep 3\nprintf 'filed'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::RECEIPT_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::RECEIPT_LOG.to_string(), log.display().to_string()));

    // Killed **inside** both deliveries rather than after them: the two subflow
    // instances have answered, `wrap`'s call is in flight, and each shim has
    // written its line and is holding. So neither delivery reaches the journal,
    // and the resume has to make them live — which is what puts one in flight
    // while the resumed graph quiesces.
    let killed = harness::crash_run(
        &project,
        &[
            "run",
            "flow.parceled",
            "--input",
            "topic=durability",
            "--input",
            "topics=[\"one\",\"two\"]",
        ],
        &environment,
        |_| provider.snapshot().requests >= 3 && harness::lines_in(&log) == 2,
    );

    // The one call a correct replay still owes the provider: `wrap`'s, which the
    // crash interrupted. The two subflow instances are in the record.
    provider.reset();
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "wrapped" })),
    ));

    let resumed = harness::resume(
        &project,
        "durability",
        &killed.execution,
        Some("json"),
        &environment,
    );
    resumed.succeeded();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "completed", "{answered}");
    assert_eq!(
        answered["outputs"]["note"], "wrapped",
        "the run reached its own answer: {answered}"
    );
    assert_eq!(
        harness::lines_in(&log),
        4,
        "the crash left no record of either delivery, so the resume made both \
         live — which is what puts one in flight at quiescence (§3.2's \
         at-least-once): {}",
        resumed.stderr()
    );
    assert!(
        harness::journal_holds(&project, &["receipt/0/0#tool/0", "receipt/0/1#tool/0"]),
        "a resumed run that quiesced with deliveries still in flight ended \
         anyway: their records are not in the journal, so the reading that \
         decided this execution's outcome was taken before the execution had \
         one: {}",
        resumed.stderr()
    );
}

/// A **human** answer is held to the contract this build declares, and an
/// answer it no longer admits is resolved q29's second divergence.
///
/// The other record kinds reach a result parse, which is where a recorded answer
/// that fails the current contract is caught. A `human` answer reaches none: it
/// is returned straight out of the journal, so a composition that narrowed the
/// node's `output:` under a journal would otherwise resume `completed` carrying
/// a value its own `outputs:` refuses — and, because the enum is narrower, would
/// report a flow output no reader of the composition could have expected.
///
/// `flow.signed` is answered `approve` and killed past the pause, and the resumed
/// copy of the composition admits only `reject`.
#[test]
fn a_recorded_human_answer_the_composition_no_longer_admits_is_a_divergence() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "signed and relayed" }))
            .after(Duration::from_secs(120)),
    ));

    let Some((project, built)) = harness::build_under_toolchain("durability", "resume-narrowed")
    else {
        return;
    };
    assert!(
        built.status.success(),
        "the durability fixture did not build"
    );

    let mut answering = harness::environment(&provider);
    answering.push((harness::INTERACTIVE.to_string(), "1".to_string()));
    let killed = harness::crash_run_answering(
        &project,
        &["run", "flow.signed", "--input", "topic=durability"],
        &answering,
        &["{\"decision\": \"approve\"}"],
        |_| provider.snapshot().requests >= 2,
    );

    // The composition narrows what a person may answer, and nothing else.
    let moved = harness::Scratch::new("narrowed");
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "          decision: { enum: [approve, reject] }",
        "          decision: { enum: [reject] }",
    );
    assert_ne!(
        edited, source,
        "the `output:` line is the one being changed"
    );
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, edited).expect("the scratch area is writable");

    provider.reset();
    let resumed = harness::resume_entrypoint(
        &project,
        &entrypoint,
        &killed.execution,
        Some("json"),
        &harness::environment(&provider),
    );
    let said_on_stderr = resumed.failed();
    let answered = resumed.outputs();
    assert_eq!(answered["status"], "failed", "{answered}\n{said_on_stderr}");
    let said = answered["error"].as_str().expect("an error");
    assert!(
        said.starts_with("ReplayDivergence: "),
        "an answer this composition no longer admits is a divergence rather than \
         a value the run goes on with: {said}"
    );
    assert!(
        said.contains("sign_off/0#human/0"),
        "…naming the step whose recorded answer it is: {said}"
    );
    assert!(
        said.contains("no longer satisfies this run's contract"),
        "…and saying which of the two divergences it is: {said}"
    );
    assert_eq!(
        provider.snapshot().requests,
        0,
        "nothing past the pause was issued"
    );
}

/// A recovered execution delivers the `callback:` webhook its caller is waiting
/// for (`docs/durability.md` §6.1).
///
/// The `async` contract of grammar 13.3 is push: the route answers `202` and the
/// caller waits to be told. The process that finishes an execution need not be
/// the one that answered `202` — `serve` recovers every open execution at start
/// — so the URL is on the lifecycle row, and a recovered run that completes has
/// to fire it. A caller of a run recovered by the second process is not polling
/// the status route, so a lost webhook is a caller with no signal at all.
#[test]
fn a_recovered_execution_delivers_the_completion_webhook_its_caller_waits_for() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("serve-recovered-callback") else {
        return;
    };
    let environment = harness::environment(&provider);

    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/gate-pushed",
                &json!({
                    "topic": "durability",
                    "callback_url": format!("{}/done", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        let status = harness::settled(&app, &execution);
        assert_eq!(status["status"], "interrupted", "{status}");
        resume_url = status["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
        // The parking fires while this process is alive, and is waited for
        // here rather than after the block: `settled` answers the moment the
        // pause opens, which is ahead of the delivery it sets going, and a
        // process killed in between would leave the webhook for the *next*
        // start to make (`docs/durability.md` §3.7) — which is a different
        // claim from the one this test is about.
        let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(30));
        assert_eq!(
            parked[0].body["execution_id"], execution,
            "the parking names the execution: {:?}",
            parked[0].body
        );
        // …and dropped holding the pause: the run never *finished*, so no
        // `settled` webhook has fired and the caller is still waiting.
    }
    assert!(
        receiver.of_event("settled").is_empty(),
        "a run that never finished fired no settle webhook: {:?}",
        receiver.delivered()
    );

    let Some(second) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");
    let recovered = harness::settled(&app, &execution);
    assert_eq!(recovered["status"], "interrupted", "{recovered}");
    let answered = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert!(
        answered.status == 200 || answered.status == 202,
        "{}",
        answered.text()
    );

    let delivered = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    let report = &delivered[0].body;
    assert_eq!(report["execution_id"], execution, "{report}");
    assert_eq!(report["status"], "completed", "{report}");
    assert_eq!(
        report["outputs"],
        json!({ "note": "worth signing off", "decision": "approve" }),
        "the webhook carries the report of the run this process finished: {report}"
    );
    // …and the recovered generation re-parked under the wait id its predecessor
    // published, which is not a new question and fires no second parking
    // (PRD resolved q35).
    assert_eq!(
        receiver.distinct("parked").len(),
        1,
        "re-parking the same wait announces nothing: {:?}",
        receiver.delivered()
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "recovery replayed the model call rather than re-issuing it"
    );
}

/// A recovery that **diverges** delivers no webhook, and the start that
/// finishes the execution delivers exactly one (`docs/durability.md` §6.1, §7).
///
/// A divergence leaves the journal row **open** on purpose: the disagreement
/// belongs to this build's composition rather than to the execution, so putting
/// the composition back is the repair and the next start replays the execution
/// again. A push fired on that outcome would report `failed` about a run that
/// has not finished — and the start that does finish it would then fire a
/// *second* one. Two completions for one execution, the first of them wrong, to
/// a caller who was handed a `202` and is not polling anything.
///
/// So the webhook is owed to a run that **finished**, which is the same
/// predicate the lifecycle row is closed by. What the status route reports is
/// still what this process saw: the report is this build's, the push is the
/// execution's.
#[test]
fn a_diverged_recovery_delivers_no_webhook_and_the_repair_delivers_one() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("serve-diverged-callback") else {
        return;
    };
    let environment = harness::environment(&provider);

    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/gate-pushed",
                &json!({
                    "topic": "durability",
                    "callback_url": format!("{}/done", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        let status = harness::settled(&app, &execution);
        assert_eq!(status["status"], "interrupted", "{status}");
        resume_url = status["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
        // Dropped holding the pause, with the row open and nothing *settled*
        // pushed. Its parking fired, and is on the journal — which is what keeps
        // the two starts below from announcing the same question again.
        receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    }

    // The composition moves under the journal, which is how one does in
    // practice: the prompt this build asks with is not the one the record holds.
    let moved = harness::Scratch::new("diverged-callback");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, moved_prompt()).expect("the scratch area is writable");

    {
        let Some(second) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
        else {
            return;
        };
        let app = Client::new(&second.base_url).expect("a client for the generated app");
        let diverged = harness::settled(&app, &execution);
        assert_eq!(diverged["status"], "failed", "{diverged}");
        assert!(
            diverged["error"]
                .as_str()
                .unwrap_or_default()
                .contains("ReplayDivergence: "),
            "this build could not replay the record, which is what the status route \
             reports: {diverged}"
        );
        // Given time to be sent, and then said not to have been: the push would
        // follow the status this process just published, so a read taken at once
        // would be a read taken too early.
        nothing_settled(&receiver, Duration::from_secs(3));
    }

    // The composition comes back, and so does the execution.
    let Some(third) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&third.base_url).expect("a client for the generated app");
    let recovered = harness::settled(&app, &execution);
    assert_eq!(
        recovered["status"], "interrupted",
        "a diverged resume closed nothing: {recovered}"
    );
    let answered = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert!(
        answered.status == 200 || answered.status == 202,
        "{}",
        answered.text()
    );

    let delivered = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    assert_eq!(
        delivered.len(),
        1,
        "one execution, one settle webhook: {delivered:?}"
    );
    let report = &delivered[0].body;
    assert_eq!(report["execution_id"], execution, "{report}");
    assert_eq!(
        report["status"], "completed",
        "…and it is the report of the run that finished rather than the opinion of \
         the build that could not replay it: {report}"
    );
    assert_eq!(
        receiver.distinct("parked").len(),
        1,
        "three starts, one question, one parking: {:?}",
        receiver.delivered()
    );
    assert_eq!(
        provider.snapshot().requests,
        1,
        "neither recovery re-issued the recorded call"
    );
}

/// A recovery that fails **before the execution is opened** delivers no webhook
/// either, and the start that finishes the execution delivers exactly one
/// (`docs/durability.md` §6.1).
///
/// The sibling test above moves a *prompt*, which fails the replay at an effect
/// — inside the run, with the journal's session open and a `ReplayDivergence` to
/// read. This one moves the flow's own `inputs:`, and that is the sharper case
/// for the same rule: `runFlow` parses the recorded invocation before it opens
/// anything, so the failure is an ordinary `Error` raised with the lifecycle row
/// untouched. It is the shape a whole class of them takes — a `session_key:` the
/// composition has since started requiring, a journal written by another
/// compiler release — and every one of them leaves the row **open** while
/// carrying nothing a predicate over the error could recognise.
///
/// So a build that asked "is this error one that keeps the row open?" instead of
/// "did this generation close the row?" pushes `failed` about an execution that
/// has not finished, and the start that puts the composition back and completes
/// it pushes again: two webhooks for one execution, the first of them wrong, to
/// a caller who was handed a `202` and is polling nothing.
#[test]
fn a_recovery_that_cannot_take_the_recorded_inputs_delivers_no_webhook() {
    let provider = MockProvider::start().expect("a loopback port");
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "note": "worth signing off" })),
    ));

    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("serve-unparseable-callback") else {
        return;
    };
    let environment = harness::environment(&provider);

    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/gate-pushed",
                &json!({
                    "topic": "durability",
                    "callback_url": format!("{}/done", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        let status = harness::settled(&app, &execution);
        assert_eq!(status["status"], "interrupted", "{status}");
        resume_url = status["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
        // Dropped holding the pause, with the row open and nothing *settled*
        // pushed. Its parking fired, and is on the journal — which is what keeps
        // the two starts below from announcing the same question again.
        receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    }

    // The flow's `inputs:` narrow under the journal, so the recorded invocation
    // is one this build refuses to start at all.
    let moved = harness::Scratch::new("unparseable-callback");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, narrowed_gate_inputs()).expect("the scratch area is writable");

    {
        let Some(second) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
        else {
            return;
        };
        let app = Client::new(&second.base_url).expect("a client for the generated app");
        let refused = harness::settled(&app, &execution);
        assert_eq!(refused["status"], "failed", "{refused}");
        assert!(
            !refused["error"]
                .as_str()
                .unwrap_or_default()
                .contains("ReplayDivergence"),
            "this build refused the invocation before the replay began, which is what \
             makes the error one no predicate over its class could catch: {refused}"
        );
        // Given time to be sent, and then said not to have been.
        nothing_settled(&receiver, Duration::from_secs(3));
    }

    // The composition comes back, and so does the execution.
    let Some(third) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&third.base_url).expect("a client for the generated app");
    let recovered = harness::settled(&app, &execution);
    assert_eq!(
        recovered["status"], "interrupted",
        "a start that never opened the execution closed nothing: {recovered}"
    );
    let answered = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert!(
        answered.status == 200 || answered.status == 202,
        "{}",
        answered.text()
    );

    let delivered = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    assert_eq!(
        delivered.len(),
        1,
        "one execution, one settle webhook: {delivered:?}"
    );
    let report = &delivered[0].body;
    assert_eq!(report["execution_id"], execution, "{report}");
    assert_eq!(
        report["status"], "completed",
        "…and it is the report of the run that finished rather than the opinion of \
         the build that would not start it: {report}"
    );
}

/// The `durability` fixture with `flow.gate`'s own `inputs:` narrowed past the
/// invocation the journal recorded, which fails `runFlow`'s parse **before**
/// `runtime.openExecution` — the one place a recovery can fail with the
/// lifecycle row untouched.
fn narrowed_gate_inputs() -> String {
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "  description: Answer, then wait for a person — the execution a restart has to bring back.\n  inputs:\n    topic:\n      type: string\n      min_length: 1\n",
        "  description: Answer, then wait for a person — the execution a restart has to bring back.\n  inputs:\n    topic:\n      type: string\n      min_length: 200\n",
    );
    assert_ne!(
        edited, source,
        "`flow.gate`'s own `inputs:` is what this copy narrows"
    );
    edited
}

/// Give a webhook that must not be sent the time to be sent, and then say it was
/// not.
///
/// A `assert!(delivered().is_empty())` taken the instant a status is published
/// would pass for a build that fires the push a moment later, which is the
/// failure it exists to catch. Polling for a budget cannot flake the other way:
/// a build that fires nothing has nothing to arrive however long this waits.
fn nothing_settled(receiver: &harness::Receiver, budget: Duration) {
    let deadline = std::time::Instant::now() + budget;
    while std::time::Instant::now() < deadline {
        let held = receiver.of_event("settled");
        assert!(
            held.is_empty(),
            "an execution the journal keeps open has not finished, so its caller is \
             owed no settlement yet: {held:?}"
        );
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// An answer that arrives **while a recovered execution is still on its way back
/// to its pause** is told to send it again — not that there is nothing waiting.
///
/// Recovery does not wait for the replays it starts (`docs/durability.md` §6.1):
/// the port is open while they are still catching up. So the `resume_url` a
/// caller holds — the one the dead process published, and the one this one will
/// publish again, because a wait id is the node's instance path and no process
/// generation is part of it (resolved q28) — can be POSTed into that window. The
/// board is empty at that instant, and both refusals that describes are
/// sentences about a pause being **over**: "this execution is not waiting for a
/// human answer" is what a settled one is refused with. A caller that reads
/// either as final drops an answer nothing was wrong with, and the execution
/// waits for ever.
///
/// `flow.held` makes the window a fact rather than a race. Its subprocess is
/// killed with the first generation, so no record of it lands and the generation
/// that recovers the execution has to run it **live** before it can re-park —
/// and the shim runs until this test releases it. Nothing here is timed.
#[test]
fn a_recovered_execution_still_catching_up_tells_a_resume_to_send_it_again() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(project) = harness::scratch_project("serve-recovery-catchup") else {
        return;
    };
    let shims = harness::Scratch::new("held");
    let log = shims.path().join("tally.log");
    // Released by the test, out of the same directory the log is in, so the two
    // halves of the shim's behaviour need one env var between them.
    let release = shims.path().join("tally.log.released");
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\n\
         until [ -e \"$TALLY_LOG.released\" ]; do sleep 0.05; done\n\
         printf 'tallied'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json("/held", &json!({ "topic": "durability" }))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // Waiting for the shim's own line is what makes "killed inside the
        // effect" a fact rather than a sleep: the line is written before it
        // starts waiting to be released.
        wait_for_lines(&log, 1);
        // Dropping it signals the whole process group, so the blocked subprocess
        // goes with the app that spawned it and its record never lands.
    }

    let Some(second) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");
    // Recovery ran in `onReady`, so this execution was on the board before the
    // port opened — and its subprocess is running again, past the frontier,
    // because the killed generation left no record of it (§2).
    wait_for_lines(&log, 2);
    let running: Value = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers")
        .json();
    assert_eq!(
        running["status"], "running",
        "the recovered execution has not come back to its pause yet: {running}"
    );

    // The URL the process that died published for this pause, which is the one
    // this process will publish for it — asserted below, rather than assumed.
    let resume_url = format!("/executions/{execution}/resume?wait=sign_off%2F0");
    let early = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert_eq!(early.status, 409, "{}", early.text());
    let refused = early.json();
    assert_eq!(
        refused["recovering"], true,
        "a refusal of its own, which a client can decide on without reading a sentence: {refused}"
    );
    let sentence = refused["error"].as_str().expect("a refusal says why");
    assert!(
        sentence.contains("send it again"),
        "…telling the caller the answer was not refused: {sentence}"
    );
    assert!(
        !sentence.contains("holding no pause"),
        "…rather than the sentence a pause that is over is refused with: {sentence}"
    );

    // The same window, addressed the other way. An execution holding one pause
    // may be answered without naming it, and that is the refusal whose sentence
    // says there is nothing waiting at all.
    let unnamed = app
        .post_json(
            &format!("/executions/{execution}/resume"),
            &json!({ "decision": "approve" }),
        )
        .expect("the resume route answers");
    assert_eq!(unnamed.status, 409, "{}", unnamed.text());
    let refused = unnamed.json();
    assert_eq!(refused["recovering"], true, "{refused}");
    assert!(
        !refused["error"]
            .as_str()
            .expect("a refusal says why")
            .contains("is not waiting for a human answer"),
        "{refused}"
    );

    // Released: the live effect answers, and the execution re-parks under the
    // wait id its composition fixes.
    std::fs::write(&release, "").expect("the scratch area is writable");
    let parked = harness::settled(&app, &execution);
    assert_eq!(parked["status"], "interrupted", "{parked}");
    let waiting = &parked["interrupts"].as_array().expect("the pauses")[0];
    assert_eq!(waiting["wait_id"], "sign_off/0", "{waiting}");
    assert_eq!(
        waiting["resume_url"].as_str().expect("a resume url"),
        resume_url,
        "…which is the URL the refused answer was addressed to all along: {waiting}"
    );

    // …and the answer the caller was told to send again is taken.
    let answered = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert!(
        answered.status == 200 || answered.status == 202,
        "{}",
        answered.text()
    );
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"],
        json!({ "tally": "tallied", "decision": "approve" }),
        "{finished}"
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the subprocess ran once per generation: the killed one left no record of \
         it, which is the one window a journal cannot close (§2)"
    );
}

/// The same window, for an execution holding **two** pauses whose branches are
/// not back at the same moment (`docs/durability.md` §6.1).
///
/// One execution can hold more than one pause and its branches reach them
/// independently (grammar 8.6). `flow.twinned` is that shape with the two
/// halves deliberately unequal: `sign_now` has nothing before it, so the
/// generation that recovers this execution re-parks it at once, while the pause
/// inside its `flow.held` instance sits behind that flow's unreleased
/// subprocess — an effect the crash left no record of, which this generation
/// therefore has to run **live** before it can reach that pause at all.
///
/// So the first pause back says nothing about the second, and a window closed on
/// the first pause the board sees hands the instance's client the very refusal
/// the window exists to prevent: `this execution is holding no pause`, listing
/// the *other* branch's wait as what is pending. That sentence is what a settled
/// pause is refused with, and a client that reads it as final drops an answer
/// nothing was wrong with — leaving the execution parked for ever.
#[test]
fn a_second_pause_still_being_replayed_to_is_told_to_send_its_answer_again() {
    let provider = MockProvider::start().expect("a loopback port");

    let Some(project) = harness::scratch_project("serve-recovery-two-pauses") else {
        return;
    };
    let shims = harness::Scratch::new("twinned");
    let log = shims.path().join("tally.log");
    let release = shims.path().join("tally.log.released");
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\n\
         until [ -e \"$TALLY_LOG.released\" ]; do sleep 0.05; done\n\
         printf 'tallied'\n",
    );
    let mut environment = harness::environment(&provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "durability", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json("/twinned", &json!({ "topic": "durability" }))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // Both halves reached: one pause published, and the other branch inside
        // the effect it will be killed in.
        let waiting = wait_for_pauses(&app, &execution, 1);
        assert_eq!(
            waiting["interrupts"][0]["wait_id"], "sign_now/0",
            "the branch with nothing before it is the one that parks: {waiting}"
        );
        wait_for_lines(&log, 1);
    }

    let Some(second) = harness::serve_into(&project, "durability", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");
    // Recovery ran in `onReady`. One branch is back at its pause; the other is
    // inside the subprocess the killed generation left no record of.
    wait_for_lines(&log, 2);
    let half = wait_for_pauses(&app, &execution, 1);
    assert_eq!(
        half["interrupts"][0]["wait_id"], "sign_now/0",
        "one of the two is back, and it is the one whose prefix is whole: {half}"
    );

    // The URL the dead process published for the *other* pause. Its wait id is
    // grammar 9.4's instance path — the `flow:` node's frame and then the node
    // inside it — so it is the URL this process will publish for it too,
    // asserted below rather than assumed.
    let later = format!("/executions/{execution}/resume?wait=later%2F0%2Fsign_off%2F0");
    let early = app
        .post_json(&later, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert_eq!(early.status, 409, "{}", early.text());
    let refused = early.json();
    assert_eq!(
        refused["recovering"], true,
        "the pause this answer names is still ahead of the replay, whatever the pause \
         beside it is doing: {refused}"
    );
    let sentence = refused["error"].as_str().expect("a refusal says why");
    assert!(
        sentence.contains("send it again"),
        "…so the caller is told the answer was not refused: {sentence}"
    );
    assert!(
        !sentence.contains("holding no pause"),
        "…rather than the sentence a pause that is over is refused with: {sentence}"
    );

    // Released: the live effect answers and the second branch parks under the id
    // its composition fixes.
    std::fs::write(&release, "").expect("the scratch area is writable");
    let both = wait_for_pauses(&app, &execution, 2);
    let ids: Vec<String> = both["interrupts"]
        .as_array()
        .expect("the pauses")
        .iter()
        .map(|one| one["wait_id"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        ids,
        vec!["later/0/sign_off/0".to_string(), "sign_now/0".to_string()],
        "{both}"
    );
    assert_eq!(
        both["interrupts"][0]["resume_url"].as_str().expect("a url"),
        later,
        "…and the second is at the URL the refused answer was addressed to all \
         along: {both}"
    );

    // …and the answer the caller was told to send again is taken, as is the one
    // beside it.
    let now = format!("/executions/{execution}/resume?wait=sign_now%2F0");
    for (url, body) in [
        (&later, json!({ "decision": "approve" })),
        (&now, json!({ "note": "signed now" })),
    ] {
        let answered = app.post_json(url, &body).expect("the resume route answers");
        assert!(
            answered.status == 200 || answered.status == 202,
            "{}",
            answered.text()
        );
    }
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(
        finished["outputs"],
        json!({ "notes": ["signed now"], "tally": "tallied", "decision": "approve" }),
        "both pauses were answered exactly once, and the live effect answered \
         between them: {finished}"
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the subprocess ran once per generation: the killed one left no record of \
         it, which is the one window a journal cannot close (§2)"
    );
}

/// Poll until a shim's log holds `lines`, and say what it holds if it never
/// does.
///
/// What a test **waits on** rather than what it counts afterwards: the shim of
/// `flow.held` writes its line and then waits to be released, so one line is
/// "the first generation is inside the effect" and two is "so is the generation
/// that recovered it".
fn wait_for_lines(log: &Path, lines: usize) {
    let deadline = std::time::Instant::now() + Duration::from_secs(60);
    loop {
        let held = harness::lines_in(log);
        if held >= lines {
            assert_eq!(
                held, lines,
                "the shim ran more times than this test expects"
            );
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the shim's log never reached {lines} lines; it holds {held}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// `flow.parceled`, run and killed while `wrap`'s model call is in flight.
///
/// Shared by the three tests above because setting it up is the expensive half:
/// two dispatched subflow instances and two detached deliveries have to be
/// **recorded** before the crash, or the resume would meet a frontier where the
/// tests want a comparison.
///
/// So the predicate waits for the **records**, not for the effects. A detached
/// delivery is journaled when it answers and nothing in the run waits for that
/// moment (grammar 8.6 rule 7), so the shim's own log line is written some way
/// ahead of the append that follows it — the child has still to exit, its
/// streams to close and the promise to resolve. Watching the log and then
/// sleeping would be a race this suite loses on a loaded runner, and would lose
/// it as "the delivery ran three times" rather than as "the kill was early".
/// [`harness::journal_holds`] is the same condition read off the journal.
/// What [`crash_a_parceled_run`] hands the tests that share it: the project
/// directory, the killed run, the environment its resume must be started with,
/// and the shim scratch whose lifetime keeps those paths alive.
type CrashedParceledRun = (
    std::path::PathBuf,
    harness::Killed,
    Vec<(String, String)>,
    harness::Scratch,
);

fn crash_a_parceled_run(provider: &MockProvider, purpose: &str) -> Option<CrashedParceledRun> {
    let (project, built) = harness::build_under_toolchain("durability", purpose)?;
    assert!(
        built.status.success(),
        "the durability fixture did not build:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let shims = harness::Scratch::new(purpose);
    let log = shims.path().join("receipt.log");
    harness::shim(
        shims.path(),
        "receipt",
        "printf 'filed\\n' >> \"$RECEIPT_LOG\"\nprintf 'filed'\n",
    );
    let mut environment = harness::environment(provider);
    environment.push((
        harness::RECEIPT_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::RECEIPT_LOG.to_string(), log.display().to_string()));

    let killed = harness::crash_run(
        &project,
        &[
            "run",
            "flow.parceled",
            "--input",
            "topic=durability",
            "--input",
            "topics=[\"one\",\"two\"]",
        ],
        &environment,
        |_| {
            // Both subflow instances answered and `wrap`'s call is the one in
            // flight, and both deliveries are in the record under the dispatch's
            // own instance path (`docs/durability.md` §4).
            provider.snapshot().requests >= 3
                && harness::journal_holds(&project, &["receipt/0/0#tool/0", "receipt/0/1#tool/0"])
        },
    );
    assert_eq!(
        harness::lines_in(&log),
        2,
        "the two deliveries the journal holds are the two the shim made"
    );
    Some((project, killed, environment, shims))
}

/// The `durability` fixture with the note-writing agent's prompt changed, which
/// is how a composition moves under a journal in practice: one line of a prompt
/// is part of every request the agent makes and so part of every recorded
/// request identity.
fn moved_prompt() -> String {
    let source = std::fs::read_to_string(harness::fixture("durability")).expect("the fixture");
    let edited = source.replace(
        "    You write one short note about the topic you are given, and nothing else.",
        "    You write one long essay about the topic you are given, and nothing else.",
    );
    assert_ne!(edited, source, "the prompt line is the one being changed");
    edited
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

// ---------------------------------------------------------------------------
// The http-native events surface (grammar 13.3, PRD resolved q32–q35)
// ---------------------------------------------------------------------------

/// The `http-events` fixture's environment: the harness's own, plus the shim
/// `flow.note` counts its executions with.
///
/// The counting shim is what makes "the refused request started **no**
/// execution" an assertion rather than an inference: a `401` says what the route
/// answered and nothing about what ran behind it, while a log holding one line
/// per admitted call and none per refusal says both.
fn events_environment(
    provider: &MockProvider,
    purpose: &str,
) -> (harness::Scratch, std::path::PathBuf, Vec<(String, String)>) {
    let shims = harness::Scratch::new(purpose);
    let log = shims.path().join("tally.log");
    harness::shim(
        shims.path(),
        "tally",
        "printf 'ran\\n' >> \"$TALLY_LOG\"\nprintf 'noted'\n",
    );
    // The second shim beside it: awaited work of a length the caller chooses,
    // which is what makes the pauses of one fan-out open at different moments
    // (`flow.consider`). `pause_for` arrives upper-snake-cased, as an `exec:`
    // node's bindings do (grammar 5.5).
    harness::shim(
        shims.path(),
        "linger",
        "sleep \"${PAUSE_FOR:-0}\"\nprintf 'lingered'\n",
    );
    // The third: work that runs until the test says otherwise, which is what a
    // **detached** delivery has to be for an assertion about a parking made
    // while one is in flight to be a fact rather than a race against a sleep.
    // The line goes down before the wait starts, so a test that has seen it
    // knows the sink is inside the loop, and only a file the test creates lets
    // it out.
    harness::shim(
        shims.path(),
        "hold",
        "printf 'holding\\n' >> \"$TALLY_LOG.hold\"\n\
         until [ -e \"$TALLY_LOG.released\" ]; do sleep 0.05; done\n\
         printf 'held'\n",
    );
    // …and its other half, which the flow that dispatched the delivery runs
    // before it parks: it returns once the sink is inside its wait, so the pause
    // opens with the delivery in flight rather than racing it.
    harness::shim(
        shims.path(),
        "escorting",
        "until [ -e \"$TALLY_LOG.hold\" ]; do sleep 0.05; done\nprintf 'escorting'\n",
    );
    let mut environment = harness::environment(provider);
    environment.push((
        harness::TALLY_BIN.to_string(),
        shims.path().display().to_string(),
    ));
    environment.push((harness::TALLY_LOG.to_string(), log.display().to_string()));
    (shims, log, environment)
}

/// The `Authorization: Bearer <token>` an `http-events` caller sends.
fn events_bearer() -> String {
    format!("Bearer {}", harness::credential(harness::EVENTS_TOKEN))
}

/// Poll a **guarded** execution's status route until the run has completed,
/// presenting the credential its starting trigger demands.
///
/// `harness::settled` cannot be used for one: its poller sends no credential,
/// and an execution an authenticated trigger started never answers an
/// unauthenticated poll — which is the rule these tests are about.
///
/// The credential is the caller's to build, because the two schemes ask for
/// different things and an `hmac:` one asks for something worth stating: the
/// signature is over the **request's** body, so a `GET` of a status route signs
/// the empty body it carries. That is what "covers three routes, not one"
/// (grammar 13.3) costs a client of a signed trigger, and it is the shape a
/// receiver of one has to write.
fn completed(app: &Client, execution: &str, credential: impl Fn(Request) -> Request) -> Value {
    harness::until(Duration::from_secs(30), || {
        let held = app
            .send(credential(Request::get(format!("/executions/{execution}"))))
            .expect("the status route answers");
        assert_eq!(held.status, 200, "{}", held.text());
        let report = held.json();
        (report["status"] == "completed").then_some(report)
    })
}

/// The `X-Hub-Signature-256` the `signed` and `sealed` triggers expect over
/// these bytes — the prefix, algorithm and encoding both declare.
///
/// The argument is **bytes** and never a `Value`, because the whole property
/// under test is that the digest is over what a request carried rather than over
/// something either side re-serialized: a helper taking a value would hash a
/// string the app never saw, and would agree with a build that hashed one too.
fn events_signature(bytes: &[u8]) -> String {
    format!(
        "sha256={}",
        harness::hmac_sha256(
            harness::credential(harness::EVENTS_SECRET).as_bytes(),
            bytes,
        )
    )
}

/// A request to the `signed` trigger, signed the way its `hmac:` expects.
///
/// The signature is over the **bytes this request carries** rather than over a
/// value re-serialized on either side, which is the property grammar 13.3 makes
/// normative: a body hashed after a decode-and-re-encode is a different byte
/// string, and a verifier that did that would refuse every genuine delivery.
fn signed_request(path: &str, body: &Value) -> Request {
    let bytes = serde_json::to_vec(body).expect("a JSON body");
    let signature = events_signature(&bytes);
    Request::post(path)
        .header("content-type", "application/json")
        .header("X-Hub-Signature-256", signature)
        .bytes(bytes)
}

/// This harness's own HMAC answers the published vector, so a signature it
/// disagrees with is the app's mistake rather than its own (RFC 4231 case 2).
///
/// Every signing assertion below is a comparison against a value computed here,
/// which is only worth something if this side is right. The vector is the one
/// every implementation is checked against, and it is checked here rather than
/// left to a reader to trust sixty lines of shifts.
#[test]
fn the_harnesss_own_hmac_answers_the_published_vector() {
    assert_eq!(
        harness::hmac_sha256(b"Jefe", b"what do ya want for nothing?"),
        "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
    );
}

/// A trigger's `auth:` admits the credential it declares and refuses every
/// other, starting nothing (grammar 13.3, PRD resolved q32).
///
/// Both schemes, because they fail differently and only one of them can be
/// tested by sending the wrong string. `bearer` is a comparison against a static
/// value, so what a test varies is the value: absent, wrong, and — the case a
/// prefix-and-`startsWith` implementation gets wrong — a **truncation** of the
/// real one, which shares its whole prefix. `hmac` is a comparison against a
/// digest of the body, so what a test varies is the **body**: the same signature
/// over a payload edited after it was computed is what a replayed request looks
/// like, and a verifier that hashed the re-serialized value rather than the
/// bytes would take it.
///
/// The counting shim is what makes the second half of resolved q32's sentence —
/// "and no execution started" — observable. A `401` says what the route
/// answered; the log says what ran.
#[test]
fn an_authenticated_start_admits_the_credential_it_declares_and_refuses_every_other() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, log, environment) = events_environment(&provider, "events-start-auth");
    let Some(project) = harness::scratch_project("events-start-auth") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let body = json!({ "topic": "events" });

    // Every way a bearer credential can be wrong, and the one way it is right.
    let token = harness::credential(harness::EVENTS_TOKEN);
    for (what, header) in [
        ("no credential at all", None),
        (
            "a token that is not the one",
            Some(format!("Bearer {token}-and-more")),
        ),
        // Shares the whole prefix of the real one, which is what a comparison
        // that stopped at the first difference — or at the shorter length —
        // would take.
        (
            "a truncation of the token",
            Some(format!("Bearer {}", &token[..token.len() - 3])),
        ),
        ("the token under no prefix", Some(token.to_string())),
    ] {
        let mut request = Request::post("/guarded").json(&body);
        if let Some(value) = header {
            request = request.header("authorization", value);
        }
        let refused = app.send(request).expect("the trigger's route answers");
        assert_eq!(refused.status, 401, "{what}: {}", refused.text());
        let said = refused.json();
        assert_eq!(said["trigger"], "guarded", "{said}");
        assert_eq!(said["scheme"], "bearer", "{said}");
        assert!(
            said.get("execution_id").is_none(),
            "a refused request starts nothing to report: {said}"
        );
        assert!(
            !refused.text().contains(token),
            "a refusal never echoes credential material: {}",
            refused.text()
        );
    }

    let accepted = app
        .send(
            Request::post("/guarded")
                .json(&body)
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(accepted.status, 202, "{}", accepted.text());
    let execution = accepted.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    // Polled with the credential, because the status route of an execution an
    // authenticated trigger started is guarded too — which is the sibling test's
    // subject and this one's premise.
    let finished = completed(&app, &execution, |request| {
        request.header("authorization", events_bearer())
    });
    assert_eq!(finished["status"], "completed", "{finished}");

    // The signed trigger: the same body signed verifies, and the same signature
    // over a body edited afterwards does not.
    let signed = app
        .send(signed_request("/signed", &body))
        .expect("the trigger's route answers");
    assert_eq!(signed.status, 202, "{}", signed.text());
    let empty = harness::hmac_sha256(harness::credential(harness::EVENTS_SECRET).as_bytes(), b"");
    completed(
        &app,
        signed.json()["execution_id"]
            .as_str()
            .expect("an execution id"),
        |request| request.header("X-Hub-Signature-256", format!("sha256={empty}")),
    );

    // …and the case that tells raw-byte verification from re-serialized
    // verification, which the tampered request below cannot: a **legitimate**
    // body whose bytes are not what re-serializing it would produce. A
    // pretty-printed payload with a trailing newline is what a vendor sends, and
    // an implementation that hashed `JSON.stringify(request.body)` computes a
    // different digest for it and answers `401` — while passing every other
    // assertion in this test, because a tampered body re-serializes differently
    // too. This is the failure grammar 13.3 spells out.
    let spaced = b"{\"topic\": \"events\"}\n".to_vec();
    let over_bytes = harness::hmac_sha256(
        harness::credential(harness::EVENTS_SECRET).as_bytes(),
        &spaced,
    );
    let sent = app
        .send(
            Request::post("/signed")
                .header("content-type", "application/json")
                .header("X-Hub-Signature-256", format!("sha256={over_bytes}"))
                .bytes(spaced),
        )
        .expect("the trigger's route answers");
    assert_eq!(
        sent.status,
        202,
        "a signature over the bytes a request carried verifies, whatever re-serializing \
         the body would have produced: {}",
        sent.text()
    );
    completed(
        &app,
        sent.json()["execution_id"]
            .as_str()
            .expect("an execution id"),
        |request| request.header("X-Hub-Signature-256", format!("sha256={empty}")),
    );

    let tampered = signed_request("/signed", &body)
        .bytes(serde_json::to_vec(&json!({ "topic": "something else" })).expect("a JSON body"));
    let refused = app.send(tampered).expect("the trigger's route answers");
    assert_eq!(refused.status, 401, "{}", refused.text());
    assert_eq!(refused.json()["scheme"], "hmac", "{}", refused.text());
    // …and a signature that is not one at all, which is the shape a caller who
    // signed with the wrong secret sends.
    let wrong = app
        .send(
            Request::post("/signed")
                .json(&body)
                .header("X-Hub-Signature-256", "sha256=00"),
        )
        .expect("the trigger's route answers");
    assert_eq!(wrong.status, 401, "{}", wrong.text());

    assert_eq!(
        harness::lines_in(&log),
        3,
        "three requests were admitted and five were refused, so three executions ran"
    );
}

/// A credential a request carried **twice** verifies nothing, on either scheme
/// (grammar 13.3, PRD resolved q32).
///
/// Which of two values under one name a verifier reads is a thing HTTP stacks
/// disagree about, and the two runtimes a generated project supports disagree
/// with each other: Node's parsed headers keep the **first** `authorization` and
/// drop the rest, Bun's keep the **last**, and both join repeats of every other
/// name with `", "`. So a route deciding on the parsed map admits a request
/// whose credential depends on which runtime the app was launched with — and a
/// proxy in front of it that normalises on the other value is a header-smuggling
/// differential in which the pair `<valid>, <junk>` and the pair `<junk>,
/// <valid>` are the same request to two readers.
///
/// The pairs are sent in both orders for exactly that reason: an app that took
/// the first value passes the second case and an app that took the last passes
/// the first, and only a refusal passes both. The counting shim says the other
/// half — a request offering two credentials starts nothing, like every other
/// refusal on this route.
#[test]
fn a_credential_a_request_carried_twice_verifies_nothing() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, log, environment) = events_environment(&provider, "events-repeated-credential");
    let Some(project) = harness::scratch_project("events-repeated-credential") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");
    let body = json!({ "topic": "repeats" });

    let token = harness::credential(harness::EVENTS_TOKEN);
    let junk = format!("Bearer {token}-and-more");
    for (what, first, second) in [
        ("the real credential first", events_bearer(), junk.clone()),
        ("the real credential last", junk, events_bearer()),
        // Two copies of the *same* valid credential, which is the case a rule
        // written as "the values must agree" would still admit: a request
        // carrying one credential carries it once.
        (
            "the same credential twice",
            events_bearer(),
            events_bearer(),
        ),
    ] {
        let refused = app
            .send(
                Request::post("/guarded")
                    .json(&body)
                    .header("authorization", first)
                    .header("authorization", second),
            )
            .expect("the trigger's route answers");
        assert_eq!(refused.status, 401, "{what}: {}", refused.text());
        let said = refused.json();
        assert_eq!(said["scheme"], "bearer", "{what}: {said}");
        assert!(
            !refused.text().contains(token),
            "{what}: a refusal never echoes credential material: {}",
            refused.text()
        );
    }

    // The signed scheme reaches the same lookup, so a rule applied to the bearer
    // half alone would leave the header a vendor's signature arrives under
    // resolved by whichever stack read it.
    let bytes = serde_json::to_vec(&body).expect("a JSON body");
    let signature = harness::hmac_sha256(
        harness::credential(harness::EVENTS_SECRET).as_bytes(),
        &bytes,
    );
    let signed = format!("sha256={signature}");
    for (what, first, second) in [
        (
            "the real signature first",
            signed.clone(),
            "sha256=00".to_string(),
        ),
        ("the real signature last", "sha256=00".to_string(), signed),
    ] {
        let refused = app
            .send(
                Request::post("/signed")
                    .header("content-type", "application/json")
                    .header("X-Hub-Signature-256", first)
                    .header("X-Hub-Signature-256", second)
                    .bytes(bytes.clone()),
            )
            .expect("the trigger's route answers");
        assert_eq!(refused.status, 401, "{what}: {}", refused.text());
        assert_eq!(
            refused.json()["scheme"],
            "hmac",
            "{what}: {}",
            refused.text()
        );
    }

    assert_eq!(
        harness::lines_in(&log),
        0,
        "every request offered two credentials and none of them started an execution"
    );
}

/// An execution's `status` and `resume` routes enforce the auth of the trigger
/// that **started** it, and an execution a no-auth trigger began keeps them open
/// (grammar 13.3's "covers three routes, not one", PRD resolved q32).
///
/// Both halves, because either alone is satisfied by a wrong build. A test that
/// only watched a guarded execution refuse an unauthenticated poll would pass on
/// an app that refused every poll of every execution; a test that only watched
/// an open one answer would pass on an app that enforced nothing anywhere. What
/// the pair pins is that the guard is the **execution's**, carried on its
/// lifecycle row rather than derived from the route.
///
/// `resume` is the sharper of the two routes and is checked for the thing a
/// refusal has to leave behind: the pause is still waiting afterwards, so a
/// refused answer consumed no turn.
#[test]
fn an_executions_status_and_resume_enforce_the_auth_of_the_trigger_that_started_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-route-auth");
    let Some(project) = harness::scratch_project("events-route-auth") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // The guarded execution. No `callback_url`, so this is about the routes
    // alone.
    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({ "topic": "guarded" }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let guarded = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let poll = format!("/executions/{guarded}");
    let unauthenticated = app.get(&poll).expect("the status route answers");
    assert_eq!(unauthenticated.status, 401, "{}", unauthenticated.text());
    assert_eq!(unauthenticated.json()["trigger"], "watched");
    assert!(
        unauthenticated.json().get("status").is_none(),
        "a refused poll answers nothing about the execution: {}",
        unauthenticated.text()
    );

    // Polling takes the credential too, so the harness's own poller cannot be
    // used for this execution: the report is read by hand until the pause is up.
    let published = harness::until(Duration::from_secs(30), || {
        let held = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers");
        assert_eq!(held.status, 200, "{}", held.text());
        let report = held.json();
        (report["status"] == "interrupted").then_some(report)
    });
    let resume_url = published["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();

    let refused = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert_eq!(refused.status, 401, "{}", refused.text());

    // …and the pause is still waiting, so the refusal consumed no turn.
    let still = app
        .send(Request::get(&poll).header("authorization", events_bearer()))
        .expect("the status route answers");
    assert_eq!(still.json()["status"], "interrupted", "{}", still.text());

    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    // The open execution, whose routes take no credential at all.
    let opened = app
        .post_json("/open", &json!({ "topic": "open" }))
        .expect("the trigger's route answers");
    assert_eq!(opened.status, 202, "{}", opened.text());
    let open = opened.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let unguarded = harness::settled(&app, &open);
    assert_eq!(unguarded["status"], "interrupted", "{unguarded}");
    let open_resume = unguarded["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();
    let taken = app
        .post_json(&open_resume, &json!({ "decision": "reject" }))
        .expect("the resume route answers");
    assert_eq!(
        taken.status,
        202,
        "an execution a no-auth trigger began keeps open routes: {}",
        taken.text()
    );
}

/// A **signed** execution's status and resume routes verify a signature over
/// the body of the request that arrived — which, for a `GET`, is no body at all
/// (grammar 13.3's "covers three routes, not one", PRD resolved q32).
///
/// The pair above runs entirely on `bearer`, where all three routes take one
/// value and a build could read it from anywhere. `hmac` is the half where "the
/// auth of the trigger that started this execution" is a *different* credential
/// per request, and two plausible builds satisfy every bearer assertion in this
/// suite: one that verifies the resume route over the decoded-and-re-serialized
/// payload, and one that verifies it over the empty body the status route signs.
/// Both are offered here and both have to be refused, over a body whose bytes no
/// serializer would emit — the spaces are the point, because they are what makes
/// the payload as sent and the payload as decoded two different byte strings.
///
/// The refusals are also checked for what they left behind: the pause is still
/// waiting afterwards, so a wrong digest consumed no turn.
#[test]
fn a_signed_executions_status_and_resume_verify_over_each_requests_own_body() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-signed-routes");
    let Some(project) = harness::scratch_project("events-signed-routes") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let topic = json!({ "topic": "signed routes" });
    let started = app
        .send(signed_request("/sealed", &topic))
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let poll = format!("/executions/{execution}");

    let unsigned = app.get(&poll).expect("the status route answers");
    assert_eq!(unsigned.status, 401, "{}", unsigned.text());
    assert_eq!(unsigned.json()["scheme"], "hmac", "{}", unsigned.text());
    assert_eq!(unsigned.json()["trigger"], "sealed", "{}", unsigned.text());

    // A signature over the payload that *started* the execution is a signature
    // over a body this request does not carry — the shape a client that reused
    // the start request's header would send.
    let stale = events_signature(&serde_json::to_vec(&topic).expect("a JSON body"));
    let wrong = app
        .send(Request::get(&poll).header("X-Hub-Signature-256", stale))
        .expect("the status route answers");
    assert_eq!(wrong.status, 401, "{}", wrong.text());

    // A `GET` carries no body, so what it signs is none of it.
    let nothing = events_signature(b"");
    let published = harness::until(Duration::from_secs(30), || {
        let held = app
            .send(Request::get(&poll).header("X-Hub-Signature-256", nothing.clone()))
            .expect("the status route answers");
        assert_eq!(held.status, 200, "{}", held.text());
        let report = held.json();
        (report["status"] == "interrupted").then_some(report)
    });
    let resume_url = published["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();

    let answer = br#"{ "decision" : "approve" }"#;
    let decoded = br#"{"decision":"approve"}"#;
    for (what, signature) in [
        (
            "a digest over the payload as a decoder hands it back",
            events_signature(decoded),
        ),
        (
            "a digest over the empty body the status route signs",
            nothing.clone(),
        ),
    ] {
        let refused = app
            .send(
                Request::post(&resume_url)
                    .header("content-type", "application/json")
                    .header("X-Hub-Signature-256", signature)
                    .bytes(answer.to_vec()),
            )
            .expect("the resume route answers");
        assert_eq!(refused.status, 401, "{what}: {}", refused.text());
        assert_eq!(
            refused.json()["scheme"],
            "hmac",
            "{what}: {}",
            refused.text()
        );
    }

    let still = app
        .send(Request::get(&poll).header("X-Hub-Signature-256", nothing.clone()))
        .expect("the status route answers");
    assert_eq!(
        still.json()["status"],
        "interrupted",
        "a refused answer consumed no turn: {}",
        still.text()
    );

    let answered = app
        .send(
            Request::post(&resume_url)
                .header("content-type", "application/json")
                .header("X-Hub-Signature-256", events_signature(answer))
                .bytes(answer.to_vec()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    let finished = completed(&app, &execution, |request| {
        request.header("X-Hub-Signature-256", events_signature(b""))
    });
    assert_eq!(finished["outputs"]["decision"], "approve", "{finished}");
}

/// One parking, one settle, in ascending ordinals, each carrying the report the
/// status route serves (grammar 13.3, PRD resolved q34).
///
/// The parking is the half a completion webhook could not stand in for, and its
/// body is what makes a *push* worth as much as a poll: the pause, what the
/// person is shown, the schema their answer has to fit, and the URL that
/// delivers it. So the test answers the pause **through the URL the webhook
/// carried** rather than through one it read off the status route — a delivery
/// whose `resume_url` did not work would otherwise pass.
#[test]
fn a_parking_and_a_settle_reach_the_callback_in_ascending_ordinals() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-lifecycle");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-lifecycle") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({
                    "topic": "lifecycle",
                    "callback_url": format!("{}/allowed", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    let first = &parked[0];
    assert_eq!(
        first.header("x-agentcompose-delivery"),
        Some(format!("{execution}:0").as_str()),
        "the delivery id is `<execution_id>:<ordinal>`: {:?}",
        first.headers
    );
    assert_eq!(first.header("x-agentcompose-ordinal"), Some("0"));
    assert!(
        first
            .header("x-agentcompose-timestamp")
            .is_some_and(|at| at.ends_with('Z') && at.contains('T')),
        "the delivery is stamped ISO-8601: {:?}",
        first.headers
    );
    assert_eq!(first.body["execution_id"], execution, "{:?}", first.body);
    assert_eq!(first.body["status"], "interrupted", "{:?}", first.body);
    let pause = &first.body["interrupts"].as_array().expect("the pauses")[0];
    assert_eq!(pause["node"], "sign_off", "{pause}");
    assert_eq!(pause["input"]["note"], "lifecycle", "{pause}");
    assert!(
        pause["output_schema"]["properties"]["decision"].is_object(),
        "{pause}"
    );
    let resume_url = pause["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();

    // Answered through the URL the *webhook* published, which is what makes the
    // push a surface a receiver can build on.
    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    let settled = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    let second = &settled[0];
    assert_eq!(second.header("x-agentcompose-ordinal"), Some("1"));
    assert_eq!(
        second.header("x-agentcompose-delivery"),
        Some(format!("{execution}:1").as_str())
    );
    assert_eq!(second.body["status"], "completed", "{:?}", second.body);
    assert_eq!(
        second.body["outputs"]["decision"], "approve",
        "{:?}",
        second.body
    );

    // One parking and one settle, in that order, and the resume that came
    // between them announced no second parking of a wait already reported.
    assert_eq!(receiver.distinct("parked"), [format!("{execution}:0")]);
    assert_eq!(receiver.distinct("settled"), [format!("{execution}:1")]);
}

/// A delivery carries the identity its trigger declared, over the bytes it
/// actually sent (grammar 13.3, PRD resolved q33).
///
/// Both outbound schemes at once, which is the shape grammar 13.3 makes legal
/// outbound and not inbound. The signature is verified here with **this
/// repository's own** HMAC rather than the app's, over the exact bytes the
/// receiver read: a check that re-serialized the decoded body would agree with
/// an implementation that signed a re-serialization too, and both would fail
/// against any receiver written to the published recipe.
///
/// The bearer half is checked for the two things this trigger configured away
/// from their defaults — the header name and the prefix — because those are the
/// values a build could plausibly hard-code and still pass a test written
/// against the defaults.
#[test]
fn a_delivery_carries_the_identity_its_trigger_declared_over_the_bytes_it_sent() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-signing");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-signing") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({
                    "topic": "signing",
                    "callback_url": format!("{}/allowed/deliveries", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());

    let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    let delivery = &parked[0];

    let expected = harness::hmac_sha256(
        harness::credential(harness::DELIVERY_SECRET).as_bytes(),
        &delivery.bytes,
    );
    assert_eq!(
        delivery.header("x-agentcompose-signature"),
        Some(format!("sha256={expected}").as_str()),
        "the signature is HMAC-SHA256 in hex over the delivered body: {:?}",
        delivery.headers
    );
    assert_eq!(
        delivery.header("x-delivery-token"),
        Some(format!("Token {}", harness::credential(harness::DELIVERY_TOKEN)).as_str()),
        "the token is written under the header and prefix the trigger declared: {:?}",
        delivery.headers
    );
    assert_eq!(
        delivery.header("content-type"),
        Some("application/json"),
        "{:?}",
        delivery.headers
    );
    // The inbound credentials are a different pair, and a delivery carries
    // neither: a build that resolved one variable where it meant another would
    // pass every assertion above that named only a shape.
    let sent = String::from_utf8_lossy(&delivery.bytes);
    for leaked in [
        harness::credential(harness::EVENTS_TOKEN),
        harness::credential(harness::EVENTS_SECRET),
        harness::credential(harness::DELIVERY_SECRET),
    ] {
        assert!(
            !sent.contains(leaked)
                && !delivery
                    .headers
                    .values()
                    .any(|value| value.contains(leaked)),
            "a delivery carries no credential but the one its `bearer:` declares: {:?}",
            delivery.headers
        );
    }
}

/// A callback URL the allowlist admits nowhere is a **refused delivery**:
/// journaled, visible on the status route, never sent, and never the execution's
/// failure (grammar 13.3, Decision D127, PRD resolved q33).
///
/// The URL is matched when it is **read** — at the parking and at the settle —
/// rather than at the start, which is why the request that named it is a `202`
/// like any other. What the run does afterwards is the assertion that matters:
/// it parks, it is answered, and it completes, with two refusals recorded beside
/// it and nothing at the receiver.
#[test]
fn a_callback_url_the_allowlist_admits_nowhere_is_refused_and_the_run_settles() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-allowlist");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-allowlist") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // `/denied` is on the receiver the allowlist admits and under a path it does
    // not: the entry is `http://127.0.0.1:*/allowed*`, so this is the delivery a
    // match against the host alone would have let through.
    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({
                    "topic": "allowlist",
                    "callback_url": format!("{}/denied", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(
        started.status,
        202,
        "the URL is read at the delivery, not at the start: {}",
        started.text()
    );
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let poll = format!("/executions/{execution}");
    let refused = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers")
            .json();
        (report["deliveries"].as_array().map(Vec::len) == Some(1)).then_some(report)
    });
    let recorded = &refused["deliveries"].as_array().expect("the deliveries")[0];
    assert_eq!(recorded["status"], "refused", "{recorded}");
    assert_eq!(recorded["event"], "parked", "{recorded}");
    assert_eq!(recorded["ordinal"], 0, "{recorded}");
    assert!(
        recorded["attempts"].as_array().is_some_and(Vec::is_empty),
        "a refused delivery was never attempted: {recorded}"
    );
    assert!(
        recorded["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("callback_allow"),
        "…and it says why: {recorded}"
    );

    let resume_url = refused["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();
    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    let finished = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers")
            .json();
        (report["status"] == "completed"
            && report["deliveries"].as_array().map(Vec::len) == Some(2))
        .then_some(report)
    });
    assert_eq!(
        finished["deliveries"].as_array().expect("the deliveries")[1]["status"],
        "refused",
        "{finished}"
    );
    assert_eq!(
        finished["outputs"]["decision"], "approve",
        "a refused delivery is nobody's failure: {finished}"
    );
    assert!(
        receiver.delivered().is_empty(),
        "nothing was sent: {:?}",
        receiver.delivered()
    );
}

/// A callback URL that hides its host behind **userinfo** is a refused delivery,
/// whatever the text of the list says about it (grammar 13.3, Decision D127).
///
/// `callback_allow:` is matched against the URL as a string, and a string is not
/// where a URL's authority ends: everything before an `@` is userinfo, and the
/// host is what follows it. So `http://127.0.0.1:9000@elsewhere/allowed` begins
/// with `http://127.0.0.1:` and holds `/allowed`, which is every literal the
/// entry `http://127.0.0.1:*/allowed*` asks for — the entry this fixture writes
/// because a loopback receiver binds a port the operating system chose, and the
/// entry any deployment with a per-tenant or per-environment port writes too.
/// What would travel to the host after the `@` is the execution's whole report,
/// under `X-AgentCompose-Signature` and a `callback_auth: bearer` token written
/// to a header name of the composition's choosing.
///
/// The two runtimes a built project runs under do not even agree what such a URL
/// means — one drops the userinfo and delivers, the other refuses to construct
/// the request — so the assertion is on the *refusal* rather than on a
/// blackhole staying quiet alone: a build that happened to throw would leave a
/// `pending` row and an attempt, and a build that delivered would leave a
/// `delivered` one. The detail is checked for **not** being the list's own
/// refusal sentence, because an entry that stopped matching this URL's text
/// would make the whole test pass for the wrong reason.
#[test]
fn a_callback_url_that_hides_its_host_behind_userinfo_is_refused() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, mut environment) = events_environment(&provider, "events-userinfo");
    // One offset, so a build that *did* send would exhaust rather than spend
    // fifteen minutes proving it.
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s".to_string()));
    let elsewhere = harness::Blackhole::start().expect("a loopback socket");
    let Some(project) = harness::scratch_project("events-userinfo") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let host = elsewhere
        .base_url
        .strip_prefix("http://")
        .expect("the blackhole's base url names its scheme");
    let callback = format!("http://127.0.0.1:9000@{host}/allowed");
    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({ "topic": "userinfo", "callback_url": callback }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(
        started.status,
        202,
        "the URL is read at the delivery, not at the start: {}",
        started.text()
    );
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let poll = format!("/executions/{execution}");
    let refused = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers")
            .json();
        (report["deliveries"].as_array().map(Vec::len) == Some(1)).then_some(report)
    });
    let recorded = &refused["deliveries"].as_array().expect("the deliveries")[0];
    assert_eq!(recorded["status"], "refused", "{recorded}");
    assert_eq!(recorded["event"], "parked", "{recorded}");
    assert!(
        recorded["attempts"].as_array().is_some_and(Vec::is_empty),
        "a refused delivery was never attempted: {recorded}"
    );
    let detail = recorded["detail"].as_str().unwrap_or_default();
    assert!(
        detail.contains("userinfo"),
        "…and it says which rule refused it: {recorded}"
    );
    assert!(
        !detail.contains("matches no `callback_allow` entry"),
        "the list admits this URL's text — the list's own refusal here would mean the entry \
         `http://127.0.0.1:*/allowed*` stopped matching and this test proves nothing: {recorded}"
    );

    let resume_url = refused["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();
    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    let finished = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers")
            .json();
        (report["status"] == "completed"
            && report["deliveries"].as_array().map(Vec::len) == Some(2))
        .then_some(report)
    });
    assert_eq!(
        finished["deliveries"].as_array().expect("the deliveries")[1]["status"],
        "refused",
        "{finished}"
    );
    assert_eq!(
        finished["outputs"]["decision"], "approve",
        "a refused delivery is nobody's failure: {finished}"
    );
    assert_eq!(
        elsewhere.reached(),
        0,
        "the host after the `@` was never connected to"
    );
}

/// A receiver that answers a delivery with a **redirect** sends it nowhere else
/// (grammar 13.3, Decision D127).
///
/// `callback_allow:` is matched against the URL the trigger produced, so a
/// delivery that followed a `Location:` would make the list a bound on the first
/// hop rather than on where a signed report lands. And the list admits an open
/// redirect as readily as any other path: `http://127.0.0.1:*/allowed*` is
/// satisfied by `…/allowed?to=https://attacker.test/collect`, which is a URL an
/// author's own receiver may well answer `307` to. What would travel is this
/// execution's whole report — outputs, trace, and every human answer — under
/// `X-AgentCompose-Signature` and a `callback_auth: bearer` token written to a
/// header name of the composition's choosing — what a cross-origin redirect
/// strips is a fixed list of standard credential headers, never a name a
/// composition chose and never the body's signature.
///
/// The blackhole is the assertion, and it counts **connections** rather than
/// deliveries: a `301`, `302` or `303` rewrites the POST into a bodyless `GET`,
/// which reaches a host without looking like a delivery at all — and which is
/// the quieter half of the same bug, since an allowlisted receiver redirecting
/// to itself would then answer `2xx` to a request carrying no report and the row
/// would be journaled `delivered`.
///
/// The `307` is a failed attempt like any other non-2xx, so the schedule runs
/// out and the row is `exhausted` — and the execution parks, is answered and
/// completes regardless, which is what a webhook being a courtesy means.
#[test]
fn a_receiver_that_redirects_a_delivery_sends_it_nowhere_else() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, mut environment) = events_environment(&provider, "events-redirect");
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let elsewhere = harness::Blackhole::start().expect("a loopback socket");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    // `307` preserves the method and the body, which is the redirect that would
    // carry the report itself. The bodyless-`GET` rewrite of a `303` is the same
    // root cause and is covered by the same assertion.
    receiver.redirecting_to(307, &format!("{}/collect", elsewhere.base_url));
    let Some(project) = harness::scratch_project("events-redirect") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({
                    "topic": "redirect",
                    "callback_url": format!("{}/allowed/deliveries", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // Both attempts of the parking, so what is asserted below is the whole of
    // the schedule rather than whichever attempt happened to have landed.
    let attempted = receiver.wait_for_event("parked", 2, Duration::from_secs(30));
    assert!(
        attempted[0].header("x-agentcompose-signature").is_some()
            && attempted[0].header("x-delivery-token").is_some(),
        "the delivery the receiver redirected carried both credentials: {:?}",
        attempted[0].headers
    );

    let poll = format!("/executions/{execution}");
    let owed = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(Request::get(&poll).header("authorization", events_bearer()))
            .expect("the status route answers")
            .json();
        (report["deliveries"][0]["status"] == "exhausted").then_some(report)
    });
    let recorded = &owed["deliveries"][0];
    assert_eq!(
        recorded["attempts"].as_array().map(Vec::len),
        Some(2),
        "a `3xx` is a failed attempt and the schedule ran out on it: {recorded}"
    );
    for attempt in recorded["attempts"].as_array().expect("the attempts") {
        assert_eq!(attempt["outcome"], "failed", "{recorded}");
        let detail = attempt["detail"].as_str().unwrap_or_default();
        assert!(
            detail.contains("307") && detail.contains("redirect"),
            "…and it says what the receiver asked for and was not given: {recorded}"
        );
    }

    // The execution is untouched by any of it.
    let resume_url = owed["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();
    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());
    let finished = completed(&app, &execution, |request| {
        request.header("authorization", events_bearer())
    });
    assert_eq!(finished["outputs"]["decision"], "approve", "{finished}");

    // …and the settle is redirected too, so the count below is taken once every
    // delivery this execution owes has been through its whole schedule.
    receiver.wait_for_event("settled", 2, Duration::from_secs(30));
    assert_eq!(
        elsewhere.reached(),
        0,
        "a delivery goes where `callback_allow:` admits it or nowhere: {} connection(s) \
         reached the host the receiver named",
        elsewhere.reached()
    );
}

/// The `http-events` fixture with `watched`'s allowlist entry given a **literal
/// tail** after its wildcards.
///
/// `http://127.0.0.1:*/allowed*/callback` is an ordinary Decision D127 entry —
/// `*` matches any run of characters and an author may write two — and it is the
/// shape that separates a matcher which *scans* an entry from one which compiles
/// it into a regular expression: the two wildcards give a backtracking engine a
/// split point to try at every occurrence of the literal between them, and the
/// tail is what makes every one of those tries fail.
fn fixture_with_a_two_wildcard_allowlist() -> String {
    let source = std::fs::read_to_string(harness::fixture("http-events")).expect("the fixture");
    let edited = source.replace(
        "      - \"http://127.0.0.1:*/allowed*\"",
        "      - \"http://127.0.0.1:*/allowed*/callback\"",
    );
    assert_ne!(
        edited, source,
        "`watched`'s `callback_allow:` is the entry this copy grows a tail on"
    );
    edited
}

/// A callback URL is matched in time **bounded by its length** (grammar 13.3,
/// Decision D127).
///
/// The URL comes out of the request payload, which is the whole reason D127 has
/// it matched at the delivery rather than at the start: it is
/// attacker-controlled by construction. So the matcher is part of the app's
/// exposed surface, and an entry compiled into a regular expression puts a
/// caller in charge of how long the one thread that answers every route and
/// works every delivery spends on their URL — half a megabyte of the literal
/// between two wildcards is minutes of blocked event loop per request, and a
/// handful of requests is a `serve` that answers nothing and delivers nothing
/// ever again.
///
/// Two executions say it, and the second is the assertion: one names a URL of
/// 480 KB that the entry admits nowhere, and the other names a short one it
/// admits. The short one's parking webhook arriving is the app still being
/// there — and the long one's row being `refused` is the match having really
/// run rather than having been skipped for its size.
#[test]
fn a_long_callback_url_is_refused_without_wedging_the_app() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-long-url");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-long-url") else {
        return;
    };
    let tailed = harness::Scratch::new("two-wildcard-allowlist");
    let entrypoint = tailed.path().join("main.yml");
    std::fs::write(&entrypoint, fixture_with_a_two_wildcard_allowlist())
        .expect("the scratch area is writable");
    let Some(served) = harness::serve_entrypoint_into(&project, &entrypoint, &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    // The entry's head, then sixty thousand copies of the literal between its
    // wildcards, then a character that is not its tail: every one of those
    // copies is a split point a backtracking engine has to try, and none of
    // them can succeed.
    let adversarial = format!("http://127.0.0.1:{}z", "/allowed".repeat(60_000));
    let started = app
        .send(
            Request::post("/watched")
                .json(&json!({ "topic": "long", "callback_url": adversarial }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let wedging = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let admitted = app
        .send(
            Request::post("/watched")
                .json(&json!({
                    "topic": "short",
                    "callback_url": format!("{}/allowed/deep/callback", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(admitted.status, 202, "{}", admitted.text());
    let short = admitted.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // The app is still answering, and the two-wildcard entry still admits what
    // it is written to admit: a matcher that had been made safe by refusing
    // everything would fail here.
    let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    assert_eq!(
        parked[0].header("x-agentcompose-delivery"),
        Some(format!("{short}:0").as_str()),
        "{:?}",
        parked[0].headers
    );

    let refused = harness::until(Duration::from_secs(30), || {
        let report = app
            .send(
                Request::get(format!("/executions/{wedging}"))
                    .header("authorization", events_bearer()),
            )
            .expect("the status route answers")
            .json();
        (report["deliveries"].as_array().map(Vec::len) == Some(1)).then_some(report)
    });
    let recorded = &refused["deliveries"].as_array().expect("the deliveries")[0];
    assert_eq!(recorded["status"], "refused", "{}", recorded["status"]);
    assert_eq!(recorded["event"], "parked", "{}", recorded["event"]);
    assert!(
        recorded["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("callback_allow"),
        "…and it says why: {}",
        recorded["detail"]
    );
}

/// A receiver that refuses twice is delivered to on the third attempt, under the
/// same delivery id every time (PRD resolved q35, `docs/durability.md` §3.7).
///
/// The retry schedule is normative and bounded, and the override this test runs
/// under — `AGENT_COMPOSE_CALLBACK_RETRY=0s,0s,0s` — is the diagnostic surface
/// that document names beside it, because a test cannot wait fifteen minutes to
/// find out that a schedule exists.
///
/// The **id** is what the assertion is about as much as the arrival: a retry is
/// the same delivery, so a receiver deduping on `X-AgentCompose-Delivery` sees
/// one event whatever number of requests reached it. A build that allocated a
/// fresh ordinal per attempt would deliver three events for one settle and pass
/// a test that only counted arrivals.
#[test]
fn a_delivery_two_refusals_could_not_stop_lands_on_the_third_attempt() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, mut environment) = events_environment(&provider, "events-retry");
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s,0s,0s".to_string()));
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    receiver.answer_with(&[500, 503]);
    let Some(project) = harness::scratch_project("events-retry") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json(
            "/posted",
            &json!({
                "topic": "retry",
                "callback_url": format!("{}/anywhere", receiver.base_url),
            }),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let arrived = receiver.wait_for_event("settled", 3, Duration::from_secs(30));
    let id = format!("{execution}:0");
    for attempt in &arrived {
        assert_eq!(
            attempt.header("x-agentcompose-delivery"),
            Some(id.as_str()),
            "every attempt is the same delivery: {:?}",
            attempt.headers
        );
        assert_eq!(attempt.bytes, arrived[0].bytes, "…carrying the same bytes");
    }

    let report = harness::until(Duration::from_secs(30), || {
        let held = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        (held["deliveries"][0]["status"] == "delivered").then_some(held)
    });
    let recorded = &report["deliveries"][0];
    assert_eq!(recorded["delivery_id"], id, "{recorded}");
    let attempts = recorded["attempts"].as_array().expect("the attempts");
    assert_eq!(attempts.len(), 3, "{recorded}");
    assert_eq!(attempts[0]["outcome"], "failed", "{recorded}");
    assert_eq!(attempts[1]["outcome"], "failed", "{recorded}");
    assert_eq!(attempts[2]["outcome"], "delivered", "{recorded}");
    assert!(
        attempts[0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("500"),
        "an attempt records what the receiver answered: {recorded}"
    );
}

/// A delivery no attempt lands is recorded **exhausted**, and the execution is
/// not touched (PRD resolved q35).
///
/// "A webhook is a courtesy the status route backstops, not a contract worth an
/// unbounded queue": the schedule ends, the journal says so, and the run that
/// produced its outputs still reports them. A build that failed or reopened the
/// execution over an unreachable receiver would make a composition's success
/// depend on somebody else's uptime.
#[test]
fn a_delivery_no_attempt_lands_is_recorded_exhausted_and_leaves_the_run_alone() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, mut environment) = events_environment(&provider, "events-exhausted");
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-exhausted") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json(
            "/posted",
            &json!({
                "topic": "exhausted",
                "callback_url": format!("{}/anywhere", receiver.base_url),
            }),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let report = harness::until(Duration::from_secs(30), || {
        let held = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        (held["deliveries"][0]["status"] == "exhausted").then_some(held)
    });
    let recorded = &report["deliveries"][0];
    assert_eq!(
        recorded["attempts"].as_array().map(Vec::len),
        Some(2),
        "the schedule is bounded and it ended: {recorded}"
    );
    assert_eq!(report["status"], "completed", "{report}");
    assert_eq!(report["outputs"]["noted"], "noted", "{report}");
    assert_eq!(
        receiver.of_event("settled").len(),
        2,
        "two attempts reached the receiver and neither was taken"
    );
}

/// A delivery a crash interrupted completes after a restart, under the same
/// delivery id and the same ordinal — and the resume of the recovered execution
/// still verifies against the trigger that started it
/// (`docs/durability.md` §3.7, §6.1, PRD resolved q32, q35).
///
/// The two halves are one test because they are one restart. What makes the
/// delivery resumable is the intent being journaled **before** the first
/// attempt; what makes the resume still guarded is the starting trigger being on
/// the lifecycle row. A process that kept either in memory would pass every
/// single-process test in this file and lose both on the first restart.
///
/// The schedule is what makes it deterministic rather than timed: the first
/// process is given `0s,1h`, so its one attempt fails against a receiver
/// answering `500` and the next is an hour away; the second is given `0s,0s`, so
/// the delivery it picks up is due at once. The offsets are measured from the
/// recorded intent either way, which is what a restart has to be able to read.
#[test]
fn a_delivery_a_restart_interrupted_completes_under_the_same_id() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-restart");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-restart") else {
        return;
    };

    let mut stalled = base.clone();
    stalled.push((harness::CALLBACK_RETRY.to_string(), "0s,1h".to_string()));
    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &stalled) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .send(
                Request::post("/watched")
                    .json(&json!({
                        "topic": "restart",
                        "callback_url": format!("{}/allowed", receiver.base_url),
                    }))
                    .header("authorization", events_bearer()),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // One attempt, refused. The intent is journaled — that is what this
        // arrival proves — and the next offset is an hour away, so the process
        // is killed holding a delivery nothing has taken.
        let attempt = receiver.wait_for_event("parked", 1, Duration::from_secs(30));
        assert_eq!(
            attempt[0].header("x-agentcompose-delivery"),
            Some(format!("{execution}:0").as_str())
        );
        let report = app
            .send(
                Request::get(format!("/executions/{execution}"))
                    .header("authorization", events_bearer()),
            )
            .expect("the status route answers")
            .json();
        resume_url = report["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
    }

    receiver.always(200);
    let mut prompt = base;
    prompt.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let Some(second) = harness::serve_into(&project, "http-events", &prompt) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");

    let taken = harness::until(Duration::from_secs(30), || {
        let held: Vec<harness::Delivered> = receiver
            .of_event("parked")
            .into_iter()
            .filter(|one| one.header("x-agentcompose-delivery") == Some(&format!("{execution}:0")))
            .collect();
        (held.len() >= 2).then_some(held)
    });
    assert_eq!(
        taken[1].header("x-agentcompose-ordinal"),
        Some("0"),
        "a resumed delivery is the same delivery: {:?}",
        taken[1].headers
    );
    assert_eq!(
        taken[1].header("x-agentcompose-timestamp"),
        taken[0].header("x-agentcompose-timestamp"),
        "…stamped with the instant of the intent it was recorded under"
    );
    assert_eq!(taken[1].bytes, taken[0].bytes, "…carrying the same bytes");
    assert_eq!(
        receiver.distinct("parked").len(),
        1,
        "one parking, delivered twice: {:?}",
        receiver.distinct("parked")
    );

    // …and the recovered execution's resume is still the *starting* trigger's to
    // guard, which this process learned from the journal rather than from a
    // request it never saw.
    let refused = app
        .post_json(&resume_url, &json!({ "decision": "approve" }))
        .expect("the resume route answers");
    assert_eq!(refused.status, 401, "{}", refused.text());
    let answered = harness::until(Duration::from_secs(30), || {
        let held = app
            .send(
                Request::post(&resume_url)
                    .json(&json!({ "decision": "approve" }))
                    .header("authorization", events_bearer()),
            )
            .expect("the resume route answers");
        // The recovered execution may still be replaying its way back to the
        // pause, which is a `409` that says to send it again (§6.1).
        (held.status == 202).then_some(held)
    });
    assert_eq!(answered.status, 202, "{}", answered.text());
}

/// An execution finished by `agent-compose resume` still journals the `settled`
/// webhook it owes, and a later `serve` delivers it
/// (`docs/durability.md` §3.7, §6.2, PRD resolved q35).
///
/// The hole this closes is the one the ordering rule exists for, reached by the
/// other door. `serve` journals the intent before the lifecycle row closes, so a
/// process killed mid-attempt leaves a row a later start finishes — but `serve`
/// is not the only process that closes a row. An operator finishing a parked
/// execution by hand closes it too, and a build that supplied the hook only in
/// the app would close it with no delivery row beside it: `recover` then
/// enumerates open executions and finds none, the ledger holds nothing
/// `pending`, and a caller who was handed a `202` — and by resolved q34's own
/// reasoning is not polling — is never told at all. Nothing anywhere would say
/// so, which is why this is asserted from both ends: the row after the resume,
/// and the request the next start makes.
///
/// **Recorded, not sent**, is the other half. The command exits when its run
/// does and the schedule outlives it, so the receiver hears nothing until an app
/// picks the row up.
#[test]
fn an_execution_finished_by_a_hand_resume_still_journals_the_settle_it_owes() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-hand-resume");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-hand-resume") else {
        return;
    };

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &base) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .send(
                Request::post("/watched")
                    .json(&json!({
                        "topic": "by hand",
                        "callback_url": format!("{}/allowed", receiver.base_url),
                    }))
                    .header("authorization", events_bearer()),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // Parked and told: the execution is open, its callback is on the
        // lifecycle row, and the app that started it is about to go.
        receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    }

    // The operator, at the terminal the app no longer answers for. The pause is
    // the same pause under the same wait id, and answering it here is what closes
    // the row.
    let resumed = harness::resume_answering(
        &project,
        "http-events",
        &execution,
        &base,
        &["{\"decision\": \"approve\"}"],
        harness::Answers::Closed,
    );
    resumed.succeeded();
    assert_eq!(
        resumed.outputs(),
        json!({ "decision": "approve" }),
        "{}",
        resumed.stderr()
    );

    let owed = journaled_deliveries(&project, &execution);
    assert_eq!(owed.len(), 2, "one parking and one settle: {owed:?}");
    assert_eq!(owed[1]["event"], "settled", "{owed:?}");
    assert_eq!(
        owed[1]["ordinal"], 1,
        "the ordinal ascends across kinds: {owed:?}"
    );
    assert_eq!(
        owed[1]["status"], "pending",
        "a command that exits when its run does records the webhook and leaves the \
         sending to an app: {owed:?}"
    );
    assert!(
        receiver.of_event("settled").is_empty(),
        "…and sent nothing itself: {:?}",
        receiver.of_event("settled")
    );

    let mut prompt = base;
    prompt.push((harness::CALLBACK_RETRY.to_string(), "0s".to_string()));
    let Some(second) = harness::serve_into(&project, "http-events", &prompt) else {
        return;
    };
    let delivered = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    assert_eq!(
        delivered[0].header("x-agentcompose-delivery"),
        Some(format!("{execution}:1").as_str()),
        "{:?}",
        delivered[0].headers
    );
    let body: Value = serde_json::from_slice(&delivered[0].bytes).expect("the body is JSON");
    assert_eq!(body["status"], "completed", "{body}");
    assert_eq!(body["trigger"], "watched", "{body}");
    assert_eq!(body["outputs"]["decision"], "approve", "{body}");
    // The identity the trigger declared, over the bytes a command in another
    // process serialized: a report journaled without it would be delivered to a
    // receiver that refuses it.
    let expected = harness::hmac_sha256(
        harness::credential(harness::DELIVERY_SECRET).as_bytes(),
        &delivered[0].bytes,
    );
    assert_eq!(
        delivered[0].header("x-agentcompose-signature"),
        Some(format!("sha256={expected}").as_str()),
        "{:?}",
        delivered[0].headers
    );
    drop(second);
}

/// A `pending` delivery a shorter schedule leaves no attempt for is
/// **exhausted**, not left pending for ever (`docs/durability.md` §3.7).
///
/// The offsets are measured from the intent and an attempt is owed per offset,
/// so a row picked up under a schedule with fewer offsets than it already has
/// attempts has nothing left to do. §3.7 gives a delivery two ends and staying
/// `pending` is neither: a row nothing ever ends is read and skipped at every
/// subsequent start and reported by the status route as a webhook still owed for
/// the life of the journal.
///
/// `AGENT_COMPOSE_CALLBACK_RETRY` going from `0s,1h` to `0s` is exactly the shape
/// an operator shortening a diagnostic schedule between two starts produces, and
/// the assertion is that nothing is *sent* either — an end reached by making one
/// more attempt would be a receiver hearing from a schedule that had run out.
#[test]
fn a_pending_delivery_a_shorter_schedule_leaves_no_attempt_for_is_exhausted() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-shortened");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-shortened") else {
        return;
    };

    let mut stalled = base.clone();
    stalled.push((harness::CALLBACK_RETRY.to_string(), "0s,1h".to_string()));
    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &stalled) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/posted",
                &json!({
                    "topic": "shortened",
                    "callback_url": format!("{}/anywhere", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // One attempt, refused: the row is `pending` with a single attempt on it
        // and its next offset an hour away.
        receiver.wait_for_event("settled", 1, Duration::from_secs(30));
        let finished = harness::settled(&app, &execution);
        assert_eq!(finished["status"], "completed", "{finished}");
    }
    let attempted = receiver.of_event("settled").len();

    let mut shortened = base;
    shortened.push((harness::CALLBACK_RETRY.to_string(), "0s".to_string()));
    let Some(second) = harness::serve_into(&project, "http-events", &shortened) else {
        return;
    };
    let ended = harness::until(Duration::from_secs(30), || {
        let held = harness::journal_rows(
            &project,
            &format!(
                "SELECT ordinal, event, status, attempts, detail FROM deliveries \
                 WHERE execution = '{execution}' ORDER BY ordinal ASC"
            ),
        );
        let rows = held.as_array().expect("the query answers rows").clone();
        (rows.first().map(|row| row["status"] == "exhausted") == Some(true)).then_some(rows)
    });
    assert_eq!(ended.len(), 1, "one event, one row: {ended:?}");
    assert!(
        ended[0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("already made"),
        "…and it says why it ended without one: {ended:?}"
    );
    let held: Vec<Value> =
        serde_json::from_str(ended[0]["attempts"].as_str().unwrap_or("[]")).expect("the attempts");
    assert_eq!(
        held.len(),
        1,
        "the row is ended by the schedule running out, not by a further attempt: {ended:?}"
    );
    // A schedule with nothing left in it sends nothing: the receiver answering
    // `500` would have taken another request at once had one been made.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        receiver.of_event("settled").len(),
        attempted,
        "nothing more was sent: {:?}",
        receiver.delivered()
    );
    drop(second);
}

/// One quiescence that opened several pauses is **one** `parked` delivery
/// listing all of them (PRD resolved q34).
///
/// "A parking fires **one** webhook listing every pause then open — never one
/// per pause, because a `map` over a flow with `human` nodes would spray a
/// receiver with N deliveries about one quiescence." Every other lifecycle test
/// in this file watches a flow holding exactly one pause, and a build that
/// coalesced with a timer rather than with a quiescence passes all of them: the
/// single pause is alone in its turn whatever the rule.
///
/// So the fixture staggers. Each item lingers for a different length of time
/// before it reaches its `human` node, which puts the three registrations in
/// three different turns of the event loop — the shape a debounce answers with
/// three deliveries, each listing a growing prefix of one event. What is
/// asserted is the **first** delivery's own body: a rule that waited for the
/// last branch lists three pauses in it, and one that did not lists one.
#[test]
fn one_quiescence_that_opened_many_pauses_is_one_parked_delivery() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-quiescence");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-quiescence") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(
            Request::post("/fanned")
                .json(&json!({
                    "items": [
                        { "topic": "first", "linger": "0" },
                        { "topic": "second", "linger": "0.2" },
                        { "topic": "third", "linger": "0.4" },
                    ],
                    "callback_url": format!("{}/anywhere", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(60));
    let first = &parked[0];
    assert_eq!(first.header("x-agentcompose-ordinal"), Some("0"));
    let interrupts = first.body["interrupts"]
        .as_array()
        .unwrap_or_else(|| panic!("a parking webhook carries its pauses: {:?}", first.body))
        .clone();
    assert_eq!(
        interrupts.len(),
        3,
        "one quiescence, one webhook, every pause it opened: {:?}",
        first.body
    );
    let mut asked: Vec<&str> = interrupts
        .iter()
        .map(|pause| pause["input"]["note"].as_str().expect("what was shown"))
        .collect();
    asked.sort_unstable();
    assert_eq!(
        asked,
        ["first", "second", "third"],
        "…and each carries what its own item asked: {:?}",
        first.body
    );

    // Answered through the URLs the webhook published, in the order it listed
    // them. Answering one leaves the others waiting, and no pause the webhook
    // already reported is announced a second time.
    for pause in &interrupts {
        let resume_url = pause["resume_url"].as_str().expect("a resume url");
        let answered = app
            .send(
                Request::post(resume_url)
                    .json(&json!({ "decision": "approve" }))
                    .header("authorization", events_bearer()),
            )
            .expect("the resume route answers");
        assert_eq!(answered.status, 202, "{}", answered.text());
    }

    let settled = receiver.wait_for_event("settled", 1, Duration::from_secs(60));
    assert_eq!(settled[0].header("x-agentcompose-ordinal"), Some("1"));
    assert_eq!(
        settled[0].body["outputs"]["verdicts"],
        json!(["approve", "approve", "approve"]),
        "{:?}",
        settled[0].body
    );
    assert_eq!(
        receiver.distinct("parked"),
        [format!("{execution}:0")],
        "one parking, delivered once — not one per pause: {:?}",
        receiver.distinct("parked")
    );
}

/// A **detached** delivery still running does not hold a parking's webhook back
/// (grammar 8.6 rule 7, PRD resolved q34).
///
/// Rule 7 says nothing a detached dispatch does may delay the enclosing flow
/// instance, and a *quiescence* is one of the things that can be delayed: the
/// sink of a detached dispatch to a `flow.*` runs its own nodes under this
/// execution's id, so a reading of "every unit of work is waiting on a human"
/// that counted them would answer `false` for as long as the delivery ran.
/// D118 refuses a `human` node under a detached target, so that work can never
/// park — the answer would be `false` until the delivery finished, whatever
/// that took. Concretely: a receiver subscribed to a flow that detaches a slow
/// sink is told about a pause it could have answered at once only when the
/// delivery releases, minutes later.
///
/// Nothing here is timed. `flow.escorted` runs a node that waits for the sink's
/// own line before it reaches the pause, so the parking happens with the
/// delivery **provably** in flight; the sink cannot leave its wait until this
/// test writes the file it is watching for, which it does after the webhook has
/// arrived. A build that waited for the delivery delivers no `parked` webhook
/// at all inside the budget below.
#[test]
fn a_parking_is_delivered_while_a_detached_dispatch_is_still_running() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, log, environment) = events_environment(&provider, "events-detached-parking");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-detached-parking") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .send(
            Request::post("/escorted")
                .json(&json!({
                    "topic": "escorted",
                    "topics": ["one"],
                    "callback_url": format!("{}/anywhere", receiver.base_url),
                }))
                .header("authorization", events_bearer()),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // The delivery is running: its subprocess has written its line, and the only
    // way out of the wait it is now in is a file this test has not created.
    let holding = log.with_extension("log.hold");
    let release = log.with_extension("log.released");
    wait_for_lines(&holding, 1);

    let parked = receiver.wait_for_event("parked", 1, Duration::from_secs(60));
    assert!(
        !release.exists(),
        "the parking webhook arrived while the detached delivery was still in flight, \
         and nothing has released it"
    );
    assert_eq!(parked[0].header("x-agentcompose-ordinal"), Some("0"));
    let interrupts = parked[0].body["interrupts"]
        .as_array()
        .unwrap_or_else(|| panic!("a parking webhook carries its pauses: {:?}", parked[0].body))
        .clone();
    assert_eq!(
        interrupts.len(),
        1,
        "the pause the flow parked at, and nothing the delivery opened: {:?}",
        parked[0].body
    );
    let resume_url = interrupts[0]["resume_url"]
        .as_str()
        .expect("a resume url")
        .to_string();

    // Released before the answer, because a run on its way out **does** wait for
    // its detached deliveries (`docs/durability.md` §3.2): rule 7 is about the
    // flow instance, and this is the one place a delivery with no record would
    // be a delivery made twice.
    std::fs::write(&release, "").expect("the release file is writable");
    let answered = app
        .send(
            Request::post(&resume_url)
                .json(&json!({ "decision": "approve" }))
                .header("authorization", events_bearer()),
        )
        .expect("the resume route answers");
    assert_eq!(answered.status, 202, "{}", answered.text());

    let settled = receiver.wait_for_event("settled", 1, Duration::from_secs(60));
    assert_eq!(settled[0].header("x-agentcompose-ordinal"), Some("1"));
    assert_eq!(
        settled[0].body["outputs"]["decision"],
        json!("approve"),
        "{:?}",
        settled[0].body
    );
    assert_eq!(
        receiver.distinct("parked"),
        [format!("{execution}:0")],
        "one parking, delivered once: {:?}",
        receiver.distinct("parked")
    );
}

/// A credential set to the **empty string** refuses the app at launch, naming
/// the variable (grammar 13.3, PRD resolved q32).
///
/// `src/env.ts` counts an empty variable as present, which is §4.3's own rule
/// and the right one for a `base_url:`. For a credential it is an open door: an
/// empty expected token compares equal to the empty token every anonymous caller
/// can send, so `EVENTS_TOKEN=` — an unexpanded `${TOKEN}` in a launch wrapper,
/// a blank key in an env file — would leave `/guarded` answering `202` to
/// anybody while looking guarded from the outside.
///
/// A refusal is checked rather than a `401`, because those are different
/// products: a route that answered `401` to the deployment's *own* callers would
/// be a service that is down, discovered on the first real request. This is one
/// sentence on the first start.
#[test]
fn a_credential_set_to_nothing_refuses_the_app_at_launch() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-blank-credential");
    let mut environment = base;
    for entry in &mut environment {
        if entry.0 == harness::EVENTS_TOKEN {
            entry.1 = String::new();
        }
    }
    let Some(refused) =
        harness::serve_refused_with("events-blank-credential", "http-events", &environment)
    else {
        return;
    };
    let said = String::from_utf8_lossy(&refused.stderr);
    assert_eq!(
        refused.status.code(),
        Some(2),
        "a variable the deployment has to fix is a usage error: {said}"
    );
    assert!(
        said.contains(harness::EVENTS_TOKEN),
        "…and it names what to set: {said}"
    );
    assert!(
        String::from_utf8_lossy(&refused.stdout).is_empty(),
        "nothing was served: a readiness line would mean the routes were mounted: {}",
        String::from_utf8_lossy(&refused.stdout)
    );
}

/// `AGENT_COMPOSE_CALLBACK_RETRY` set to something that is not a schedule
/// refuses the app at launch (`docs/durability.md` §3.7, Decision D50).
///
/// Both ways it can fail to be one, because only the first is obvious. A
/// duration the grammar does not spell is a typo somebody sees; an **empty
/// list** is what `AGENT_COMPOSE_CALLBACK_RETRY=$SHORT_SCHEDULE` expands to when
/// the wrapper's own variable is unset, and a build that read it as "unset"
/// would start clean on the fifteen-minute schedule while the diagnostic run it
/// was written for waits ten minutes for a retry the operator believed was a
/// second away. That is D50's failure exactly: a setting nobody read.
#[test]
fn a_callback_retry_schedule_that_is_not_one_refuses_the_app_at_launch() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-retry-refused");
    for (what, written) in [
        ("a duration the grammar does not spell", "0s,soon"),
        ("an empty list, which a variable set to nothing is", ""),
    ] {
        let mut environment = base.clone();
        environment.push((harness::CALLBACK_RETRY.to_string(), written.to_string()));
        let Some(refused) =
            harness::serve_refused_with("events-retry-refused", "http-events", &environment)
        else {
            return;
        };
        let said = String::from_utf8_lossy(&refused.stderr);
        assert_eq!(
            refused.status.code(),
            Some(2),
            "{what} is refused rather than ignored: {said}"
        );
        assert!(
            said.contains(harness::CALLBACK_RETRY),
            "…and the refusal names the variable ({what}): {said}"
        );
    }
}

/// A receiver that takes a delivery and never answers does not hold it open:
/// the attempt runs out, the schedule ends, and the row stops being `pending`
/// (`docs/durability.md` §3.7, PRD resolved q35).
///
/// "Retry is bounded … because a webhook is a courtesy the status route
/// backstops, not a contract worth an unbounded queue" — which is a claim about
/// wall-clock time as much as about the count. A socket that completes the
/// handshake and writes nothing is the ordinary shape of an unreachable
/// endpoint, and against it a `fetch` with no timeout never returns: the row
/// stays `pending` for the life of the process, is re-attempted at every later
/// start, and the status route reports an execution that finished minutes ago as
/// still owing a webhook. The sibling exhaustion test cannot see this — its
/// receiver answers `500` at once — so the black hole is what asks the question.
#[test]
fn a_receiver_that_never_answers_does_not_hold_a_delivery_open() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, mut environment) = events_environment(&provider, "events-unanswered");
    // One attempt, so what is being timed is the attempt rather than the
    // schedule around it.
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s".to_string()));
    let hole = harness::Blackhole::start().expect("a loopback socket");
    let Some(project) = harness::scratch_project("events-unanswered") else {
        return;
    };
    let Some(served) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&served.base_url).expect("a client for the generated app");

    let started = app
        .post_json(
            "/posted",
            &json!({
                "topic": "unanswered",
                "callback_url": format!("{}/anywhere", hole.base_url),
            }),
        )
        .expect("the trigger's route answers");
    assert_eq!(started.status, 202, "{}", started.text());
    let execution = started.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();

    // The budget is generous against the ten seconds one attempt waits: what is
    // being asserted is that the attempt ends at all.
    let report = harness::until(Duration::from_secs(60), || {
        let held = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        (held["deliveries"][0]["status"] == "exhausted").then_some(held)
    });
    let recorded = &report["deliveries"][0];
    assert_eq!(
        recorded["attempts"].as_array().map(Vec::len),
        Some(1),
        "the one attempt the schedule allowed ended: {recorded}"
    );
    assert!(
        recorded["attempts"][0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("did not answer"),
        "…and the journal says the receiver never answered rather than naming a class: {recorded}"
    );
    assert_eq!(report["status"], "completed", "{report}");
    assert_eq!(report["outputs"]["noted"], "noted", "{report}");
    assert!(
        hole.reached() >= 1,
        "the delivery really reached the socket that swallowed it"
    );
}

/// The statements the surgery below makes about a journal, which are the shape
/// of the file a build **before** the delivery ledger wrote
/// (`docs/durability.md` §11.2).
///
/// Two changes, and each fails loudly if this build's journal is not what it is
/// described as: `DROP TABLE deliveries` without `IF EXISTS` refuses a file that
/// has no such table, and the `INSERT … SELECT` names `executions.callback`, so
/// a build that had never added the column could not run this either.
const JOURNAL_BEFORE_DELIVERIES: &str = "\
DROP TABLE deliveries;
CREATE TABLE before_deliveries (
  id              TEXT PRIMARY KEY,
  flow            TEXT NOT NULL,
  trigger_kind    TEXT NOT NULL,
  inputs          TEXT NOT NULL,
  session_key     TEXT NOT NULL,
  status          TEXT NOT NULL,
  journal_version INTEGER NOT NULL,
  started_at      TEXT NOT NULL,
  ended_at        TEXT,
  error           TEXT
);
INSERT INTO before_deliveries
  SELECT id, flow, trigger_kind, inputs, session_key, status, journal_version, started_at, ended_at, error
  FROM executions;
DROP TABLE executions;
ALTER TABLE before_deliveries RENAME TO executions;
";

/// A journal written before the delivery ledger existed opens under this build,
/// replays what it holds, and takes this build's deliveries
/// (`docs/durability.md` §11.2).
///
/// §11.2 keeps `JOURNAL_VERSION` where it is by arguing that the ledger arrived
/// as a compatible change: "A journal written before it opens unchanged — the
/// table is created on first open, as `CREATE TABLE IF NOT EXISTS`". That
/// argument is the whole reason an execution parked in an older file is still
/// resumable, and until this test it was made only in a comment — every test in
/// the suite creates its journal fresh under the current schema, so a `NOT NULL`
/// column added to `deliveries` without the `PRAGMA table_info` probe the
/// migrations beside it use would leave `cargo test` green and every deployed
/// journal unable to record a webhook.
///
/// There is no older build in the tree to write the file, so the file is made:
/// a real run's journal with the `deliveries` table dropped and the lifecycle
/// row rebuilt without its `callback` column, which is exactly the two things
/// this change added to the physical schema. What is then asserted is the three
/// halves of the claim — it opens, the execution it holds replays and can still
/// be answered, and this build writes its own ledger into the same file.
#[test]
fn a_journal_written_before_the_delivery_ledger_opens_and_serves_under_this_build() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-old-journal");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-old-journal") else {
        return;
    };

    let execution;
    let resume_url;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        // The no-auth trigger, so what the second process has to do with this
        // execution is recovery rather than authentication.
        let started = app
            .post_json("/open", &json!({ "topic": "before the ledger" }))
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        let parked = harness::settled(&app, &execution);
        assert_eq!(parked["status"], "interrupted", "{parked}");
        resume_url = parked["interrupts"].as_array().expect("the pauses")[0]["resume_url"]
            .as_str()
            .expect("a resume url")
            .to_string();
    }

    harness::journal_sql(&project, JOURNAL_BEFORE_DELIVERIES);

    let Some(second) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");

    // It opened, and what it held came back: the execution is on the board under
    // the wait id the process that died published, which is what makes the
    // `resume_url` that process handed out still work.
    let recovered = harness::until(Duration::from_secs(30), || {
        let held = app
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers")
            .json();
        (held["status"] == "interrupted").then_some(held)
    });
    assert_eq!(recovered["execution_id"], execution, "{recovered}");
    let answered = harness::until(Duration::from_secs(30), || {
        let held = app
            .post_json(&resume_url, &json!({ "decision": "approve" }))
            .expect("the resume route answers");
        // A recovered execution may still be replaying its way back to the
        // pause, which is a `409` that says to send it again (§6.1).
        (held.status == 202).then_some(held)
    });
    assert_eq!(answered.status, 202, "{}", answered.text());
    let finished = harness::settled(&app, &execution);
    assert_eq!(finished["status"], "completed", "{finished}");
    assert_eq!(finished["outputs"]["decision"], "approve", "{finished}");

    // …and the ledger this build wants is back in the same file: a new
    // execution's webhook is journaled and delivered out of a journal that was
    // created without one.
    let posted = app
        .post_json(
            "/posted",
            &json!({
                "topic": "after the ledger",
                "callback_url": format!("{}/anywhere", receiver.base_url),
            }),
        )
        .expect("the trigger's route answers");
    assert_eq!(posted.status, 202, "{}", posted.text());
    let fresh = posted.json()["execution_id"]
        .as_str()
        .expect("an execution id")
        .to_string();
    let delivered = receiver.wait_for_event("settled", 1, Duration::from_secs(30));
    assert_eq!(
        delivered[0].header("x-agentcompose-delivery"),
        Some(format!("{fresh}:0").as_str()),
        "{:?}",
        delivered[0].headers
    );
    let report = harness::until(Duration::from_secs(30), || {
        let held = app
            .get(&format!("/executions/{fresh}"))
            .expect("the status route answers")
            .json();
        (held["deliveries"][0]["status"] == "delivered").then_some(held)
    });
    assert_eq!(report["deliveries"][0]["ordinal"], 0, "{report}");
}

/// A lifecycle row this build cannot read, under a delivery it still owes.
///
/// `inputs` is the column taken away because reading it is what fails: the
/// journal's queries are `SELECT *`, so a missing column is a missing *field*,
/// and the one field the row's reader parses rather than copies is this one.
/// Every other column would be read as `"undefined"` and hurt nothing.
///
/// The row is kept rather than deleted, because a row that is **not there** is
/// an answer the picker-up already has a posture for (the execution's trigger
/// cannot be named, so the delivery stays pending). What has no posture until
/// one is written is a read that *throws*, which is what any transient journal
/// failure looks like from inside the loop.
///
/// The delivery's own `trigger` is cleared beside it, and that is what leaves
/// the lifecycle row as the only thing that can name this delivery's trigger: a
/// row that names its own is picked up without reading an execution at all
/// (`docs/durability.md` §3.7). So this is also the older shape of a delivery —
/// a row written before the ledger recorded that field — met by the start that
/// cannot read the execution behind it.
const JOURNAL_WITH_AN_UNREADABLE_EXECUTION: &str = "\
UPDATE deliveries SET trigger_kind = NULL;
CREATE TABLE unreadable (
  id              TEXT PRIMARY KEY,
  flow            TEXT NOT NULL,
  trigger_kind    TEXT NOT NULL,
  session_key     TEXT NOT NULL,
  callback        TEXT,
  status          TEXT NOT NULL,
  journal_version INTEGER NOT NULL,
  started_at      TEXT NOT NULL,
  ended_at        TEXT,
  error           TEXT
);
INSERT INTO unreadable
  SELECT id, flow, trigger_kind, session_key, callback, status, journal_version, started_at, ended_at, error
  FROM executions;
DROP TABLE executions;
ALTER TABLE unreadable RENAME TO executions;
";

/// A journal read that fails while owed deliveries are being picked up leaves
/// **one delivery** owed, not a deployment that will not start
/// (`docs/durability.md` §3.7, §6.1).
///
/// Picking the owed webhooks up happens in the `onReady` hook, so a rejection
/// out of that walk is an `app.listen()` that rejects: the process exits without
/// binding a port, and the executions, routes and journal it was going to serve
/// go with it. The reads in that walk can fail for reasons that have nothing to
/// do with the deployment's health — §2 and §12 both make a second process
/// contending the file a state a healthy project reaches, and a `SELECT` that
/// waits out `busy_timeout` throws — so a webhook, which §3.7 calls "a courtesy
/// the status route backstops", would take down a service that owes one.
///
/// A contended file cannot be scripted, so the read is made to fail the way a
/// contended one does: the delivery is put back into the shape that has to read
/// an execution at all — the older row, carrying no trigger of its own — and the
/// lifecycle row it belongs to is rebuilt without the column its reader parses.
/// What is asserted is that the app serves — a `401` from a guarded route is a
/// route answering — and that the row is left exactly as it was found, for a
/// start that can read it.
#[test]
fn a_delivery_whose_execution_cannot_be_read_leaves_the_app_serving() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-unreadable-row");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    // Refused, so the delivery is still owed when the process ends: one attempt
    // is made at `0s` and the next is an hour away.
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-unreadable-row") else {
        return;
    };
    let mut environment = base;
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s,1h".to_string()));

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/posted",
                &json!({
                    "topic": "unreadable",
                    "callback_url": format!("{}/anywhere", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        receiver.wait_for_event("settled", 1, Duration::from_secs(30));
        let finished = harness::settled(&app, &execution);
        assert_eq!(finished["status"], "completed", "{finished}");
    }

    harness::journal_sql(&project, JOURNAL_WITH_AN_UNREADABLE_EXECUTION);

    // The start enumerates the owed delivery, cannot read the execution it
    // reports on — and binds its port anyway.
    let Some(second) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");
    let refused = app
        .post_json("/guarded", &json!({ "topic": "unreadable" }))
        .expect("the trigger's route answers");
    assert_eq!(
        refused.status,
        401,
        "the app is serving its routes: {}",
        refused.text()
    );

    let held = journaled_deliveries(&project, &execution);
    assert_eq!(
        held[0]["status"], "pending",
        "the row is left as it was found, for a start that can read it: {held:?}"
    );
    drop(second);
}

/// A delivery whose execution the journal does **not** hold is still finished,
/// because the row names its own trigger (`docs/durability.md` §3.7).
///
/// The shape this is about is the one settle the `closing` hook is never reached
/// for: a run that failed **before** it was journaled at all — its recorded
/// inputs refused by a narrowed `inputs:`, a `session_key:` the composition has
/// since started requiring — leaves no lifecycle row, so no start will ever
/// recover it and the caller holding a `202` is owed the failure by the process
/// that saw it. That delivery is journaled like any other, and a start that read
/// its trigger off the execution would find no execution, name no trigger, and
/// neither send the row nor end it: it would be read, logged and skipped at
/// every start for the life of the journal, which is neither of the two ends
/// §3.7 gives a delivery.
///
/// A run that fails that early cannot be scripted, so the state one leaves is
/// made instead: an ordinary settle is journaled and left owed by a receiver
/// answering `500`, and then the lifecycle row is taken out from under it. What
/// the restart meets is exactly what that run leaves behind — a `pending`
/// delivery about an execution the journal does not hold — and the delivery
/// still lands, under the id it was allocated.
#[test]
fn a_settle_journaled_for_an_execution_the_journal_never_held_is_still_delivered() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-orphan-settle");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    // One attempt, refused, and the next an hour away: the process is killed
    // holding a delivery nothing has taken.
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-orphan-settle") else {
        return;
    };

    let mut stalled = base.clone();
    stalled.push((harness::CALLBACK_RETRY.to_string(), "0s,1h".to_string()));
    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &stalled) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .post_json(
                "/posted",
                &json!({
                    "topic": "orphaned",
                    "callback_url": format!("{}/anywhere", receiver.base_url),
                }),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        receiver.wait_for_event("settled", 1, Duration::from_secs(30));
        let finished = harness::settled(&app, &execution);
        assert_eq!(finished["status"], "completed", "{finished}");
    }

    // The lifecycle row goes and the delivery it is about stays: the file now
    // holds what a settle journaled for a never-journaled run leaves behind.
    harness::journal_sql(
        &project,
        &format!("DELETE FROM executions WHERE id = '{execution}';"),
    );
    let owed = journaled_deliveries(&project, &execution);
    assert_eq!(owed.len(), 1, "one settle, still owed: {owed:?}");
    assert_eq!(owed[0]["status"], "pending", "{owed:?}");

    receiver.always(200);
    let mut prompt = base;
    prompt.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let Some(second) = harness::serve_into(&project, "http-events", &prompt) else {
        return;
    };
    let app = Client::new(&second.base_url).expect("a client for the generated app");

    let taken = harness::until(Duration::from_secs(30), || {
        let held: Vec<harness::Delivered> = receiver
            .of_event("settled")
            .into_iter()
            .filter(|one| one.header("x-agentcompose-delivery") == Some(&format!("{execution}:0")))
            .collect();
        (held.len() >= 2).then_some(held)
    });
    assert_eq!(
        taken[1].bytes, taken[0].bytes,
        "the delivery a start with no execution to read picked up is the same delivery"
    );
    let ended = harness::until(Duration::from_secs(30), || {
        let held = journaled_deliveries(&project, &execution);
        (held.first().map(|row| row["status"] == "delivered") == Some(true)).then_some(held)
    });
    assert_eq!(ended[0]["event"], "settled", "{ended:?}");

    // …and the execution really is one nothing in this file describes: the row
    // a build would have read the trigger off is gone.
    let unknown = app
        .get(&format!("/executions/{execution}"))
        .expect("the status route answers");
    assert_eq!(unknown.status, 404, "{}", unknown.text());
    drop(second);
}

/// The `http-events` fixture with one trigger renamed, so a build meets an
/// execution its journal says was started by a trigger it does not declare.
///
/// The flows are untouched, which is what keeps the difference to the one thing
/// under test: recovery replays the execution exactly, and only the *trigger*
/// the lifecycle row names is missing.
fn fixture_without_the_fanned_trigger() -> String {
    let source = std::fs::read_to_string(harness::fixture("http-events")).expect("the fixture");
    let edited = source.replace("\n  fanned:\n", "\n  unfanned:\n");
    assert_ne!(
        edited, source,
        "`fanned` is the trigger this copy renames out of the composition"
    );
    edited
}

/// The same fixture with `watched`'s allowlist narrowed to admit nothing a test
/// receiver binds.
fn fixture_with_a_narrower_allowlist() -> String {
    let source = std::fs::read_to_string(harness::fixture("http-events")).expect("the fixture");
    let edited = source.replace(
        "      - \"http://127.0.0.1:*/allowed*\"",
        "      - \"http://127.0.0.1:*/nowhere*\"",
    );
    assert_ne!(
        edited, source,
        "`watched`'s `callback_allow:` is what this copy narrows"
    );
    edited
}

/// Every delivery row one execution has, as the journal holds it.
fn journaled_deliveries(project: &std::path::Path, execution: &str) -> Vec<Value> {
    harness::journal_rows(
        project,
        &format!(
            "SELECT ordinal, event, status, url, detail FROM deliveries \
             WHERE execution = '{execution}' ORDER BY ordinal ASC"
        ),
    )
    .as_array()
    .expect("the query answers rows")
    .clone()
}

/// A lifecycle event a build cannot deliver is **journaled** and finished by a
/// build that can (`docs/durability.md` §3.7).
///
/// The document's promise about a delivery whose trigger the composition no
/// longer declares is that "the row stays `pending` for a build that declares
/// the trigger, and the reason is written on stderr" — which is a promise about
/// a row, and so a promise that a row exists. A build that recorded nothing
/// would make the event both unrecoverable and invisible: no start could find
/// it, and the status route would show a parking nobody was ever told about.
///
/// Reaching it takes three processes, because a lifecycle row only names a
/// trigger the build lacks after the composition has moved under it. The first
/// is killed while its fan-out is still lingering, so the execution is open and
/// has announced nothing; the second is the same composition with the trigger
/// renamed, which recovers the execution, parks it, and has a webhook to make
/// and no identity to make it under; the third is the original again, which
/// finishes what the second wrote down.
///
/// What is asserted between them is the journal itself, because the second
/// build refuses that execution's status route on purpose (grammar 13.3: the
/// guard is the starting trigger's, and this build cannot read it) — which is
/// exactly the position a reader debugging one would be in.
#[test]
fn a_delivery_journaled_without_its_trigger_is_finished_by_a_build_that_declares_it() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, environment) = events_environment(&provider, "events-moved-trigger");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    let Some(project) = harness::scratch_project("events-moved-trigger") else {
        return;
    };
    let callback = format!("{}/anywhere", receiver.base_url);

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        // Long enough that the process is killed while the item is still
        // lingering: the execution is open, and no pause has been announced.
        let started = app
            .send(
                Request::post("/fanned")
                    .json(&json!({
                        "items": [{ "topic": "moved", "linger": "5" }],
                        "callback_url": callback,
                    }))
                    .header("authorization", events_bearer()),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // The lifecycle row is on the disk before the process goes, so what the
        // next build meets is an execution rather than nothing.
        harness::until(Duration::from_secs(30), || {
            harness::journal_holds(&project, &[&execution]).then_some(())
        });
        // …and it has announced nothing, which is what the linger buys: a
        // parking this process delivered would leave the next build with a
        // question already asked and nothing to journal.
        assert!(
            receiver.delivered().is_empty(),
            "the process was killed before its fan-out parked: {:?}",
            receiver.delivered()
        );
    }

    let moved = harness::Scratch::new("moved-trigger");
    let entrypoint = moved.path().join("main.yml");
    std::fs::write(&entrypoint, fixture_without_the_fanned_trigger())
        .expect("the scratch area is writable");

    {
        let Some(second) = harness::serve_entrypoint_into(&project, &entrypoint, &environment)
        else {
            return;
        };
        // The row this build writes and does not send. It is the only thing this
        // process publishes about the execution — its status route refuses,
        // because the trigger that guarded it is not here to say who may poll.
        let pending = harness::until(Duration::from_secs(60), || {
            let held = journaled_deliveries(&project, &execution);
            (!held.is_empty()).then_some(held)
        });
        assert_eq!(pending.len(), 1, "{pending:?}");
        assert_eq!(pending[0]["status"], "pending", "{pending:?}");
        assert_eq!(pending[0]["event"], "parked", "{pending:?}");
        assert_eq!(pending[0]["ordinal"], 0, "{pending:?}");
        assert_eq!(pending[0]["url"], callback, "{pending:?}");
        let refused = Client::new(&second.base_url).expect("a client for the generated app");
        let polled = refused
            .get(&format!("/executions/{execution}"))
            .expect("the status route answers");
        assert_eq!(
            polled.status,
            401,
            "an execution whose starting trigger this build cannot read is not one it \
             answers about: {}",
            polled.text()
        );
        // …and nothing was sent under an identity this build could not promise.
        assert!(
            receiver.of_event("parked").is_empty(),
            "{:?}",
            receiver.delivered()
        );
    }

    // The composition is put back, and the row is finished under the delivery id
    // the second build allocated for it.
    let Some(third) = harness::serve_into(&project, "http-events", &environment) else {
        return;
    };
    let delivered = receiver.wait_for_event("parked", 1, Duration::from_secs(60));
    assert_eq!(
        delivered[0].header("x-agentcompose-delivery"),
        Some(format!("{execution}:0").as_str()),
        "the row the build that could not send it wrote is the delivery that lands: {:?}",
        delivered[0].headers
    );
    let finished = harness::until(Duration::from_secs(60), || {
        let held = journaled_deliveries(&project, &execution);
        (held.first().map(|row| row["status"] == "delivered") == Some(true)).then_some(held)
    });
    assert_eq!(finished.len(), 1, "one event, one row: {finished:?}");
    drop(third);
}

/// A pending delivery the trigger's allowlist no longer admits is **refused**
/// rather than sent (grammar 13.3, Decision D127, `docs/durability.md` §3.7).
///
/// `callback_allow:` is matched when the URL is read, and a row a restart picks
/// up is a URL being read again — by a build whose list may not be the list the
/// row was written under. The URL came out of a request payload and is
/// attacker-controlled by construction, so a resumed delivery that skipped the
/// match would POST it to a host the deployment has since said it may not reach,
/// which is the one thing the allowlist exists to prevent.
///
/// The refusal is written **onto the row**, not opened as a second event: the
/// ordinal counts an execution's lifecycle events, and the event did not happen
/// twice.
#[test]
fn a_pending_delivery_the_allowlist_no_longer_admits_is_refused_rather_than_sent() {
    let provider = MockProvider::start().expect("a loopback port");
    let (_shims, _log, base) = events_environment(&provider, "events-narrowed-allowlist");
    let receiver = harness::Receiver::start().expect("a loopback receiver");
    receiver.always(500);
    let Some(project) = harness::scratch_project("events-narrowed-allowlist") else {
        return;
    };
    // One attempt now and the next an hour away, so the row is still `pending`
    // when the process goes — the shape the restart test uses for the same
    // reason.
    let mut environment = base.clone();
    environment.push((harness::CALLBACK_RETRY.to_string(), "0s,1h".to_string()));
    let callback = format!("{}/allowed", receiver.base_url);

    let execution;
    {
        let Some(first) = harness::serve_into(&project, "http-events", &environment) else {
            return;
        };
        let app = Client::new(&first.base_url).expect("a client for the generated app");
        let started = app
            .send(
                Request::post("/watched")
                    .json(&json!({ "topic": "narrowed", "callback_url": callback }))
                    .header("authorization", events_bearer()),
            )
            .expect("the trigger's route answers");
        assert_eq!(started.status, 202, "{}", started.text());
        execution = started.json()["execution_id"]
            .as_str()
            .expect("an execution id")
            .to_string();
        // One attempt, refused by the receiver: the row is journaled, pending,
        // and its next offset is an hour away.
        receiver.wait_for_event("parked", 1, Duration::from_secs(30));
    }
    let attempted = receiver.of_event("parked").len();

    let narrowed = harness::Scratch::new("narrowed-allowlist");
    let entrypoint = narrowed.path().join("main.yml");
    std::fs::write(&entrypoint, fixture_with_a_narrower_allowlist())
        .expect("the scratch area is writable");

    let mut prompt = base;
    prompt.push((harness::CALLBACK_RETRY.to_string(), "0s,0s".to_string()));
    let Some(second) = harness::serve_entrypoint_into(&project, &entrypoint, &prompt) else {
        return;
    };
    let refused = harness::until(Duration::from_secs(60), || {
        let held = journaled_deliveries(&project, &execution);
        (held.first().map(|row| row["status"] == "refused") == Some(true)).then_some(held)
    });
    assert_eq!(refused.len(), 1, "one event, one row: {refused:?}");
    assert_eq!(refused[0]["ordinal"], 0, "{refused:?}");
    assert!(
        refused[0]["detail"]
            .as_str()
            .unwrap_or_default()
            .contains("callback_allow"),
        "…and it says why: {refused:?}"
    );
    // The schedule this process runs under would have re-attempted it at once,
    // so a receiver that saw no more requests saw a delivery that was refused
    // rather than one that is merely late.
    std::thread::sleep(Duration::from_secs(2));
    assert_eq!(
        receiver.of_event("parked").len(),
        attempted,
        "nothing more was sent to a URL this build's allowlist admits nowhere: {:?}",
        receiver.delivered()
    );
    drop(second);
}
