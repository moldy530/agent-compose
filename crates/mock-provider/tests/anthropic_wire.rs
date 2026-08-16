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

/// The tool surface a request carries is observable, which is how an
/// `agent_access: read` store attachment is checked (grammar 11.5, PRD 9.13).
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
        "a `docs_upsert` appearing here would be `agent_access: read` not being honoured"
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
