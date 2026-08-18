//! The Anthropic Messages surface, over a real socket.
//!
//! The unit tests in `src/anthropic.rs` pin what the surface decides; these pin
//! what a client on the other end of a connection actually receives — status,
//! headers, and body — because that is what the Anthropic SDK inside a compiled
//! graph will parse. A request here is written the way generated code will write
//! it: the agent's output schema as a forced tool (PRD 5.2), its `tools:` and
//! `stores:` entries as the tool surface (grammar 5.4, 11.5).

use std::time::Duration;

use mock_provider::{
    Client, HARNESS_HEADER, HARNESS_STATUS, MockProvider, Outcome, REFUSED_INVALID,
    REFUSED_UNSCRIPTED, Request, Script, ToolCall,
};
use serde_json::{Value, json};

/// The model id every fixture in this file scripts against.
const MODEL: &str = "claude-sonnet-4-6";

/// A request shaped like the first model call of a compiled agent node: a system
/// prompt, the node's bound input as the user turn, and the output schema as a
/// forced tool.
fn structured_request(tools: Value, forced: &str) -> Value {
    json!({
        "model": MODEL,
        "max_tokens": 4096,
        "system": "You are a meticulous technical reviewer.",
        "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\",\"draft\":\"a draft\"}" }],
        "tools": tools,
        "tool_choice": { "type": "tool", "name": forced },
    })
}

fn output_schema_tool() -> Value {
    json!([{
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
    }])
}

fn send(client: &Client, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post("/v1/messages").anthropic_auth().json(body))
        .expect("the mock provider answers")
}

/// The load-bearing path: a scripted structured output comes back as a forced
/// `tool_use` block, which is what `withStructuredOutput` reads.
#[test]
fn a_scripted_structured_output_arrives_as_forced_tool_use() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let response = send(
        &provider.client(),
        &structured_request(output_schema_tool(), "reviewer_output"),
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.header("content-type"), Some("application/json"));

    let body = response.json();
    assert_eq!(body["type"], "message");
    assert_eq!(body["role"], "assistant");
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["stop_reason"], "tool_use");
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "reviewer_output");
    assert_eq!(body["content"][0]["input"]["verdict"], "approve");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 1);
    assert!(recorded[0].is_valid());
    assert_eq!(recorded[0].tools, ["reviewer_output"]);
    assert_eq!(
        recorded[0]
            .structured_output
            .as_ref()
            .map(|output| output.name()),
        Some("reviewer_output")
    );
    assert!(provider.snapshot().is_drained(), "the script was consumed");
}

/// The tool surface a request carries is observable, in request order.
///
/// This is what a test asserting about synthesized store tools reads (grammar
/// 11.5) — including an `agent_access: read` narrowing, if the key survives:
/// `agent_access` is PRD §10's first open question, so what is pinned here is
/// the *observability*, which the acceptance suite needs either way.
#[test]
fn the_tool_surface_of_a_request_is_recorded_in_order() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("thinking")));

    let tools = json!([
        { "name": "web_search", "input_schema": { "type": "object" } },
        { "name": "docs_search", "input_schema": { "type": "object" } },
        { "name": "reviewer_output", "input_schema": { "type": "object" } },
    ]);
    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "go" }],
            "tools": tools,
        }),
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        provider.requests()[0].tools,
        ["web_search", "docs_search", "reviewer_output"],
        "a `docs_upsert` appearing beside `docs_search` is what a read-only \
         store attachment failing to narrow the surface would look like"
    );
}

/// A tool call, then the result, then the answer: the intra-agent tool loop
/// (grammar 5, `max_tool_iterations`), scripted end to end.
#[test]
fn a_tool_loop_runs_over_two_calls() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(
            MODEL,
            Outcome::tool_calls(vec![ToolCall::new("web_search", json!({ "query": "it" }))]),
        ),
        Script::new(
            MODEL,
            Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
        ),
    ]);
    let client = provider.client();
    let tools = json!([
        { "name": "web_search", "input_schema": { "type": "object" } },
        { "name": "reviewer_output", "input_schema": { "type": "object" } },
    ]);

    let first = send(
        &client,
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "review it" }],
            "tools": tools,
        }),
    )
    .json();
    assert_eq!(first["stop_reason"], "tool_use");
    let call = &first["content"][0];
    assert_eq!(call["name"], "web_search");
    let call_id = call["id"].as_str().expect("a tool use id").to_string();

    // The second turn is what the tool loop must build: the assistant's own
    // block echoed back, then a `tool_result` leading the user turn.
    let second = send(
        &client,
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [
                { "role": "user", "content": "review it" },
                { "role": "assistant", "content": [call] },
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": call_id, "content": "[]" },
                ]},
            ],
            "tools": tools,
            "tool_choice": { "type": "tool", "name": "reviewer_output" },
        }),
    );
    assert_eq!(second.status, 200);
    assert_eq!(second.json()["content"][0]["input"]["verdict"], "approve");

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded
            .iter()
            .all(mock_provider::RecordedRequest::is_valid)
    );
    assert_eq!(recorded[0].sequence, 1);
    assert_eq!(recorded[1].sequence, 2);
}

/// A tool loop that appends its results in the wrong place is refused, the
/// refusal is recorded with the field that is wrong, and the scripted answer is
/// still waiting for the call that was supposed to get it.
#[test]
fn a_malformed_tool_loop_is_refused_and_recorded() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {} },
                ]},
                { "role": "user", "content": [
                    { "type": "text", "text": "here" },
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "[]" },
                ]},
            ],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }),
    );

    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = response.json();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("at the beginning of this message"),
        "{body}"
    );

    let recorded = provider.requests();
    assert_eq!(recorded[0].served, "rejected");
    assert_eq!(recorded[0].failures().len(), 1);
    assert_eq!(recorded[0].failures()[0].pointer, "messages.2.content.1");
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "a refused request consumes nothing"
    );
}

/// An assistant turn with an **empty content list** is refused, which is the
/// shape a tool loop produces when it replays an answer that carried nothing.
///
/// The Messages API refuses `content: []` (`List should have at least 1 item`),
/// and a loop that rebuilt its assistant turn out of the text and tool calls it
/// read would send exactly that after a `max_tokens` cut. The compiler's runtime
/// stops the node on such an answer instead — but only this side says the shape
/// is refused at all, and a mock that quietly accepted it would let that
/// regression back in without a failing test.
#[test]
fn an_assistant_turn_with_no_content_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": [] },
            ],
        }),
    );

    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = response.json();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("messages.1.content: List should have at least 1 item"),
        "{body}"
    );
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "a refused request consumes nothing"
    );
}

/// A call with no `x-api-key` is refused **401 `authentication_error`**, the
/// status the Messages API answers and the one `@anthropic-ai/sdk` raises
/// `AuthenticationError` from.
///
/// The harness needs no API *keys* (WIRE-NOTES §12 — values are never compared),
/// but the header still has to be there, and a missing one is an authentication
/// failure rather than a malformed request: answering 400 would teach generated
/// code to classify the two the same way. 401 is outside PRD 5.9's failover set
/// and outside the SDK's retry set, so nothing else changes shape.
#[test]
fn a_request_without_an_api_key_is_refused_as_authentication() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .header("anthropic-version", "2023-06-01")
                .json(&json!({
                    "model": MODEL,
                    "max_tokens": 1024,
                    "messages": [{ "role": "user", "content": "go" }],
                })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 401);
    let body = response.json();
    assert_eq!(body["type"], "error");
    assert_eq!(body["error"]["type"], "authentication_error");
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));

    let recorded = provider.requests();
    assert!(!recorded[0].is_valid());
    assert_eq!(recorded[0].failures()[0].pointer, "headers.x-api-key");
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "an unauthenticated call consumes nothing"
    );

    // Authentication is settled before the body is: a request that is both
    // unauthenticated and malformed is the 401, naming the credential only.
    let response = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .header("anthropic-version", "2023-06-01")
                .json(&json!({ "model": MODEL })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 401);
    let message = response.json()["error"]["message"]
        .as_str()
        .expect("a message")
        .to_string();
    assert!(message.contains("x-api-key"), "{message}");
    assert!(!message.contains("max_tokens"), "{message}");
    assert_eq!(
        provider.requests()[1]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["headers.x-api-key", "max_tokens", "messages"],
        "the transcript still records everything that was wrong"
    );

    // …and a request that carries its key is refused at 400 as it always was.
    let response = send(&provider.client(), &json!({ "model": MODEL }));
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["type"], "invalid_request_error");
}

/// Every failover condition PRD 5.9 names, on the wire, with the status and body
/// the SDK classifies from.
#[test]
fn the_failover_conditions_are_scriptable() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client().with_timeout(Duration::from_millis(500));
    let request = json!({
        "model": MODEL,
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "go" }],
    });

    provider.enqueue(Script::new(MODEL, Outcome::rate_limit()));
    let response = send(&client, &request);
    assert_eq!(response.status, 429);
    assert_eq!(response.json()["error"]["type"], "rate_limit_error");
    assert_eq!(response.header("retry-after"), Some("1"));

    provider.enqueue(Script::new(MODEL, Outcome::overloaded()));
    let response = send(&client, &request);
    assert_eq!(response.status, 529);
    assert_eq!(response.json()["error"]["type"], "overloaded_error");

    provider.enqueue(Script::new(MODEL, Outcome::server_error()));
    assert_eq!(send(&client, &request).status, 500);

    // A timeout is no answer at all: the client's own budget is what ends it.
    provider.enqueue(Script::new(
        MODEL,
        Outcome::timeout(Duration::from_millis(5_000)),
    ));
    let error = client
        .send(
            Request::post("/v1/messages")
                .anthropic_auth()
                .json(&request),
        )
        .expect_err("a scripted timeout never answers");
    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);

    let recorded = provider.requests();
    let served: Vec<&str> = recorded
        .iter()
        .map(|request| request.served.as_str())
        .collect();
    assert_eq!(
        served,
        [
            "failure.rate_limit",
            "failure.overloaded",
            "failure.server_error",
            "failure.timeout",
        ]
    );
}

/// An unscripted call is refused loudly, naming the model and the queue, at a
/// status no failover condition claims — so a graph cannot fail over past a
/// missing script and report success.
#[test]
fn an_unscripted_call_names_the_model_and_the_empty_queue() {
    let provider = MockProvider::start().expect("a port");
    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "go" }],
        }),
    );

    assert_eq!(response.status, HARNESS_STATUS);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_UNSCRIPTED));
    let message = response.json()["error"]["message"]
        .as_str()
        .expect("a message")
        .to_string();
    assert!(message.contains(MODEL), "{message}");
    assert!(message.contains("no scripted outcome"), "{message}");
    assert!(message.contains("/_mock/enqueue"), "{message}");
    assert_eq!(provider.requests()[0].served, "unscripted");
    assert_eq!(provider.snapshot().unscripted, 1);
}

/// A scripted `text` answer to a request that forced a tool is refused as a
/// harness bug, in the harness's own status — because the Messages API cannot
/// answer that way, and a codegen PR that tested its structured-output parser
/// against such an answer would be testing against an input no provider sends.
#[test]
fn a_text_reply_to_a_forced_tool_is_refused_as_a_script_mismatch() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::text("I am not going to call the tool"),
    ));

    let response = send(
        &provider.client(),
        &structured_request(output_schema_tool(), "reviewer_output"),
    );
    assert_eq!(response.status, HARNESS_STATUS);
    assert_eq!(
        response.header(HARNESS_HEADER),
        Some(mock_provider::REFUSED_MISMATCH)
    );
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("tool_choice"),
        "the refusal names what the request pinned: {}",
        response.text()
    );

    // The transcript records what the client received, not what the queue handed
    // over: no reply was sent, and a `served` reading `reply.text` would tell a
    // test the opposite. The snapshot says the same thing — in an e2e run the 422
    // goes to the generated process, so a drained snapshot here would leave the
    // Rust side with a graph that failed for no visible reason.
    let recorded = &provider.requests()[0];
    assert_eq!(recorded.served, mock_provider::REFUSED_MISMATCH);
    assert!(recorded.was_refused());
    let snapshot = provider.snapshot();
    assert_eq!(snapshot.refused, 1);
    assert!(
        !snapshot.is_drained(),
        "a refused call is not a clean run: {snapshot:?}"
    );
}

/// A scripted call to a tool the request *offered* but did not *pin* is refused
/// too — the shape between the two refusals above, and the one an agent fixture
/// makes reachable: an agent with a `tools:` list and a pinned output tool
/// offers both, and only one of them can be called.
#[test]
fn a_tool_call_beside_the_pinned_tool_is_refused_as_a_script_mismatch() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
    ));

    let tools = json!([
        { "name": "lookup", "input_schema": { "type": "object" } },
        { "name": "reviewer_output", "input_schema": { "type": "object" } },
    ]);
    let response = send(
        &provider.client(),
        &structured_request(tools.clone(), "reviewer_output"),
    );
    assert_eq!(response.status, HARNESS_STATUS);
    assert_eq!(
        response.header(HARNESS_HEADER),
        Some(mock_provider::REFUSED_MISMATCH)
    );
    let message = response.json()["error"]["message"]
        .as_str()
        .expect("a message")
        .to_string();
    assert!(message.contains("lookup"), "{message}");
    assert!(message.contains("reviewer_output"), "{message}");

    // The same call against the same tool surface, with nothing pinned, is the
    // tool loop's own first turn — and it is served.
    provider.enqueue(Script::new(
        MODEL,
        Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "a fact" }))]),
    ));
    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "review it" }],
            "tools": tools,
        }),
    );
    assert_eq!(response.status, 200);
    assert_eq!(response.json()["content"][0]["name"], "lookup");

    // Both halves in one transcript: the refused call records the refusal, the
    // answered one records the reply it actually sent, and only the first is
    // counted as refused.
    let served: Vec<String> = provider
        .requests()
        .iter()
        .map(|request| request.served.clone())
        .collect();
    assert_eq!(served, [mock_provider::REFUSED_MISMATCH, "reply.tools"]);
    assert_eq!(provider.snapshot().refused, 1);
}

/// The escape hatch: a body served verbatim, for the responses generated code
/// must reject rather than parse.
#[test]
fn a_raw_outcome_is_served_verbatim() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::raw(200, json!({ "type": "message", "content": [] })),
    ));
    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [{ "role": "user", "content": "go" }],
        }),
    );
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json(),
        json!({ "type": "message", "content": [] }),
        "nothing is added to a raw body: no id, no usage, no stop_reason"
    );
}

/// Every answer a client receives carries a request id — the 200s and the
/// errors alike, in the header and in the error envelope's own member.
///
/// The real Messages API answers every request with one, and it is what an SDK
/// error object surfaces and what a transcript reader correlates by. Asserted
/// over a socket because the header is the half a client actually reads.
#[test]
fn every_answer_carries_a_request_id_including_the_errors() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();
    let request = json!({
        "model": MODEL,
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "go" }],
    });

    // 1: a 200. 2: a scripted rate limit. 3: an unscripted call. 4: a malformed
    // request. Each id is the call's own arrival sequence.
    provider.enqueue_all([
        Script::new(MODEL, Outcome::text("answered")),
        Script::new(MODEL, Outcome::rate_limit()),
    ]);
    let answered = send(&client, &request);
    assert_eq!(answered.status, 200);
    assert_eq!(answered.header("request-id"), Some("req_mock_00000001"));

    let limited = send(&client, &request);
    assert_eq!(limited.status, 429);
    assert_eq!(limited.header("request-id"), Some("req_mock_00000002"));
    assert_eq!(limited.json()["request_id"], "req_mock_00000002");

    let unscripted = send(&client, &request);
    assert_eq!(unscripted.status, HARNESS_STATUS);
    assert_eq!(unscripted.header("request-id"), Some("req_mock_00000003"));

    let malformed = send(&client, &json!({ "model": MODEL }));
    assert_eq!(malformed.status, 400);
    assert_eq!(malformed.header("request-id"), Some("req_mock_00000004"));
    assert_eq!(malformed.json()["request_id"], "req_mock_00000004");
}

/// The same script answered twice is the same bytes, which is what makes a
/// transcript assertable and a golden run repeatable.
#[test]
fn two_identical_runs_answer_identically() {
    let answer = |()| {
        let provider = MockProvider::start().expect("a port");
        provider.enqueue(Script::new(
            MODEL,
            Outcome::structured(json!({ "verdict": "revise", "feedback": "tighten it" })),
        ));
        send(
            &provider.client(),
            &structured_request(output_schema_tool(), "reviewer_output"),
        )
        .text()
    };
    assert_eq!(answer(()), answer(()));
}
