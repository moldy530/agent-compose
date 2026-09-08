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
            .and_then(mock_provider::StructuredOutput::name),
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

/// A request may carry **both** mechanisms, and the pinned call is what answers
/// it: the turn ends with `finish_reason: "tool_calls"` and a null content, so
/// the `response_format` shapes nothing.
///
/// This is the pairing a codegen path that confused the two would send —
/// WIRE-NOTES §3 records that LangChain JS selects between them with a `method`
/// option — and a mock that answered it in the content would hand that path a
/// `stop` branch to pass on, then meet a tool call on the first live request.
#[test]
fn a_request_carrying_both_structured_output_mechanisms_is_answered_with_the_pinned_call() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let mut body = json_schema_request();
    body["tools"] = json!([{
        "type": "function",
        "function": {
            "name": "reviewer_output",
            "parameters": { "type": "object", "properties": {} },
        },
    }]);
    body["tool_choice"] = json!({ "type": "function", "function": { "name": "reviewer_output" } });

    let response = send(&provider.client(), "/v1/chat/completions", &body);
    assert_eq!(response.status, 200);
    let answered = response.json();
    assert_eq!(answered["choices"][0]["finish_reason"], "tool_calls");
    assert_eq!(answered["choices"][0]["message"]["content"], Value::Null);
    let call = &answered["choices"][0]["message"]["tool_calls"][0];
    assert_eq!(call["function"]["name"], "reviewer_output");
    assert_eq!(call["function"]["arguments"], "{\"verdict\":\"approve\"}");

    // `"required"` pins a call too, but names no function to make it under, so
    // the script has to say which one — and is told so rather than served
    // content the service could not have sent.
    provider.reset();
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    body["tool_choice"] = json!("required");
    let response = send(&provider.client(), "/v1/chat/completions", &body);
    assert_eq!(response.status, HARNESS_STATUS);
    assert_eq!(
        response.header(HARNESS_HEADER),
        Some(mock_provider::REFUSED_MISMATCH)
    );
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("`tool_choice: \"required\"`"),
        "{}",
        response.json()
    );
}

/// `strict: true` closes the schema, and a schema that is not closed is a 400 —
/// the most common one on this surface, and the one a Zod-to-JSON-Schema path
/// produces when it drops `additionalProperties` or an entry in `required`.
///
/// This is the check that keeps PRD 5.2 honest here: without it a compiled graph
/// passes every acceptance run and fails on its first live call, which is the
/// exact failure the mock exists to move into CI.
#[test]
fn a_strict_schema_that_is_not_closed_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "reviewer_output",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": { "verdict": { "type": "string" } },
                    },
                },
            },
        }),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = response.json();
    assert_eq!(
        body["error"]["param"], "response_format.json_schema.schema",
        "the refusal points at the schema, where the author has to go"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("'additionalProperties' is required to be supplied and to be false"),
        "{body}"
    );
    assert_eq!(provider.snapshot().queues[MODEL], 1, "nothing was consumed");

    // The closed spelling of the same schema is the one every fixture writes,
    // and it is served — the strictness is about the schema, not about `strict`.
    provider.reset();
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));
    assert_eq!(
        send(
            &provider.client(),
            "/v1/chat/completions",
            &json_schema_request()
        )
        .status,
        200
    );
}

/// The same 400, under the definitions bucket `zod-to-json-schema` actually
/// writes.
///
/// Its `definitionPath` option defaults to `definitions`, not `$defs`, so any
/// Zod model with a reused or recursive sub-schema puts its objects there — and
/// an unclosed one has to be caught in the bucket the tool uses, not only in the
/// spelling OpenAI's examples show. This is the same rule as
/// `a_strict_schema_that_is_not_closed_is_refused`, one indirection down, and
/// the reason it is a separate test is that a walk can pass that one and miss
/// this one.
#[test]
fn a_strict_schema_left_open_under_definitions_is_refused() {
    let provider = MockProvider::start().expect("a port");

    for bucket in ["$defs", "definitions"] {
        provider.reset();
        provider.enqueue(Script::new(MODEL, Outcome::text("never served")));
        let mut schema = json!({
            "type": "object",
            "additionalProperties": false,
            "required": ["author"],
            "properties": { "author": { "$ref": format!("#/{bucket}/Author") } },
        });
        schema[bucket] = json!({
            "Author": { "type": "object", "properties": { "name": { "type": "string" } } },
        });

        let response = send(
            &provider.client(),
            "/v1/chat/completions",
            &json!({
                "model": MODEL,
                "messages": [{ "role": "user", "content": "go" }],
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "reviewer_output",
                        "strict": true,
                        "schema": schema,
                    },
                },
            }),
        );
        assert_eq!(response.status, 400, "{bucket}");
        assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
        let message = response.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .to_string();
        assert!(
            message.contains(&format!(
                "In context=('{bucket}', 'Author'), 'additionalProperties' is required to be \
                 supplied and to be false."
            )),
            "{message}"
        );
        assert_eq!(provider.snapshot().queues[MODEL], 1, "nothing was consumed");
    }
}

/// A tagged union at the root of an output schema is a 400 too, and it is the
/// shape codegen is most likely to send: `zod-to-json-schema` renders a
/// `z.discriminatedUnion` as a bare root `anyOf`, and PRD §7 M1 promises "state
/// models (incl. tagged unions via Zod)".
///
/// Structured Outputs requires the root of a schema to be an object and not an
/// `anyOf` — the union has to be one level down, under a property. A mock that
/// served this would let the tagged-union acceptance test pass on a request the
/// service refuses.
#[test]
fn a_tagged_union_at_the_root_of_an_output_schema_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "reviewer_output",
                    "strict": true,
                    "schema": {
                        "anyOf": [
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "approve" } },
                                "required": ["kind"],
                                "additionalProperties": false,
                            },
                            {
                                "type": "object",
                                "properties": { "kind": { "const": "revise" } },
                                "required": ["kind"],
                                "additionalProperties": false,
                            },
                        ],
                    },
                },
            },
        }),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = response.json();
    assert_eq!(
        body["error"]["param"], "response_format.json_schema.schema.type",
        "the refusal points at the root, where the author has to go"
    );
    assert!(
        body["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("schema must be a JSON Schema of 'type: \"object\"'"),
        "{body}"
    );
    assert_eq!(provider.snapshot().queues[MODEL], 1, "nothing was consumed");
}

/// An empty tool list is a 400, not an ignored key: it is what codegen emits for
/// an agent with neither `tools:` nor `stores:` if it always writes `tools`.
#[test]
fn an_empty_tools_array_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
            "tools": [],
        }),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["param"], "tools");
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

/// The newer Azure route has no deployment in its path, so a body that names no
/// model is refused there — the same refusal the direct route gives.
///
/// The classic route is the only place a missing `model` is legal, because the
/// deployment in the path is what selects it. Accepting it on `/openai/v1` would
/// key every such call under the empty string and let codegen that dropped
/// `model` pass an acceptance run.
#[test]
fn the_newer_azure_route_still_requires_a_model_in_the_body() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(
            Request::post("/openai/v1/chat/completions?api-version=preview")
                .azure_auth()
                .json(&json!({ "messages": [{ "role": "user", "content": "go" }] })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["param"], "model");

    let recorded = provider.requests();
    assert_eq!(recorded[0].served, "rejected");
    assert!(!recorded[0].is_valid());
    assert_eq!(
        recorded[0]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["model"]
    );
}

/// Azure's **classic** route without `api-version` is a refusal, because the
/// service refuses it there — and the newer v1 route without one is served,
/// because the service serves it (WIRE-NOTES §7).
///
/// The positive half is why the two routes are told apart at all: refusing a
/// request the service accepts would fail a fixture that is correct, which is
/// the same class of bug as accepting one the service refuses.
#[test]
fn an_api_version_is_required_on_the_classic_azure_route_only() {
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

    provider.enqueue(Script::new(MODEL, Outcome::text("served without one")));
    let response = provider
        .client()
        .send(
            Request::post("/openai/v1/chat/completions")
                .azure_auth()
                .json(&json!({
                    "model": MODEL,
                    "messages": [{ "role": "user", "content": "go" }],
                })),
        )
        .expect("the v1 route answers");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["choices"][0]["message"]["content"],
        "served without one"
    );
    assert_eq!(provider.requests()[1].query, "");
}

/// A transcript spells the surface the way grammar 12.1 spells the provider
/// kinds that reach it, which is what a harness filters on.
#[test]
fn the_transcript_spells_each_surface_the_way_the_grammar_does() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();
    provider.enqueue_all([
        Script::new(MODEL, Outcome::text("direct")),
        Script::new("smart-deployment", Outcome::text("azure")),
    ]);

    send(
        &client,
        "/v1/chat/completions",
        &json!({ "model": MODEL, "messages": [{ "role": "user", "content": "go" }] }),
    );
    client
        .send(
            Request::post(
                "/openai/deployments/smart-deployment/chat/completions?api-version=2024-10-21",
            )
            .azure_auth()
            .json(&json!({ "messages": [{ "role": "user", "content": "go" }] })),
        )
        .expect("the classic Azure route answers");

    let transcript = client
        .get("/_mock/requests")
        .expect("the control plane answers")
        .json();
    assert_eq!(
        transcript["requests"][0]["surface"], "openai",
        "a harness filtering on the grammar's own kind spelling must match"
    );
    assert_eq!(transcript["requests"][1]["surface"], "azure_openai");
}

/// A call with no credentials is **served** on the direct route, because a
/// composition is entitled to send none.
///
/// All three kinds that reach this route may omit `api_key:`:
/// `openai_compatible` always could, and grammar 12.1 makes it conditional on
/// `openai` too (Decision D120), so a provider naming a `base_url:` sends no
/// `Authorization` at all. This server stands in for whatever that `base_url:`
/// names, so refusing the request would make the harness stricter than the
/// grammar — and in CI it is the only endpoint a compiled graph reaches
/// (WIRE-NOTES (12)). The Azure routes keep their check, which the test below
/// pins.
#[test]
fn a_request_without_credentials_is_served_on_the_direct_route() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("served without a key")));
    let response = provider
        .client()
        .send(Request::post("/v1/chat/completions").json(&json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
        })))
        .expect("the route answers");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["choices"][0]["message"]["content"],
        "served without a key"
    );
    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert!(
        !recorded[0].headers.contains_key("authorization"),
        "the request really carried no credential: {:?}",
        recorded[0].headers
    );
    assert!(provider.snapshot().is_drained(), "the call was served");

    // The body is still checked: dropping the credential check did not drop the
    // request check that shares the route.
    let response = provider
        .client()
        .send(Request::post("/v1/chat/completions").json(&json!({ "model": MODEL })))
        .expect("the route answers");
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["param"], "messages");
    assert_eq!(
        provider.requests()[1]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["messages"],
    );
}

/// A call that *does* carry a credential is still held to its shape: only the
/// presence requirement was dropped on the direct route, not the check.
///
/// This is the half of WIRE-NOTES (12) the keyless relaxation must not take with
/// it. A graph that declares `api_key:` and puts it on the wire raw — no `Bearer`
/// prefix — or that renders `Bearer ` around a key that resolved to nothing, is
/// a codegen bug `api.openai.com` answers 401; a harness that served it would
/// pass the bug through to the first live call, which is the failure this whole
/// file exists to prevent. The refusal names the credential and nothing else,
/// and consumes no script.
///
/// Every shape here is decidable **from one request**, which is what separates
/// them from the scheme the test below serves: a token-less `Bearer` in both of
/// its spellings, a value carrying no scheme at all, a gateway scheme with
/// nothing behind it, and a non-`Bearer` scheme riding beside an `api-key` —
/// two credentials on a route that reads neither, which no keyless composition
/// produces.
#[test]
fn a_request_whose_credential_is_malformed_is_refused_on_the_direct_route() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("never served")));

    let malformed: [&[(&str, &str)]; 5] = [
        &[("authorization", "mock-provider-key")],
        &[("authorization", "Bearer")],
        &[("authorization", "Bearer ")],
        &[("authorization", "Basic ")],
        &[
            ("authorization", "Basic bW9jay1nYXRld2F5"),
            ("api-key", "mock-provider-key"),
        ],
    ];
    for headers in malformed {
        let mut request = Request::post("/v1/chat/completions");
        for (name, value) in headers {
            request = request.header(name, *value);
        }
        let response = provider
            .client()
            .send(request.json(&json!({
                "model": MODEL,
                "messages": [{ "role": "user", "content": "go" }],
            })))
            .expect("the route answers");
        assert_eq!(response.status, 401, "{headers:?}");
        let body = response.json();
        assert_eq!(body["error"]["code"], "invalid_api_key");
        assert_eq!(
            body["error"]["param"],
            Value::Null,
            "a header is not a request parameter"
        );
        assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    }

    let recorded = provider.requests();
    assert_eq!(recorded.len(), malformed.len());
    for request in &recorded {
        assert!(!request.is_valid());
        assert_eq!(
            request
                .failures()
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["headers.authorization"],
            "the body was well formed: only the credential is wrong"
        );
    }
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "a request refused for its credential consumes nothing"
    );

    // …and the well-formed spelling of the same key is served, so what the five
    // refusals measure is the shape and not the route.
    let response = send(
        &provider.client(),
        "/v1/chat/completions",
        &json!({
            "model": MODEL,
            "messages": [{ "role": "user", "content": "go" }],
        }),
    );
    assert_eq!(response.status, 200);
    assert!(
        provider.snapshot().queues.is_empty(),
        "the well-formed call is the one that consumed the script"
    );
}

/// A gateway's own token under a scheme that is **not** `Bearer` is served on
/// the direct route, because from one request it is not a mistake.
///
/// This is the concession `src/anthropic.rs` already makes on the Messages wire,
/// reached from the other side, and WIRE-NOTES (12) records both. A keyless
/// provider declares the gateway's credential through `headers:`
/// (`docs/topics/models.md`, "Keyless providers behind a gateway"), whose values
/// are free strings — `authorization: "Basic ${GW_TOKEN}"` is an ordinary
/// composition, and a gateway is entitled to any scheme it likes. Refusing it
/// would make the harness stricter than the grammar for the one deployment shape
/// D120 exists to admit, and in CI this server is the only endpoint a compiled
/// graph reaches.
///
/// What keeps that from swallowing the check above is that the served shape is
/// still a *shape*: `<scheme> <token>`, alone on the request. `Basic ` with no
/// token and `Basic …` beside an `api-key` are both refused by the test above,
/// so what this one buys is the scheme and nothing else.
#[test]
fn a_gateway_token_under_another_scheme_is_served_on_the_direct_route() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(MODEL, Outcome::text("served for the gateway")),
        Script::new(MODEL, Outcome::text("served for the other gateway")),
    ]);

    let body = json!({
        "model": MODEL,
        "messages": [{ "role": "user", "content": "go" }],
    });
    for (scheme, answer) in [
        ("Basic bW9jay1nYXRld2F5", "served for the gateway"),
        ("Token mock-gateway-token", "served for the other gateway"),
    ] {
        let response = provider
            .client()
            .send(
                Request::post("/v1/chat/completions")
                    .header("authorization", scheme)
                    .json(&body),
            )
            .expect("the route answers");
        assert_eq!(response.status, 200, "`authorization: {scheme}`");
        assert_eq!(response.json()["choices"][0]["message"]["content"], answer);
    }

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    for request in &recorded {
        assert!(request.is_valid(), "{:?}", request.failures());
    }
    assert_eq!(
        recorded[0].headers["authorization"], "Basic bW9jay1nYXRld2F5",
        "…and the token the composition declared is what rode the request"
    );
    assert!(
        provider.snapshot().is_drained(),
        "both gateway calls were served"
    );
}

/// Azure's missing subscription key is still a **401**, and it is the last
/// credential this server requires.
///
/// `azure_openai` is the one kind whose `api_key:` grammar 12.1 requires
/// outright — a per-resource deployment has no default endpoint and no keyless
/// posture — so a request reaching a deployment route with neither spelling of
/// the credential is the codegen bug this check was written for. 401 rather than
/// 400 is the status the `openai` SDK raises `AuthenticationError` from, and
/// neither the failover set (PRD 5.9) nor the SDK's retry set claims it, so
/// nothing else changes shape.
///
/// The second half is the ordering rule that used to live on the direct route:
/// authentication is settled before the body is, so a request that is both
/// unauthenticated and malformed is the 401, naming the credential only — while
/// the transcript still records everything that was wrong.
#[test]
fn an_azure_request_without_a_subscription_key_is_refused() {
    let provider = MockProvider::start().expect("a port");
    let response = provider
        .client()
        .send(
            Request::post("/openai/deployments/smart/chat/completions?api-version=2024-10-21")
                .json(&json!({
                    "model": MODEL,
                    "messages": [{ "role": "user", "content": "go" }],
                })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 401);
    let body = response.json();
    assert_eq!(body["error"]["code"], "invalid_api_key");
    assert_eq!(
        body["error"]["param"],
        Value::Null,
        "a header is not a request parameter"
    );
    assert_eq!(
        provider.requests()[0].failures()[0].pointer,
        "headers.api-key"
    );

    let response = provider
        .client()
        .send(
            Request::post("/openai/deployments/smart/chat/completions?api-version=2024-10-21")
                .json(&json!({ "model": MODEL })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 401);
    let message = response.json()["error"]["message"]
        .as_str()
        .expect("a message")
        .to_string();
    assert!(message.contains("subscription key"), "{message}");
    assert!(!message.contains("messages"), "{message}");
    assert_eq!(
        provider.requests()[1]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["headers.api-key", "messages"],
        "the transcript still records everything that was wrong"
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
