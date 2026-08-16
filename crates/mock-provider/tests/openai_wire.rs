//! The Chat Completions surface, over a real socket — the one three of grammar
//! 12.1's provider kinds reach.
//!
//! `openai` and `openai_compatible` differ only in where they point, which is a
//! `base_url` and not a wire difference, so one route serves both; `azure_openai`
//! differs in route and auth header and is exercised here on both of its
//! spellings. What a test asserts on is the same thing either way: the request a
//! compiled graph sent, and the answer its client library will parse.

use std::time::Duration;

use mock_provider::{
    Client, HARNESS_HEADER, HARNESS_STATUS, MockProvider, Outcome, REFUSED_INVALID, Request,
    Script, Surface, ToolCall,
};
use serde_json::{Value, json};

const MODEL: &str = "gpt-4o-mini";

fn send(client: &Client, path: &str, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post(path).openai_auth().json(body))
        .expect("the mock provider answers")
}

fn json_schema_request() -> Value {
    json!({
        "model": MODEL,
        "messages": [
            { "role": "system", "content": "You are a meticulous technical reviewer." },
            { "role": "user", "content": "{\"goal\":\"ship it\"}" },
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
                    },
                    "required": ["verdict"],
                    "additionalProperties": false,
                },
            },
        },
    })
}

/// `response_format: json_schema` — one of the two shapes `withStructuredOutput`
/// produces. The object comes back serialized into the message content.
#[test]
fn a_json_schema_structured_output_arrives_in_the_message_content() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json_schema_request(),
    );
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["object"], "chat.completion");
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["choices"][0]["finish_reason"], "stop");
    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .expect("content is a string");
    assert_eq!(
        serde_json::from_str::<Value>(content).expect("content parses as JSON"),
        json!({ "verdict": "approve" })
    );

    let recorded = provider.requests();
    assert_eq!(recorded[0].surface, Surface::OpenAi);
    assert!(recorded[0].is_valid());
    assert_eq!(
        recorded[0]
            .structured_output
            .as_ref()
            .map(mock_provider::StructuredOutput::name),
        Some("reviewer_output")
    );
}

/// The other shape: a forced function, answered with a tool call whose
/// `arguments` is a JSON **string**.
#[test]
fn a_forced_function_structured_output_arrives_as_a_tool_call() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));

    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [{
                "type": "function",
                "function": {
                    "name": "reviewer_output",
                    "parameters": { "type": "object", "properties": {} },
                },
            }],
            "tool_choice": { "type": "function", "function": { "name": "reviewer_output" } },
        }),
    );
    let body = response.json();
    let call = &body["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["type"], "function");
    assert_eq!(call["function"]["name"], "reviewer_output");
    assert_eq!(call["function"]["arguments"], "{\"verdict\":\"revise\"}");
    assert_eq!(body["choices"][0]["finish_reason"], "tool_calls");
}

/// The whole tool loop on this surface, including the `tool` message that
/// answers a call by id.
#[test]
fn a_tool_loop_runs_over_two_calls() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(
            MODEL,
            Outcome::tool_calls(vec![ToolCall::new("web_search", json!({ "query": "it" }))]),
        ),
        Script::new(MODEL, Outcome::text("done")),
    ]);
    let client = provider.client();
    let tools = json!([{
        "type": "function",
        "function": { "name": "web_search", "parameters": { "type": "object" } },
    }]);

    let first = send(
        &client,
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "search" }],
            "tools": tools,
        }),
    )
    .json();
    let call = first["choices"][0]["message"]["tool_calls"][0].clone();
    let call_id = call["id"].as_str().expect("a call id").to_string();

    let second = send(
        &client,
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [
                { "role": "user", "content": "search" },
                { "role": "assistant", "tool_calls": [call] },
                { "role": "tool", "tool_call_id": call_id, "content": "[]" },
            ],
            "tools": tools,
        }),
    );
    assert_eq!(second.status, 200);
    assert_eq!(second.json()["choices"][0]["message"]["content"], "done");
    assert!(
        provider
            .requests()
            .iter()
            .all(mock_provider::RecordedRequest::is_valid)
    );
}

/// A `tool` message with no call to answer is refused with the API's own
/// sentence, and the refusal points at the parameter.
#[test]
fn an_orphaned_tool_message_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "tool", "tool_call_id": "call_1", "content": "[]" },
            ],
        }),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = response.json();
    assert_eq!(body["error"]["param"], "messages.1.tool_call_id");
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(provider.snapshot().queues[MODEL], 1, "nothing was consumed");
}

/// An unknown argument is a refusal on this surface too — the mistake a settings
/// block (grammar 12.2) can put on the wire.
#[test]
fn an_unknown_argument_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
            "thinking": { "budget_tokens": 4000 },
        }),
    );
    assert_eq!(response.status, 400);
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("thinking"),
        "an Anthropic-only setting on an OpenAI provider is refused on the wire, \
         which is the runtime half of the compile-time check in grammar 12.2"
    );
}

/// Both Azure spellings reach the same surface, and the classic one keys its
/// queue on the deployment in the path.
#[test]
fn both_azure_routes_are_served() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();
    provider.enqueue_all([
        Script::new("smart-deployment", Outcome::text("from the deployment")),
        Script::new(MODEL, Outcome::text("from the body")),
    ]);

    let classic = client
        .send(
            Request::post(
                "/openai/deployments/smart-deployment/chat/completions?api-version=2024-10-21",
            )
            .azure_auth()
            .json(&json!({ "messages": [{ "role": "user", "content": "go" }] })),
        )
        .expect("the classic Azure route answers");
    assert_eq!(classic.status, 200);
    assert_eq!(
        classic.json()["choices"][0]["message"]["content"],
        "from the deployment"
    );

    let modern = client
        .send(
            Request::post("/openai/v1/chat/completions?api-version=preview")
                .azure_auth()
                .json(&json!({
                    "model": MODEL,
                    "messages": [{ "role": "user", "content": "go" }],
                })),
        )
        .expect("the newer Azure route answers");
    assert_eq!(modern.status, 200);
    assert_eq!(
        modern.json()["choices"][0]["message"]["content"],
        "from the body"
    );

    let recorded = provider.requests();
    assert!(
        recorded
            .iter()
            .all(|request| request.surface == Surface::AzureOpenAi)
    );
    assert_eq!(recorded[0].model, "smart-deployment");
    assert_eq!(recorded[0].query, "api-version=2024-10-21");
    assert_eq!(recorded[1].model, MODEL);
}

/// Azure without `api-version` is a refusal, because the service refuses it.
#[test]
fn an_azure_request_without_an_api_version_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(
            Request::post("/openai/deployments/smart-deployment/chat/completions")
                .azure_auth()
                .json(&json!({ "messages": [{ "role": "user", "content": "go" }] })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["param"], "query.api-version");
}

/// A call with no credentials is refused: the harness needs no API *keys*, but
/// the request still has to carry the header a client sends.
#[test]
fn a_request_without_credentials_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(Request::post("/v1/chat/completions").json(&json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the route answers");
    assert_eq!(response.status, 400);
    assert_eq!(
        provider.requests()[0].failures()[0].pointer,
        "headers.authorization"
    );
}

/// The failover conditions, in this surface's own statuses.
#[test]
fn the_failover_conditions_carry_this_surfaces_statuses() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client().with_timeout(Duration::from_millis(500));
    let request = json!({
        "model": MODEL,
        "messages": [{ "role": "user", "content": "go" }],
    });

    provider.enqueue(Script::new(MODEL, Outcome::rate_limit()));
    let response = send(&client, "/v1/chat/completions", &request);
    assert_eq!(response.status, 429);
    assert_eq!(response.json()["error"]["code"], "rate_limit_exceeded");

    provider.enqueue(Script::new(MODEL, Outcome::overloaded()));
    assert_eq!(
        send(&client, "/v1/chat/completions", &request).status,
        503,
        "overload is a 503 here and a 529 on the Anthropic surface"
    );

    provider.enqueue(Script::new(MODEL, Outcome::server_error()));
    assert_eq!(send(&client, "/v1/chat/completions", &request).status, 500);
}

/// A path no provider serves answers in the harness's voice rather than
/// silently: a base URL joined wrongly is a codegen bug too.
#[test]
fn an_unknown_route_is_refused_by_the_harness() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(
            Request::post("/v1/completions")
                .openai_auth()
                .json(&json!({})),
        )
        .expect("the server answers");
    assert_eq!(response.status, 404);
    assert_eq!(response.header(HARNESS_HEADER), Some("unknown-route"));
    assert!(
        provider.requests().is_empty(),
        "a request to no surface is not a model call"
    );
    assert_ne!(
        response.status, HARNESS_STATUS,
        "an unrouted path is a 404, not the refusal status"
    );
}
