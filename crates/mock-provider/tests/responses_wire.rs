//! The Responses surface, over a real socket — the wire an `openai` provider
//! reaches once it declares `server_tools:` (grammar 12.1, Decision D122).
//!
//! Parity with the other two wire suites: the request a compiled graph sends,
//! and the answer its client will parse. What is new here is the third thing a
//! server-tool composition needs — that the provider's own suite arrives
//! **verbatim**, and that a scripted use of it comes back woven into the turn
//! rather than as something the graph has to run.

use mock_provider::{
    Client, HARNESS_HEADER, HARNESS_STATUS, MockProvider, Outcome, REFUSED_INVALID, Request,
    Script, ServerToolUse, Surface, ToolCall,
};
use serde_json::{Value, json};

const MODEL: &str = "gpt-5";

fn send(client: &Client, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post("/v1/responses").openai_auth().json(body))
        .expect("the mock provider answers")
}

/// The suite a provider declares, exactly as the runtime appends it.
fn web_search() -> Value {
    json!({
        "type": "web_search",
        "search_context_size": "medium",
        "filters": { "allowed_domains": ["docs.example.com"] },
    })
}

fn structured_request() -> Value {
    json!({
        "model": MODEL,
        "instructions": "You are a meticulous technical reviewer.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "{\"goal\":\"ship it\"}" }],
        }],
        "tools": [web_search()],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "reviewer_output",
                "strict": true,
                "schema": {
                    "type": "object",
                    "properties": { "verdict": { "type": "string", "enum": ["approve", "revise"] } },
                    "required": ["verdict"],
                    "additionalProperties": false,
                },
            },
        },
    })
}

/// Structured output is `text.format`, and the object comes back as the
/// assistant's own text — which is q16's posture on this wire: what was
/// constrained is exactly what is parsed.
#[test]
fn a_text_format_structured_output_arrives_as_the_assistant_text() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let response = send(&provider.client(), &structured_request());
    assert_eq!(response.status, 200);
    let body = response.json();
    assert_eq!(body["object"], "response");
    assert_eq!(body["status"], "completed");
    assert_eq!(body["model"], MODEL);
    let text = body["output"][0]["content"][0]["text"]
        .as_str()
        .expect("the assistant answered with text");
    assert_eq!(
        serde_json::from_str::<Value>(text).expect("the text parses as JSON"),
        json!({ "verdict": "approve" })
    );

    let recorded = provider.requests();
    assert_eq!(recorded[0].surface, Surface::Responses);
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert_eq!(
        recorded[0]
            .structured_output
            .as_ref()
            .map(mock_provider::StructuredOutput::name),
        Some("reviewer_output")
    );
}

/// The provider's suite reaches the wire **as written**, and is recorded apart
/// from the functions the graph dispatches.
#[test]
fn a_declared_server_tool_arrives_verbatim_and_is_recorded_as_one() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("nothing to look up")));

    let request = json!({
        "model": MODEL,
        "input": [{ "type": "message", "role": "user", "content": "hello" }],
        "tools": [
            {
                "type": "function",
                "name": "lookup",
                "description": "Look one thing up.",
                "parameters": {
                    "type": "object",
                    "properties": { "query": { "type": "string" } },
                    "required": ["query"],
                    "additionalProperties": false,
                },
            },
            web_search(),
        ],
    });
    assert_eq!(send(&provider.client(), &request).status, 200);

    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert_eq!(recorded[0].tools, ["lookup"]);
    assert_eq!(recorded[0].server_tools, ["web_search"]);
    // Verbatim: the config the provider declared, field for field, is the object
    // on the wire. This is the assertion resolved q30's pass-through is *for*.
    assert_eq!(recorded[0].body()["tools"][1], web_search());
}

/// A scripted server-tool use comes back as an item of the turn, ahead of what
/// the model then said — already answered, with nothing for the graph to run.
#[test]
fn a_scripted_server_tool_use_is_woven_into_the_turn() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })).with_server_tools(vec![
            ServerToolUse::new(
                "web_search",
                json!({ "type": "search", "query": "agent-compose" }),
                json!([{ "url": "https://docs.example.com/a", "title": "A" }]),
            ),
        ]),
    ));

    let body = send(&provider.client(), &structured_request()).json();
    let output = body["output"].as_array().expect("an output list");
    assert_eq!(output.len(), 2, "the use, then the message: {output:?}");
    assert_eq!(output[0]["type"], "web_search_call");
    assert_eq!(output[0]["status"], "completed");
    assert_eq!(output[0]["results"][0]["url"], "https://docs.example.com/a");
    assert_eq!(output[1]["type"], "message");
}

/// A provider runs only the server tools it was given: a script naming one the
/// request does not declare is the harness bug it looks like.
#[test]
fn a_server_tool_the_request_did_not_declare_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::text("looked it up").with_server_tools(vec![ServerToolUse::new(
            "file_search",
            json!({}),
            json!([]),
        )]),
    ));

    let request = json!({
        "model": MODEL,
        "input": [{ "type": "message", "role": "user", "content": "hello" }],
        "tools": [web_search()],
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, HARNESS_STATUS);
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("file_search")),
        "the refusal names the tool: {}",
        response.json()
    );
}

/// A function call is a `function_call` item, and its arguments travel as a
/// JSON **string** — the shape a compiled graph's loop reads back.
#[test]
fn a_scripted_function_call_is_its_own_item() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::tool_calls(vec![ToolCall::new("lookup", json!({ "query": "who" }))]),
    ));

    let request = json!({
        "model": MODEL,
        "input": [{ "type": "message", "role": "user", "content": "hello" }],
        "tools": [{
            "type": "function",
            "name": "lookup",
            "parameters": {
                "type": "object",
                "properties": { "query": { "type": "string" } },
                "required": ["query"],
                "additionalProperties": false,
            },
        }],
    });
    let body = send(&provider.client(), &request).json();
    let call = &body["output"][0];
    assert_eq!(call["type"], "function_call");
    assert_eq!(call["name"], "lookup");
    assert_eq!(
        serde_json::from_str::<Value>(call["arguments"].as_str().expect("a JSON string"))
            .expect("arguments parse"),
        json!({ "query": "who" })
    );
    assert!(
        call["call_id"].as_str().is_some_and(|id| !id.is_empty()),
        "a call carries the id its output answers"
    );
}

/// The correlation rule of this wire: every `function_call` is answered by a
/// `function_call_output` carrying its `call_id`, and an output that answers
/// nothing is refused.
#[test]
fn an_unanswered_function_call_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("carrying on")));

    let request = json!({
        "model": MODEL,
        "input": [
            { "type": "message", "role": "user", "content": "hello" },
            { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}" },
        ],
        "tools": [{
            "type": "function",
            "name": "lookup",
            "parameters": { "type": "object", "properties": {}, "required": [], "additionalProperties": false },
        }],
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, 400);
    assert_eq!(
        response.headers.get(HARNESS_HEADER).map(String::as_str),
        Some(REFUSED_INVALID)
    );
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("call_1")),
        "the refusal names the unanswered call: {}",
        response.json()
    );
}

/// An output answering a call the conversation does not contain is the mirror
/// failure, and is refused too.
#[test]
fn an_output_answering_nothing_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("carrying on")));

    let request = json!({
        "model": MODEL,
        "input": [
            { "type": "message", "role": "user", "content": "hello" },
            { "type": "function_call_output", "call_id": "call_missing", "output": "{}" },
        ],
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, 400);
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("call_missing")),
        "{}",
        response.json()
    );
}

/// `max_tokens` is the Chat Completions spelling, and a runtime that forgot to
/// translate it is caught by the closed key list rather than serving a request
/// whose bound the service would ignore.
#[test]
fn the_chat_completions_spelling_of_the_token_bound_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("hello")));

    let request = json!({
        "model": MODEL,
        "input": [{ "type": "message", "role": "user", "content": "hello" }],
        "max_tokens": 512,
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, 400);
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("max_tokens")),
        "{}",
        response.json()
    );
}

/// An assistant turn replayed back — the shape a tool loop's second call carries
/// — is accepted with its own items, server-tool records included.
#[test]
fn a_replayed_turn_carrying_a_server_tool_item_is_accepted() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("done")));

    let request = json!({
        "model": MODEL,
        "input": [
            { "type": "message", "role": "user", "content": "hello" },
            { "type": "web_search_call", "id": "srv_1", "status": "completed" },
            {
                "type": "message",
                "role": "assistant",
                "content": [{ "type": "output_text", "text": "I looked it up." }],
            },
            { "type": "function_call", "call_id": "call_1", "name": "lookup", "arguments": "{}" },
            { "type": "function_call_output", "call_id": "call_1", "output": "\"a fact\"" },
        ],
        "tools": [
            {
                "type": "function",
                "name": "lookup",
                "parameters": { "type": "object", "properties": {}, "required": [], "additionalProperties": false },
            },
            web_search(),
        ],
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, 200, "{}", response.json());
    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
}

/// A scripted `max_output_tokens` cut comes back as `status: "incomplete"` with
/// the reason beside it — the pair a compiled agent reads to tell a cut answer
/// from a refusal.
#[test]
fn a_scripted_cut_answer_is_incomplete_with_its_reason() {
    let provider = MockProvider::start().expect("a port");
    let mut script = Script::new(MODEL, Outcome::text("as far as I go"));
    if let Outcome::Reply(reply) = &mut script.outcome {
        reply.stop_reason = Some("max_output_tokens".to_string());
    }
    provider.enqueue(script);

    let request = json!({
        "model": MODEL,
        "input": [{ "type": "message", "role": "user", "content": "hello" }],
    });
    let body = send(&provider.client(), &request).json();
    assert_eq!(body["status"], "incomplete");
    assert_eq!(body["incomplete_details"]["reason"], "max_output_tokens");
}

/// The route exists and is named where a 404 lists what this server serves, so a
/// base URL joined wrongly says so rather than looking like an unimplemented
/// wire.
#[test]
fn an_unknown_route_names_the_responses_surface() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(Request::post("/v1/respones").openai_auth().json(&json!({})))
        .expect("the server answers");
    assert_eq!(response.status, 404);
    assert!(
        response.json()["mock_provider"]
            .as_str()
            .is_some_and(|message| message.contains("/v1/responses")),
        "{}",
        response.json()
    );
}
