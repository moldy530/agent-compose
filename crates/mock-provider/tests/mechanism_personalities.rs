//! Endpoint personalities: which structured-output mechanisms a model's
//! endpoint carries, and the exact words it refuses the other one with
//! (PRD §9 resolved q53 ruling c, `WIRE-NOTES` (26), (27)).
//!
//! Every other suite in this crate describes **one** endpoint — the vendor's, as
//! it is today, which takes both mechanisms. This one describes the endpoints a
//! compiled graph also has to work against and cannot be told about: a gateway a
//! generation behind the wire it proxies, which 400s the native
//! structured-output parameter, and the newest model generation, which 400s
//! forced tool use and names structured outputs as the replacement.
//!
//! # What is pinned here, and why it is the wording
//!
//! The **refusal texts**, verbatim. The generated runtime decides whether to
//! ladder to the other mechanism by reading the response body, so these
//! sentences are the fixture its recognizer is written against: a mock that
//! refused with wording no service uses would let a recognizer matching nothing
//! pass every test it has, and the ladder would be discovered broken by the
//! first gateway to see it — which is exactly how resolved q52's bug shipped.
//!
//! And the **arithmetic**: a personality refusal takes nothing from a queue, so
//! the retry that follows it finds the scripted answer still there. That is what
//! makes the ladder scriptable at all, and it is a property of this server
//! rather than of any graph, so it is decided here rather than in the acceptance
//! suite.

use mock_provider::{
    Client, HARNESS_HEADER, MockProvider, Outcome, OutputMechanism, Personality, REFUSED_INVALID,
    REFUSED_UNSUPPORTED, Request, Script, StructuredOutput,
};
use serde_json::{Value, json};

const SONNET: &str = "claude-sonnet-4-6";
const GPT: &str = "gpt-5";

/// The agent output schema every request here asks for, closed the way both
/// wires' decoders want it.
fn schema() -> Value {
    json!({
        "type": "object",
        "properties": { "verdict": { "type": "string", "enum": ["approve", "revise"] } },
        "required": ["verdict"],
        "additionalProperties": false,
    })
}

/// The Messages wire asking the native way: `output_config`'s format, and no
/// tool at all.
fn messages_native() -> Value {
    json!({
        "model": SONNET,
        "max_tokens": 4096,
        "system": "You are a meticulous technical reviewer.",
        "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\"}" }],
        "output_config": { "format": { "type": "json_schema", "schema": schema() } },
    })
}

/// The Messages wire asking the other way: the synthetic output tool, pinned.
fn messages_forced() -> Value {
    json!({
        "model": SONNET,
        "max_tokens": 4096,
        "system": "You are a meticulous technical reviewer.",
        "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\"}" }],
        "tools": [{
            "name": "reviewer_output",
            "description": "The agent's output.",
            "input_schema": schema(),
        }],
        "tool_choice": { "type": "tool", "name": "reviewer_output" },
    })
}

/// Chat Completions asking the native way.
fn chat_native() -> Value {
    json!({
        "model": GPT,
        "messages": [
            { "role": "system", "content": "You are a meticulous technical reviewer." },
            { "role": "user", "content": "{\"goal\":\"ship it\"}" },
        ],
        "response_format": {
            "type": "json_schema",
            "json_schema": { "name": "reviewer_output", "strict": true, "schema": schema() },
        },
    })
}

/// Chat Completions asking the other way: the same schema as a forced function.
fn chat_forced() -> Value {
    json!({
        "model": GPT,
        "messages": [
            { "role": "system", "content": "You are a meticulous technical reviewer." },
            { "role": "user", "content": "{\"goal\":\"ship it\"}" },
        ],
        "tools": [{
            "type": "function",
            "function": {
                "name": "reviewer_output",
                "description": "The agent's output.",
                "parameters": schema(),
                "strict": true,
            },
        }],
        "tool_choice": { "type": "function", "function": { "name": "reviewer_output" } },
    })
}

/// The Responses wire asking the native way.
fn responses_native() -> Value {
    json!({
        "model": GPT,
        "instructions": "You are a meticulous technical reviewer.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "{\"goal\":\"ship it\"}" }],
        }],
        "text": {
            "format": {
                "type": "json_schema",
                "name": "reviewer_output",
                "strict": true,
                "schema": schema(),
            },
        },
    })
}

/// The Responses wire asking the other way: a flat forced function.
fn responses_forced() -> Value {
    json!({
        "model": GPT,
        "instructions": "You are a meticulous technical reviewer.",
        "input": [{
            "type": "message",
            "role": "user",
            "content": [{ "type": "input_text", "text": "{\"goal\":\"ship it\"}" }],
        }],
        "tools": [{
            "type": "function",
            "name": "reviewer_output",
            "description": "The agent's output.",
            "parameters": schema(),
            "strict": true,
        }],
        "tool_choice": { "type": "function", "name": "reviewer_output" },
    })
}

fn messages(client: &Client, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post("/v1/messages").anthropic_auth().json(body))
        .expect("the mock provider answers")
}

fn chat(client: &Client, body: &Value) -> mock_provider::Response {
    client
        .send(
            Request::post("/v1/chat/completions")
                .openai_auth()
                .json(body),
        )
        .expect("the mock provider answers")
}

fn responses(client: &Client, body: &Value) -> mock_provider::Response {
    client
        .send(Request::post("/v1/responses").openai_auth().json(body))
        .expect("the mock provider answers")
}

/// The Anthropic error envelope's message, or a panic naming what came instead.
fn anthropic_message(response: &mock_provider::Response) -> String {
    let body = response.json();
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("an Anthropic error envelope, got {body}"))
        .to_string()
}

/// The OpenAI error envelope's message, which the Responses route borrows.
fn openai_message(response: &mock_provider::Response) -> String {
    let body = response.json();
    body["error"]["message"]
        .as_str()
        .unwrap_or_else(|| panic!("an OpenAI error envelope, got {body}"))
        .to_string()
}

/// Both mechanisms answer where nothing has staged otherwise — the endpoint
/// every other suite in this crate is talking to.
#[test]
fn the_default_endpoint_carries_both_mechanisms() {
    let provider = MockProvider::start().expect("a port");
    provider.enqueue_all([
        Script::new(SONNET, Outcome::structured(json!({ "verdict": "approve" }))),
        Script::new(SONNET, Outcome::structured(json!({ "verdict": "revise" }))),
    ]);

    let client = provider.client();
    assert_eq!(messages(&client, &messages_native()).status, 200);
    assert_eq!(messages(&client, &messages_forced()).status, 200);

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
        "the two mechanisms are told apart by the shape of the request"
    );
    assert!(
        recorded.iter().all(|request| !request.was_unsupported()),
        "…and nothing was refused for its mechanism"
    );
    assert!(provider.snapshot().is_drained());
}

/// The **lagging gateway**: `output_config` is an argument it has never heard
/// of, and the forced tool works.
#[test]
fn a_native_rejecting_endpoint_refuses_output_config_in_the_apis_own_words() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::NativeRejected);
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let client = provider.client();
    let refused = messages(&client, &messages_native());
    assert_eq!(
        refused.status, 400,
        "a refusal, in the provider's own shape"
    );
    assert_eq!(
        anthropic_message(&refused),
        "output_config: Extra inputs are not permitted",
        "the unknown-argument family, in the Messages API's pydantic wording"
    );

    // …and the same call the other way is answered, out of the queue the
    // refusal did not touch.
    let answered = messages(&client, &messages_forced());
    assert_eq!(answered.status, 200);
    assert_eq!(
        answered.json()["content"][0]["input"],
        json!({ "verdict": "approve" })
    );

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 2);
    assert!(
        recorded[0].is_valid(),
        "the refused request was **well formed** — what it asked for is not a \
         codegen bug: {:?}",
        recorded[0].failures()
    );
    assert_eq!(recorded[0].unsupported, Some(OutputMechanism::Native));
    assert_eq!(recorded[0].served, "unsupported");
    assert_eq!(recorded[1].unsupported, None);
    let snapshot = provider.snapshot();
    assert_eq!(snapshot.unsupported, 1, "counted");
    assert_eq!(snapshot.invalid, 0, "…and not as a malformed request");
    assert!(
        snapshot.is_drained(),
        "a run that laddered exactly as it was asked to is a clean run: {snapshot:?}"
    );
}

/// The **newest generation**: forced tool use is gone and the native parameter
/// works — the mirror image, on the same wire.
#[test]
fn a_forced_tool_removed_endpoint_refuses_the_pin_in_the_apis_own_words() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::ForcedToolRemoved);
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "revise" })),
    ));

    let client = provider.client();
    let refused = messages(&client, &messages_forced());
    assert_eq!(refused.status, 400);
    assert_eq!(
        anthropic_message(&refused),
        "tool_choice: type \"tool\" and \"any\" are not supported for this model.",
        "the model generation that removed forced tool use, in its own words"
    );

    let answered = messages(&client, &messages_native());
    assert_eq!(answered.status, 200);
    let text = answered.json()["content"][0]["text"]
        .as_str()
        .expect("the format shaped the assistant's text")
        .to_string();
    assert_eq!(
        serde_json::from_str::<Value>(&text).expect("which parses"),
        json!({ "verdict": "revise" })
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded[0].unsupported,
        Some(OutputMechanism::ForcedTool),
        "the mechanism this endpoint refused is on the record"
    );
    assert!(provider.snapshot().is_drained());
}

/// Neither — the endpoint the double-refusal diagnostic exists for. Both calls
/// are refused, and neither takes the scripted answer.
#[test]
fn a_both_rejecting_endpoint_refuses_either_mechanism_and_eats_no_script() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::BothRejected);
    provider.enqueue(Script::new(
        SONNET,
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let client = provider.client();
    assert_eq!(messages(&client, &messages_native()).status, 400);
    assert_eq!(messages(&client, &messages_forced()).status, 400);

    let snapshot = provider.snapshot();
    assert_eq!(snapshot.unsupported, 2);
    assert_eq!(
        snapshot.queues[SONNET], 1,
        "the scripted answer is still there: a mechanism refusal takes nothing, \
         which is what keeps one bad rung from eating the answer meant for the \
         other: {snapshot:?}"
    );
}

/// The OpenAI wires carry the same two rungs and their own two sentences.
#[test]
fn the_openai_wires_refuse_each_mechanism_in_their_own_words() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(GPT, Personality::BothRejected);

    let client = provider.client();
    assert_eq!(
        openai_message(&chat(&client, &chat_native())),
        "Unrecognized request argument supplied: response_format",
    );
    assert_eq!(
        openai_message(&chat(&client, &chat_forced())),
        "Invalid parameter: 'tool_choice' of type 'function' is not supported with this model.",
    );
    assert_eq!(
        openai_message(&responses(&client, &responses_native())),
        "Invalid parameter: 'text.format' of type 'json_schema' is not supported with this model.",
    );
    assert_eq!(
        openai_message(&responses(&client, &responses_forced())),
        "Invalid parameter: 'tool_choice' of type 'function' is not supported with this model.",
    );

    let recorded = provider.requests();
    assert_eq!(
        recorded
            .iter()
            .map(|request| request.unsupported)
            .collect::<Vec<_>>(),
        [
            Some(OutputMechanism::Native),
            Some(OutputMechanism::ForcedTool),
            Some(OutputMechanism::Native),
            Some(OutputMechanism::ForcedTool),
        ],
        "each request is recorded against the mechanism it asked through"
    );
    assert!(
        recorded
            .iter()
            .all(mock_provider::RecordedRequest::is_valid),
        "…and every one of them was well formed"
    );
}

/// What a mechanism refusal says **on the wire** about whose bug it is:
/// `unsupported-mechanism`, never `invalid-request`.
///
/// The status is the provider's own 400 either way and the envelope is the
/// surface's, so the `x-mock-provider-error` value is the only thing in a raw
/// exchange that tells a staged endpoint from a request this server could not
/// parse — and the transcript records this one as **valid** beside
/// `unsupported: Some(_)`, so the two had to be made to agree. Labelled
/// `invalid-request`, a laddering run would send a contributor reading its
/// headers after a codegen bug that does not exist, and would falsify the
/// natural spelling of "every request in this run was composed correctly".
#[test]
fn a_mechanism_refusal_is_labelled_apart_from_a_malformed_request() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::BothRejected);
    provider.personality(GPT, Personality::BothRejected);

    let client = provider.client();
    let staged = [
        messages(&client, &messages_native()),
        messages(&client, &messages_forced()),
        chat(&client, &chat_forced()),
        responses(&client, &responses_native()),
    ];
    for refused in &staged {
        assert_eq!(refused.status, 400, "the provider's own bad request");
        assert_eq!(
            refused.header(HARNESS_HEADER),
            Some(REFUSED_UNSUPPORTED),
            "…and the label that says the endpoint is being what a test staged"
        );
    }

    // The contrast, on the same wire and at the same status: a request this
    // server could not parse at all. `output_format` is q53's *deprecated*
    // spelling of the very parameter above ((26)), so it is also the nearest
    // neighbour a mislabelling could hide behind.
    let malformed = messages(
        &client,
        &json!({
            "model": SONNET,
            "max_tokens": 4096,
            "messages": [{ "role": "user", "content": "hi" }],
            "output_format": { "type": "json_schema", "schema": schema() },
        }),
    );
    assert_eq!(malformed.status, 400);
    assert_eq!(
        malformed.header(HARNESS_HEADER),
        Some(REFUSED_INVALID),
        "a malformed request keeps the label it has always had"
    );

    let recorded = provider.requests();
    assert_eq!(recorded.len(), 5);
    let valid = recorded
        .iter()
        .map(mock_provider::RecordedRequest::is_valid)
        .collect::<Vec<_>>();
    assert_eq!(
        valid,
        [true, true, true, true, false],
        "…which is what the transcript says of each, and the whole reason the \
         four staged refusals may not wear `invalid-request`"
    );
}

/// A personality says nothing about a call that asks for **no** object: a tool
/// loop's calls carry neither mechanism, so there is nothing to refuse.
#[test]
fn a_call_asking_for_no_object_is_untouched_by_any_personality() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::BothRejected);
    provider.enqueue(Script::new(SONNET, Outcome::text("thinking about it")));

    let loop_call = json!({
        "model": SONNET,
        "max_tokens": 4096,
        "system": "You are a researcher.",
        "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\"}" }],
        "tools": [{
            "name": "lookup",
            "description": "Look one thing up.",
            "input_schema": { "type": "object", "properties": {}, "additionalProperties": false },
        }],
    });
    let answered = messages(&provider.client(), &loop_call);
    assert_eq!(answered.status, 200, "{}", answered.text());
    assert_eq!(provider.requests()[0].unsupported, None);
    assert!(provider.snapshot().is_drained());
}

/// The personality is a property of the **model's endpoint**, and one model's
/// is not another's.
#[test]
fn a_personality_applies_to_its_own_model_only() {
    let provider = MockProvider::start().expect("a port");
    provider.personality(SONNET, Personality::NativeRejected);
    provider.enqueue(Script::new(
        "claude-haiku-4-5",
        Outcome::structured(json!({ "verdict": "approve" })),
    ));

    let mut other = messages_native();
    other["model"] = json!("claude-haiku-4-5");
    let answered = messages(&provider.client(), &other);
    assert_eq!(
        answered.status,
        200,
        "the other model's endpoint carries both: {}",
        answered.text()
    );
    assert!(provider.snapshot().is_drained());
}

/// The control plane and the in-process handle are one store, and `reset`
/// forgets a staged endpoint along with everything else.
#[test]
fn the_control_plane_stages_and_resets_a_personality() {
    let provider = MockProvider::start().expect("a port");
    let client = provider.client();

    let staged = client
        .post_json(
            "/_mock/personality",
            &json!({ "model": SONNET, "personality": "forced_tool_removed" }),
        )
        .expect("the control plane accepts an endpoint");
    assert_eq!(staged.status, 200);
    assert_eq!(staged.json()["registered"], 1);
    assert_eq!(
        staged.json()["state"]["personalities"][SONNET],
        "forced_tool_removed"
    );

    assert_eq!(
        messages(&client, &messages_forced()).status,
        400,
        "what was staged over HTTP is what the wire does"
    );

    provider.reset();
    let state = client
        .get("/_mock/state")
        .expect("the control plane answers");
    assert!(
        state.json()["personalities"].is_null(),
        "a reset endpoint is the default one again: {}",
        state.json()
    );

    // A list, and an unknown spelling, on the same route.
    let listed = client
        .post_json(
            "/_mock/personality",
            &json!([
                { "model": SONNET, "personality": "native_rejected" },
                { "model": GPT, "personality": "both_rejected" },
            ]),
        )
        .expect("the control plane accepts a list");
    assert_eq!(listed.json()["registered"], 2);
    let refused = client
        .post_json(
            "/_mock/personality",
            &json!({ "model": SONNET, "personality": "whatever_i_like" }),
        )
        .expect("the control plane answers");
    assert_eq!(
        refused.status,
        mock_provider::HARNESS_STATUS,
        "a personality this server does not model is a harness error, not a wire one"
    );
}
