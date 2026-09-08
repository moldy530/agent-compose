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
            .and_then(|output| output.name()),
        Some("reviewer_output")
    );
    assert!(provider.snapshot().is_drained(), "the script was consumed");
}

/// The wire's **other** structured-output mechanism: `output_config`'s format,
/// answered with a text block that parses (PRD §9 resolved q53,
/// `WIRE-NOTES` (26)).
///
/// The same scripted object as the test above, and the script says nothing about
/// which mechanism asked — the *request* decides where the object goes, which is
/// what lets one script drive both rungs of the runtime's ladder.
#[test]
fn an_output_config_structured_output_arrives_as_the_assistant_text() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 4096,
            "system": "You are a meticulous technical reviewer.",
            "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\"}" }],
            "output_config": {
                "format": {
                    "type": "json_schema",
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
        }),
    );
    assert_eq!(response.status, 200, "{}", response.text());
    let body = response.json();
    assert_eq!(body["stop_reason"], "end_turn", "no call ended this turn");
    assert_eq!(body["content"][0]["type"], "text");
    let text = body["content"][0]["text"]
        .as_str()
        .expect("the format shaped the assistant's text")
        .to_string();
    assert_eq!(
        serde_json::from_str::<Value>(&text).expect("which parses"),
        json!({ "verdict": "approve", "feedback": "" })
    );

    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert!(
        recorded[0].tools.is_empty(),
        "this mechanism declares no tool at all: {:?}",
        recorded[0].tools
    );
    assert_eq!(
        recorded[0]
            .structured_output
            .as_ref()
            .map(mock_provider::StructuredOutput::mechanism),
        Some(mock_provider::OutputMechanism::Native)
    );
    assert_eq!(
        recorded[0]
            .structured_output
            .as_ref()
            .and_then(|output| output.name()),
        None,
        "…and no name: a format constrains the text and has no call to name"
    );
    assert!(provider.snapshot().is_drained());
}

/// The **deprecated** spelling is not this wire's parameter, and a request that
/// sent one is refused as the unknown argument it is (`WIRE-NOTES` (26)).
///
/// The failure mode a compiled graph that regressed to `output_format` should
/// have: refused here, in CI, rather than accepted by a server more forgiving
/// than the API.
#[test]
fn the_deprecated_output_format_spelling_is_an_unknown_argument() {
    let provider = MockProvider::start().expect("a port");
    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": "hi" }],
            "output_format": { "type": "json_schema", "schema": { "type": "object" } },
        }),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let recorded = provider.requests();
    assert!(!recorded[0].is_valid());
    assert_eq!(
        recorded[0].failures()[0].message,
        "output_format: Extra inputs are not permitted"
    );
    assert_eq!(
        recorded[0].unsupported, None,
        "a malformed request is not an endpoint refusing a mechanism"
    );
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

/// The conversation a tool loop hands its pinned call, as it reaches a socket:
/// ending on the assistant it is **refused**, and ending on the closing user
/// turn it is served (PRD §9 resolved q52).
///
/// A trailing assistant message is prefill, and prefill under
/// `tool_choice: {type: "tool", …}` is a contradiction. `api.anthropic.com`
/// tolerates it; the strict Anthropic-compatible gateways an enterprise
/// deployment runs behind answer 400, which is how the shape reached a live
/// 0.6.0 field report at all — every test passed against a mock as lenient as
/// the vendor. So this server is the strict one, and the pair of runs below is
/// the whole rule: the same history, the same forced tool, and the closing user
/// turn as the only difference.
#[test]
fn a_forced_tool_choice_needs_the_conversation_to_end_on_the_user() {
    let loop_turns = [
        json!({ "role": "user", "content": "{\"goal\":\"ship it\"}" }),
        json!({ "role": "assistant", "content": [
            { "type": "tool_use", "id": "toolu_1", "name": "lookup", "input": { "query": "it" } },
        ]}),
        json!({ "role": "user", "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1", "content": "a looked-up snippet" },
        ]}),
        json!({ "role": "assistant", "content": [{ "type": "text", "text": "found it" }] }),
    ];
    let tools = json!([
        { "name": "lookup", "description": "Look one fact up.", "input_schema": { "type": "object" } },
        {
            "name": "reviewer_output",
            "description": "The structured output agent.reviewer must produce.",
            "input_schema": { "type": "object", "properties": { "verdict": { "type": "string" } } },
        },
    ]);
    let pinned = |messages: Value| {
        json!({
            "model": MODEL,
            "max_tokens": 4096,
            "system": "You are a meticulous technical reviewer.",
            "messages": messages,
            "tools": tools,
            "tool_choice": { "type": "tool", "name": "reviewer_output" },
        })
    };

    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let refused = send(
        &provider.client(),
        &pinned(Value::Array(loop_turns.to_vec())),
    );
    assert_eq!(refused.status, 400);
    assert_eq!(refused.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    let body = refused.json();
    assert_eq!(body["error"]["type"], "invalid_request_error");
    assert_eq!(
        body["error"]["message"],
        "messages.3: This model does not support assistant message prefill. \
         The conversation must end with a user message when `tool_choice` \
         forces a tool.",
        "{body}"
    );
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "a refused request consumes nothing"
    );

    // The same request with the turn the runtime appends: served, and the pinned
    // tool is still what the answer comes back as.
    let mut closed = loop_turns.to_vec();
    closed.push(json!({ "role": "user", "content": "Now produce the structured result." }));
    let served = send(&provider.client(), &pinned(Value::Array(closed)));
    assert_eq!(served.status, 200);
    let body = served.json();
    assert_eq!(body["content"][0]["type"], "tool_use");
    assert_eq!(body["content"][0]["name"], "reviewer_output");
    assert_eq!(body["content"][0]["input"]["verdict"], "approve");
    let snapshot = provider.snapshot();
    assert!(snapshot.queues.is_empty(), "the one script was consumed");
    assert_eq!(snapshot.invalid, 1, "by the second request, not the first");
}

/// Prefill on its own is **not** refused: it is the Messages API's own feature,
/// and only a forced `tool_choice` beside it makes the pair a contradiction.
///
/// Without this the rule above would be indistinguishable from "assistant turns
/// may not end a conversation", which is a different and wrong server.
#[test]
fn prefill_without_a_forced_tool_choice_is_served() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text(" rest of the sentence")));

    let response = send(
        &provider.client(),
        &json!({
            "model": MODEL,
            "max_tokens": 1024,
            "messages": [
                { "role": "user", "content": "finish this sentence" },
                { "role": "assistant", "content": "Here is the" },
            ],
        }),
    );

    assert_eq!(response.status, 200);
    assert_eq!(response.json()["content"][0]["type"], "text");
    assert!(provider.snapshot().is_drained());
}

/// A call with **no `x-api-key`** is served, because a composition is entitled
/// to send none.
///
/// Grammar 12.1 makes the key conditional on this kind (Decision D120): a
/// provider that names a `base_url:` may declare none, and a compiled graph then
/// sends no authentication header at all. This server stands in for whatever
/// that `base_url:` names — routinely a gateway that injects the vendor
/// credential itself — so requiring the header would refuse the one shape D120
/// exists to admit, in CI, where this is the only endpoint a graph reaches
/// (WIRE-NOTES (12)). The vendor's own host would answer 401; that divergence is
/// the deliberate one this file records.
///
/// What is *not* relaxed is the rest of the envelope, which the second half
/// pins: dropping the credential check did not drop the request check beside it.
#[test]
fn a_request_without_an_api_key_is_served() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("served without a key")));

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
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["content"][0]["text"],
        "served without a key"
    );

    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert!(
        !recorded[0].headers.contains_key("x-api-key"),
        "the request really carried no credential: {:?}",
        recorded[0].headers
    );
    assert!(provider.snapshot().is_drained(), "the call was served");

    // The body is still checked, and so is `anthropic-version`: a keyless
    // request is a legal shape, not an unchecked one.
    let response = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .header("anthropic-version", "2023-06-01")
                .json(&json!({ "model": MODEL })),
        )
        .expect("the route answers");
    assert_eq!(response.status, 400);
    assert_eq!(response.json()["error"]["type"], "invalid_request_error");
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    assert_eq!(
        provider.requests()[1]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["max_tokens", "messages"],
        "the transcript still records everything that was wrong"
    );

    let response = send(&provider.client(), &json!({ "model": MODEL }));
    assert_eq!(
        response.status, 400,
        "and a keyed request is refused the same way"
    );
}

/// An **empty** `x-api-key` is refused 401, because it is neither posture the
/// grammar admits.
///
/// This is the half of WIRE-NOTES (12) the keyless relaxation must not take with
/// it: only the *presence* requirement was dropped, not the check. A keyless
/// provider sends no header at all — the emitted runtime's `credential` drops it
/// rather than emptying it, precisely so a gateway is never handed a request
/// that claims to authenticate with nothing. `x-api-key: ""` is therefore a
/// codegen bug on the way to a live 401, and the harness has to be the one to
/// find it.
///
/// A **gateway token under `authorization`** is the other direction and is
/// served: `docs/topics/models.md` documents exactly that composition
/// (`headers: { authorization: "Bearer ${PROXY_TOKEN}" }` on a keyless
/// `kind: anthropic`), so this surface cannot treat the header as a misplaced
/// vendor credential without refusing the shape D120 exists to admit.
#[test]
fn an_empty_api_key_is_refused_while_a_gateway_token_is_served() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(MODEL, Outcome::text("served for the gateway")));

    let body = json!({
        "model": MODEL,
        "max_tokens": 1024,
        "messages": [{ "role": "user", "content": "go" }],
    });
    let response = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .header("x-api-key", "")
                .header("anthropic-version", "2023-06-01")
                .json(&body),
        )
        .expect("the route answers");
    assert_eq!(response.status, 401);
    assert_eq!(response.json()["error"]["type"], "authentication_error");
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    assert_eq!(
        provider.requests()[0]
            .failures()
            .iter()
            .map(|failure| failure.pointer.as_str())
            .collect::<Vec<_>>(),
        ["headers.x-api-key"],
        "the body was well formed: only the credential is wrong"
    );
    assert_eq!(
        provider.snapshot().queues[MODEL],
        1,
        "a request refused for its credential consumes nothing"
    );

    let response = provider
        .client()
        .send(
            Request::post("/v1/messages")
                .header("authorization", "Bearer proxy-token")
                .header("anthropic-version", "2023-06-01")
                .json(&body),
        )
        .expect("the route answers");
    assert_eq!(response.status, 200);
    assert_eq!(
        response.json()["content"][0]["text"],
        "served for the gateway"
    );
    assert!(
        provider.snapshot().queues.is_empty(),
        "the gateway call is the one that consumed the script"
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

/// A provider's `server_tools:` ride the same `tools` array as the agent's own,
/// arrive **verbatim**, and are recorded apart from what the graph dispatches
/// (grammar 12.1, Decision D122).
#[test]
fn a_declared_server_tool_arrives_verbatim_and_is_recorded_as_one() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let web_search = json!({
        "type": "web_search_20250305",
        "name": "web_search",
        "max_uses": 5,
        "allowed_domains": ["docs.example.com"],
    });
    let mut tools = output_schema_tool();
    tools
        .as_array_mut()
        .expect("a tool list")
        .push(web_search.clone());
    let response = send(
        &provider.client(),
        &structured_request(tools, "reviewer_output"),
    );
    assert_eq!(response.status, 200, "{}", response.json());

    let recorded = provider.requests();
    assert!(recorded[0].is_valid(), "{:?}", recorded[0].failures());
    assert_eq!(recorded[0].tools, ["reviewer_output"]);
    assert_eq!(recorded[0].server_tools, ["web_search_20250305"]);
    // Verbatim, field for field: this is the assertion resolved q30's
    // pass-through exists for.
    assert_eq!(recorded[0].body()["tools"][1], web_search);
}

/// The `tools` array is one namespace, and a name in it twice is a 400 whichever
/// side runs the tool (`WIRE-NOTES` (22)).
///
/// Both shapes the compiler's `tool-name-collision` rule now refuses are refused
/// here too, which is what lets the acceptance harness witness that rule rather
/// than take the compiler's word for it: two dated revisions of one server tool
/// carry one canonical `name:`, and a client tool may take a name the
/// connection's suite already spends.
#[test]
fn a_name_the_tools_array_already_carries_is_refused_whichever_side_runs_it() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let mut both_revisions = output_schema_tool();
    let array = both_revisions.as_array_mut().expect("a tool list");
    array.push(json!({ "type": "code_execution_20250522", "name": "code_execution" }));
    array.push(json!({ "type": "code_execution_20250825", "name": "code_execution" }));
    let response = send(
        &client,
        &structured_request(both_revisions, "reviewer_output"),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .expect("a message")
            .contains("Duplicate tool name `code_execution`"),
        "{}",
        response.json()
    );

    let mut against_a_client_tool = output_schema_tool();
    let array = against_a_client_tool.as_array_mut().expect("a tool list");
    array.push(json!({
        "name": "web_search",
        "description": "Search the web the long way round.",
        "input_schema": { "type": "object" },
    }));
    array.push(json!({ "type": "web_search_20250305", "name": "web_search" }));
    let response = send(
        &client,
        &structured_request(against_a_client_tool, "reviewer_output"),
    );
    assert_eq!(response.status, 400);
    assert_eq!(response.header(HARNESS_HEADER), Some(REFUSED_INVALID));

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    assert_eq!(recorded[0].failures().len(), 1);
    assert_eq!(recorded[0].failures()[0].pointer, "tools.2.name");
    assert_eq!(recorded[1].failures().len(), 1);
    assert_eq!(recorded[1].failures()[0].pointer, "tools.2.name");
}

/// A scripted server-tool use comes back as the pair of blocks the Messages wire
/// answers with — the use, and the result the service produced for it — ahead of
/// whatever the model then said. Nothing here is for the graph to run.
#[test]
fn a_scripted_server_tool_use_arrives_already_answered() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })).with_server_tools(
            vec![mock_provider::ServerToolUse::new(
                "web_search_20250305",
                json!({ "query": "agent-compose" }),
                json!([{ "type": "web_search_result", "url": "https://docs.example.com/a" }]),
            )],
        ),
    ));

    let mut tools = output_schema_tool();
    tools.as_array_mut().expect("a tool list").push(json!({
        "type": "web_search_20250305",
        "name": "web_search",
    }));
    let body = send(
        &provider.client(),
        &structured_request(tools, "reviewer_output"),
    )
    .json();
    let content = body["content"].as_array().expect("a content list");
    assert_eq!(
        content.len(),
        3,
        "use, result, then the answer: {content:?}"
    );
    assert_eq!(content[0]["type"], "server_tool_use");
    assert_eq!(content[0]["name"], "web_search");
    assert_eq!(content[1]["type"], "web_search_tool_result");
    assert_eq!(content[1]["tool_use_id"], content[0]["id"]);
    assert_eq!(content[2]["type"], "tool_use");
    assert_eq!(content[2]["name"], "reviewer_output");
}

/// The turn above, replayed back on the next request — which is what a tool loop
/// does — is accepted with its server-tool blocks intact.
#[test]
fn a_replayed_turn_carrying_server_tool_blocks_is_accepted() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::structured(json!({ "verdict": "approve", "feedback": "" })),
    ));

    let mut tools = output_schema_tool();
    tools.as_array_mut().expect("a tool list").push(json!({
        "type": "web_search_20250305",
        "name": "web_search",
    }));
    let request = json!({
        "model": MODEL,
        "max_tokens": 4096,
        "system": "You are a meticulous technical reviewer.",
        "messages": [
            { "role": "user", "content": "look it up" },
            {
                "role": "assistant",
                "content": [
                    { "type": "server_tool_use", "id": "srvtoolu_1", "name": "web_search", "input": { "query": "x" } },
                    { "type": "web_search_tool_result", "tool_use_id": "srvtoolu_1", "content": [] },
                    { "type": "text", "text": "I looked it up." },
                ],
            },
            { "role": "user", "content": "now answer" },
        ],
        "tools": tools,
        "tool_choice": { "type": "tool", "name": "reviewer_output" },
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, 200, "{}", response.json());
    assert!(
        provider.requests()[0].is_valid(),
        "{:?}",
        provider.requests()[0].failures()
    );
}

/// A provider runs only the server tools it was given.
#[test]
fn a_server_tool_the_request_did_not_declare_is_refused() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue(Script::new(
        MODEL,
        Outcome::text("looked it up").with_server_tools(vec![mock_provider::ServerToolUse::new(
            "web_fetch_20250910",
            json!({}),
            json!({}),
        )]),
    ));

    let request = json!({
        "model": MODEL,
        "max_tokens": 4096,
        "messages": [{ "role": "user", "content": "hello" }],
    });
    let response = send(&provider.client(), &request);
    assert_eq!(response.status, HARNESS_STATUS);
    assert!(
        response.json()["error"]["message"]
            .as_str()
            .is_some_and(|message| message.contains("web_fetch_20250910")),
        "{}",
        response.json()
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
