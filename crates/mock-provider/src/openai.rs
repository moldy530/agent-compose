//! The OpenAI Chat Completions surface: `POST /v1/chat/completions`, and the
//! Azure routes that carry the same body.
//!
//! Three of grammar 12.1's six provider kinds reach this surface — `openai`,
//! `openai_compatible` (ollama, vLLM, proxies), and `azure_openai` — because
//! they differ in *connection*, which is exactly the layer PRD 5.9 split off
//! from behavior. Azure differs on the wire only in route and auth header:
//! `POST /openai/deployments/<deployment>/chat/completions?api-version=…` with
//! an `api-key:` header, or the newer `POST /openai/v1/chat/completions`. The
//! body is the same body, so it is checked by the same code.
//!
//! # Structured output, two ways
//!
//! Both are accepted because both are what the LangChain JS integration sends,
//! depending on the `method` it is configured with:
//!
//! * `response_format: { type: "json_schema", json_schema: { name, schema } }`,
//!   answered with the object serialized into the assistant message's content;
//! * a forced function (`tool_choice: { type: "function", function: { name } }`),
//!   answered with a tool call whose `arguments` is the object serialized as a
//!   **string** — which is the shape mistake this surface most wants to catch in
//!   the other direction too, on requests that send `arguments` as an object.
//!
//! `WIRE-NOTES.md` records which of these could not be confirmed offline.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::control::{
    Failure, Outcome, Reply, ReplyBody, StructuredOutput, ValidationFailure, canonical, estimate,
};
use crate::strict::{Checker, Dialect, Kind, at};
use crate::wire::{Answer, HARNESS_STATUS, INVALID, MISMATCH, Response, UNSCRIPTED};

/// The top-level keys this surface accepts.
///
/// `top_k` is on the list even though `api.openai.com` refuses it: this route
/// serves all three of grammar 12.1's Chat-Completions kinds, and
/// `openai_compatible` (vLLM, ollama) accepts `top_k` — which is also why
/// grammar 12.2 lists it in the typed `settings:` vocabulary. The mock cannot
/// tell the kinds apart from one route, and refusing a key one of them accepts
/// would fail a fixture that is correct. `WIRE-NOTES.md` records the choice.
const REQUEST_KEYS: &[&str] = &[
    "model",
    "messages",
    "tools",
    "tool_choice",
    "response_format",
    "max_tokens",
    "max_completion_tokens",
    "temperature",
    "top_p",
    "top_k",
    "n",
    "stop",
    "seed",
    "stream",
    "stream_options",
    "user",
    "parallel_tool_calls",
    "presence_penalty",
    "frequency_penalty",
    "logprobs",
    "top_logprobs",
    "logit_bias",
    "reasoning_effort",
    "metadata",
    "store",
    "service_tier",
];

/// The roles this surface accepts.
const ROLES: &[&str] = &["system", "developer", "user", "assistant", "tool"];

/// What the surface understood about a request.
pub(crate) struct Parsed {
    pub(crate) model: String,
    pub(crate) failures: Vec<ValidationFailure>,
    pub(crate) tools: Vec<String>,
    pub(crate) structured_output: Option<StructuredOutput>,
}

/// How the request reached this surface, which is all Azure changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Route {
    /// `POST /v1/chat/completions`, `Authorization: Bearer …`.
    Direct,
    /// An Azure route: `api-key:` (or a bearer token), and an `api-version`
    /// query parameter the service requires.
    Azure,
}

/// Read and check one request.
pub(crate) fn parse(
    route: Route,
    headers: &BTreeMap<String, String>,
    query: &str,
    deployment: Option<&str>,
    body: Option<&Value>,
) -> Parsed {
    let mut checker = Checker::new(Dialect::OpenAi);
    check_headers(&mut checker, route, headers);
    if route == Route::Azure
        && !query
            .split('&')
            .any(|pair| pair.starts_with("api-version="))
    {
        checker.fail(
            "query.api-version",
            "Missing required parameter: 'api-version'.",
        );
    }

    let Some(body) = body.and_then(Value::as_object) else {
        checker.fail(
            "",
            "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly.)",
        );
        return Parsed {
            model: deployment.unwrap_or_default().to_string(),
            failures: checker.into_failures(),
            tools: Vec::new(),
            structured_output: None,
        };
    };

    checker.closed("", body, REQUEST_KEYS);

    // On the **classic** Azure route the deployment in the path is what selects
    // the model, so the body's `model` is optional there and the deployment
    // stands in for it. Everywhere else — including the newer
    // `/openai/v1/chat/completions`, which carries no deployment and where the
    // body names the model (WIRE-NOTES §7) — it is required, and a request that
    // omits it is refused rather than keyed under the empty string.
    let model = match (route, deployment, body.get("model")) {
        (Route::Azure, Some(deployment), None) => deployment.to_string(),
        _ => checker
            .required_string("", body, "model")
            .unwrap_or_default()
            .to_string(),
    };
    // A present `model` that names nothing is its own mistake, asked separately
    // so it cannot double up on a missing or mistyped one.
    if body.get("model").and_then(Value::as_str) == Some("") {
        checker.fail("model", "Invalid value: ''. 'model' must name a model.");
    }

    let tools = check_tools(&mut checker, body);
    let forced = check_tool_choice(&mut checker, body, &tools);
    check_messages(&mut checker, body, &tools);
    let response_format = check_response_format(&mut checker, body);
    check_streaming(&mut checker, body);

    let structured_output = response_format.or_else(|| {
        let name = forced?;
        let schema = function_schema(body, &name)?;
        Some(StructuredOutput::ForcedFunction { name, schema })
    });

    Parsed {
        model,
        failures: checker.into_failures(),
        tools,
        structured_output,
    }
}

fn check_headers(checker: &mut Checker, route: Route, headers: &BTreeMap<String, String>) {
    let value = |name: &str| headers.get(name).filter(|value| !value.is_empty());
    let bearer = value("authorization").is_some_and(|value| value.starts_with("Bearer "));
    // A credential, not a field: answered 401 rather than 400 (see `rejected`),
    // because that is the status the `openai` SDK's `AuthenticationError` comes
    // from and generated code may well classify the two apart.
    match route {
        Route::Direct if !bearer => checker.credential(
            "headers.authorization",
            "You didn't provide an API key. You need to provide your API key in an Authorization header using Bearer auth (i.e. Authorization: Bearer YOUR_KEY).",
        ),
        Route::Azure if !bearer && value("api-key").is_none() => checker.credential(
            "headers.api-key",
            "Access denied due to missing subscription key. Make sure to include subscription key when making requests to an API.",
        ),
        _ => {}
    }
    let json = headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/json"));
    if !json {
        checker.fail(
            "headers.content-type",
            "Invalid content type. Expected 'application/json'.",
        );
    }
}

/// The tool surface, in request order.
fn check_tools(checker: &mut Checker, body: &Map<String, Value>) -> Vec<String> {
    let mut names = Vec::new();
    let Some(tools) = checker.optional("", body, "tools", Kind::Array) else {
        return names;
    };
    // An empty list is refused, not ignored: `tools: []` is what codegen emits
    // for an agent with no `tools:` and no `stores:` if it always writes the
    // key, and the service answers that with a 400.
    if tools.as_array().is_some_and(Vec::is_empty) {
        checker.fail(
            "tools",
            "Invalid 'tools': empty array. Expected an array with minimum length 1.",
        );
        return names;
    }
    let mut seen = BTreeSet::new();
    for (index, tool) in tools.as_array().into_iter().flatten().enumerate() {
        let pointer = at("tools", index);
        let Some(tool) = checker
            .typed(&pointer, tool, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        checker.closed(&pointer, tool, &["type", "function"]);
        if let Some(kind) = checker.required(&pointer, tool, "type") {
            checker.one_of(&at(&pointer, "type"), kind, &["function"]);
        }
        let Some(function) = checker.required_object(&pointer, tool, "function") else {
            continue;
        };
        let pointer = at(&pointer, "function");
        checker.closed(
            &pointer,
            function,
            &["name", "description", "parameters", "strict"],
        );
        let Some(name) = checker.required_string(&pointer, function, "name") else {
            continue;
        };
        let name = name.to_string();
        if !well_formed(&name) {
            checker.fail(
                &at(&pointer, "name"),
                format!(
                    "Invalid 'tools[{index}].function.name': string does not match pattern. Expected a string that matches the pattern '^[a-zA-Z0-9_-]+$'."
                ),
            );
        }
        if !seen.insert(name.clone()) {
            checker.fail(
                &at(&pointer, "name"),
                format!("Invalid 'tools': duplicate function name '{name}'."),
            );
        }
        let parameters = checker.optional(&pointer, function, "parameters", Kind::Object);
        if let Some(parameters) = parameters
            && parameters.get("type").and_then(Value::as_str) != Some("object")
        {
            checker.fail(
                &at(&pointer, "parameters.type"),
                format!(
                    "Invalid schema for function '{name}': schema must be a JSON Schema of 'type: \"object\"'."
                ),
            );
        }
        let strict = checker
            .optional(&pointer, function, "strict", Kind::Boolean)
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if let (true, Some(parameters)) = (strict, parameters) {
            check_strict_schema(
                checker,
                &at(&pointer, "parameters"),
                &format!("function '{name}'"),
                parameters,
            );
        }
        names.push(name);
    }
    names
}

/// `tool_choice`, and the function it forces if it forces one.
fn check_tool_choice(
    checker: &mut Checker,
    body: &Map<String, Value>,
    tools: &[String],
) -> Option<String> {
    let choice = body.get("tool_choice")?;
    // The same rule the Messages surface keeps: `tool_choice` says how the model
    // must use its tools, so a request that carries one without a tool surface
    // is refused — once, at the missing surface, rather than at a name that
    // could not have been in it.
    if !body.contains_key("tools") {
        checker.fail(
            "tool_choice",
            "Invalid value: 'tool_choice' may only be specified when 'tools' are provided.",
        );
        return None;
    }
    match choice {
        Value::String(_) => {
            checker.one_of("tool_choice", choice, &["none", "auto", "required"]);
            None
        }
        Value::Object(choice) => {
            checker.closed("tool_choice", choice, &["type", "function"]);
            if let Some(kind) = checker.required("tool_choice", choice, "type") {
                checker.one_of("tool_choice.type", kind, &["function"]);
            }
            let function = checker.required_object("tool_choice", choice, "function")?;
            checker.closed("tool_choice.function", function, &["name"]);
            let name = checker
                .required_string("tool_choice.function", function, "name")?
                .to_string();
            if !tools.contains(&name) {
                checker.fail(
                    "tool_choice.function.name",
                    format!("Invalid value: '{name}'. Supported values are the names in 'tools'."),
                );
                return None;
            }
            Some(name)
        }
        other => {
            checker.typed("tool_choice", other, Kind::Object);
            None
        }
    }
}

/// The schema a named function declares.
fn function_schema(body: &Map<String, Value>, name: &str) -> Option<Value> {
    body.get("tools")?
        .as_array()?
        .iter()
        .map(|tool| tool.get("function").unwrap_or(&Value::Null))
        .find(|function| function.get("name").and_then(Value::as_str) == Some(name))?
        .get("parameters")
        .cloned()
}

/// `response_format`, and the structured output it asks for.
fn check_response_format(
    checker: &mut Checker,
    body: &Map<String, Value>,
) -> Option<StructuredOutput> {
    let format = checker.optional("", body, "response_format", Kind::Object)?;
    let format = format.as_object()?;
    checker.closed("response_format", format, &["type", "json_schema"]);
    let kind = checker.required("response_format", format, "type")?;
    let kind = checker
        .one_of(
            "response_format.type",
            kind,
            &["text", "json_object", "json_schema"],
        )?
        .to_string();
    if kind != "json_schema" {
        if format.contains_key("json_schema") {
            checker.fail(
                "response_format.json_schema",
                format!("Unrecognized request argument supplied: response_format.json_schema (with type '{kind}')"),
            );
        }
        return None;
    }
    let schema = checker.required_object("response_format", format, "json_schema")?;
    checker.closed(
        "response_format.json_schema",
        schema,
        &["name", "schema", "strict", "description"],
    );
    let name = checker
        .required_string("response_format.json_schema", schema, "name")?
        .to_string();
    let declared = checker
        .required_object("response_format.json_schema", schema, "schema")
        .cloned()?;
    let strict = schema
        .get("strict")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let declared = Value::Object(declared);
    if strict {
        check_strict_schema(
            checker,
            "response_format.json_schema.schema",
            &format!("response_format '{name}'"),
            &declared,
        );
    }
    Some(StructuredOutput::JsonSchema {
        name,
        schema: declared,
        strict,
    })
}

/// The rules `strict: true` adds to a schema, wherever it was asked for.
///
/// `strict` is not a hint. The service compiles the schema into a constrained
/// decoder, and it can only do that over a **closed** shape: every object must
/// carry `additionalProperties: false` and list every one of its properties in
/// `required`. A schema that does not is a 400 — the most common one on this
/// surface — so a Zod-to-JSON-Schema path that drops either key must fail here
/// rather than on the first live call, which is the whole reason this server
/// validates at all.
///
/// The complaint is the service's own, `context=` and all: it addresses the
/// offending object by its path *inside the schema*, which is the only address
/// that means anything once the schema nests.
fn check_strict_schema(checker: &mut Checker, pointer: &str, subject: &str, schema: &Value) {
    walk_strict_schema(checker, pointer, subject, &mut Vec::new(), schema);
}

fn walk_strict_schema(
    checker: &mut Checker,
    pointer: &str,
    subject: &str,
    context: &mut Vec<String>,
    schema: &Value,
) {
    let Some(object) = schema.as_object() else {
        return;
    };
    let properties = object.get("properties").and_then(Value::as_object);
    if object.get("type").and_then(Value::as_str) == Some("object") {
        let printed = printed_context(context);
        if object.get("additionalProperties") != Some(&Value::Bool(false)) {
            checker.fail(
                pointer,
                format!(
                    "Invalid schema for {subject}: In context={printed}, 'additionalProperties' is required to be supplied and to be false."
                ),
            );
        }
        let required: BTreeSet<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|listed| listed.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        // One complaint per object, naming the first property left out — the
        // way the service reports it, and enough to send whoever reads it to
        // the object that is wrong.
        if let Some(missing) = properties
            .into_iter()
            .flatten()
            .map(|(name, _)| name)
            .find(|name| !required.contains(name.as_str()))
        {
            checker.fail(
                pointer,
                format!(
                    "Invalid schema for {subject}: In context={printed}, 'required' is required to be supplied and to be an array including every key in properties. Missing '{missing}'."
                ),
            );
        }
    }

    // Every place a subschema can hide. The rules apply to all of them, because
    // the decoder has to be constrained all the way down.
    for (name, subschema) in properties.into_iter().flatten() {
        context.push("properties".to_string());
        context.push(name.clone());
        walk_strict_schema(checker, pointer, subject, context, subschema);
        context.truncate(context.len() - 2);
    }
    for (key, subschema) in [("items", object.get("items"))] {
        if let Some(subschema) = subschema {
            context.push(key.to_string());
            walk_strict_schema(checker, pointer, subject, context, subschema);
            context.pop();
        }
    }
    for (key, group) in [("$defs", object.get("$defs"))] {
        for (name, subschema) in group.and_then(Value::as_object).into_iter().flatten() {
            context.push(key.to_string());
            context.push(name.clone());
            walk_strict_schema(checker, pointer, subject, context, subschema);
            context.truncate(context.len() - 2);
        }
    }
    for (index, subschema) in object
        .get("anyOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .enumerate()
    {
        context.push("anyOf".to_string());
        context.push(index.to_string());
        walk_strict_schema(checker, pointer, subject, context, subschema);
        context.truncate(context.len() - 2);
    }
}

/// A schema path as the service prints it: a Python tuple, `()` at the root.
fn printed_context(context: &[String]) -> String {
    let members = context
        .iter()
        .map(|member| format!("'{member}'"))
        .collect::<Vec<_>>()
        .join(", ");
    match context.len() {
        0 => "()".to_string(),
        1 => format!("({members},)"),
        _ => format!("({members})"),
    }
}

/// The message list, including the `tool` correlation rule.
fn check_messages(checker: &mut Checker, body: &Map<String, Value>, tools: &[String]) {
    let Some(messages) = checker.required_array("", body, "messages") else {
        return;
    };
    if messages.is_empty() {
        checker.fail(
            "messages",
            "Invalid 'messages': empty array. Expected an array with minimum length 1.",
        );
        return;
    }

    // The ids the most recent assistant turn is waiting on, and which turn that
    // was — a dropped result is reported at the message that asked for it, which
    // is where the author has to go to fix it.
    let mut awaiting: Vec<String> = Vec::new();
    let mut asked_at = 0;

    for (index, message) in messages.iter().enumerate() {
        let pointer = at("messages", index);
        let Some(message) = checker
            .typed(&pointer, message, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(role) = checker.required(&pointer, message, "role") else {
            continue;
        };
        let Some(role) = checker
            .one_of(&at(&pointer, "role"), role, ROLES)
            .map(str::to_string)
        else {
            continue;
        };

        // A turn that is not the answer to the pending tool calls means those
        // calls were dropped — the tool loop losing a result.
        if role != "tool" && !awaiting.is_empty() {
            unanswered(checker, asked_at, &awaiting);
            awaiting.clear();
        }

        match role.as_str() {
            "system" | "developer" | "user" => {
                checker.closed(&pointer, message, &["role", "content", "name"]);
                if let Some(content) = checker.required(&pointer, message, "content") {
                    check_content(checker, &at(&pointer, "content"), content);
                }
            }
            "assistant" => {
                checker.closed(
                    &pointer,
                    message,
                    &["role", "content", "name", "tool_calls", "refusal"],
                );
                let calls = check_tool_calls(checker, &pointer, message, tools);
                let has_content = message
                    .get("content")
                    .is_some_and(|content| !content.is_null());
                if has_content {
                    check_content(checker, &at(&pointer, "content"), &message["content"]);
                }
                if !has_content && calls.is_empty() {
                    checker.fail(
                        &pointer,
                        format!(
                            "Invalid 'messages[{index}]': assistant message must carry 'content' or 'tool_calls'."
                        ),
                    );
                }
                awaiting = calls;
                asked_at = index;
            }
            "tool" => {
                checker.closed(&pointer, message, &["role", "content", "tool_call_id"]);
                if let Some(content) = checker.required(&pointer, message, "content") {
                    check_content(checker, &at(&pointer, "content"), content);
                }
                let Some(answers) = checker.required_string(&pointer, message, "tool_call_id")
                else {
                    continue;
                };
                let answers = answers.to_string();
                if let Some(position) = awaiting.iter().position(|id| *id == answers) {
                    awaiting.remove(position);
                } else {
                    checker.fail(
                        &at(&pointer, "tool_call_id"),
                        format!(
                            "Invalid parameter: messages with role 'tool' must be a response to a preceding message with 'tool_calls'; '{answers}' answers no pending call."
                        ),
                    );
                }
            }
            _ => unreachable!("`one_of` admitted only the roles above"),
        }
    }

    if !awaiting.is_empty() {
        unanswered(checker, asked_at, &awaiting);
    }
}

/// The assistant turn at `message` asked for these ids and never got them.
fn unanswered(checker: &mut Checker, message: usize, ids: &[String]) {
    let list = ids
        .iter()
        .map(|id| format!("'{id}'"))
        .collect::<Vec<_>>()
        .join(", ");
    checker.fail(
        &at("messages", message),
        format!(
            "An assistant message with 'tool_calls' must be followed by tool messages responding to each 'tool_call_id'. The following tool_call_ids did not have response messages: {list}"
        ),
    );
}

/// An assistant turn's `tool_calls`, returning the ids it is waiting on.
fn check_tool_calls(
    checker: &mut Checker,
    pointer: &str,
    message: &Map<String, Value>,
    tools: &[String],
) -> Vec<String> {
    let mut ids = Vec::new();
    let Some(calls) = checker.optional(pointer, message, "tool_calls", Kind::Array) else {
        return ids;
    };
    for (index, call) in calls.as_array().into_iter().flatten().enumerate() {
        let pointer = at(&at(pointer, "tool_calls"), index);
        let Some(call) = checker
            .typed(&pointer, call, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        checker.closed(&pointer, call, &["id", "type", "function"]);
        if let Some(kind) = checker.required(&pointer, call, "type") {
            checker.one_of(&at(&pointer, "type"), kind, &["function"]);
        }
        if let Some(id) = checker.required_string(&pointer, call, "id") {
            ids.push(id.to_string());
        }
        let Some(function) = checker.required_object(&pointer, call, "function") else {
            continue;
        };
        let pointer = at(&pointer, "function");
        checker.closed(&pointer, function, &["name", "arguments"]);
        if let Some(name) = checker.required_string(&pointer, function, "name")
            && !tools.is_empty()
            && !tools.iter().any(|tool| tool == name)
        {
            checker.fail(
                &at(&pointer, "name"),
                format!("Invalid value: '{name}'. This message calls a function the request does not define."),
            );
        }
        // The shape mistake this surface exists to catch: `arguments` travels as
        // a JSON *string*, not as an object, and a client that sends the object
        // is a client the real API refuses.
        if let Some(arguments) = checker.required(&pointer, function, "arguments") {
            let pointer = at(&pointer, "arguments");
            if let Some(text) = checker
                .typed(&pointer, arguments, Kind::String)
                .and_then(Value::as_str)
                && serde_json::from_str::<Value>(text).is_err()
            {
                checker.fail(
                    &pointer,
                    format!("Invalid '{pointer}': expected a JSON-encoded string."),
                );
            }
        }
    }
    ids
}

/// Message content: a string, or a list of parts.
fn check_content(checker: &mut Checker, pointer: &str, content: &Value) {
    match content {
        Value::String(_) => {}
        Value::Array(parts) => {
            if parts.is_empty() {
                checker.fail(
                    pointer,
                    format!(
                        "Invalid '{pointer}': empty array. Expected an array with minimum length 1."
                    ),
                );
            }
            for (index, part) in parts.iter().enumerate() {
                let pointer = at(pointer, index);
                let Some(part) = checker
                    .typed(&pointer, part, Kind::Object)
                    .and_then(Value::as_object)
                else {
                    continue;
                };
                let Some(kind) = checker.required(&pointer, part, "type") else {
                    continue;
                };
                let Some(kind) = checker
                    .one_of(
                        &at(&pointer, "type"),
                        kind,
                        &["text", "image_url", "input_audio"],
                    )
                    .map(str::to_string)
                else {
                    continue;
                };
                if kind == "text" {
                    checker.closed(&pointer, part, &["type", "text"]);
                    checker.required_string(&pointer, part, "text");
                }
            }
        }
        other => {
            checker.typed(pointer, other, Kind::String);
        }
    }
}

fn check_streaming(checker: &mut Checker, body: &Map<String, Value>) {
    if body.get("stream") == Some(&Value::Bool(true)) {
        checker.fail(
            "stream",
            "stream: the mock provider does not implement streaming responses (WIRE-NOTES.md); \
             a compiled graph must invoke the model, not stream it.",
        );
    }
}

fn well_formed(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name.chars().all(|character| {
            character.is_ascii_alphanumeric() || character == '_' || character == '-'
        })
}

/// Render a served outcome into the Chat Completions wire shape.
pub(crate) fn render(
    sequence: u64,
    request: &Value,
    model: &str,
    structured: Option<&StructuredOutput>,
    outcome: &Outcome,
) -> Answer {
    match outcome {
        Outcome::Failure(failure) => failure_answer(failure),
        Outcome::Raw(raw) => {
            let mut response = Response::new(raw.status, raw.body.clone()).after(raw.delay);
            for (name, value) in &raw.headers {
                response = response.header(name, value.clone());
            }
            response.answer()
        }
        Outcome::Reply(reply) => reply_answer(sequence, request, model, structured, reply),
    }
}

fn reply_answer(
    sequence: u64,
    request: &Value,
    model: &str,
    structured: Option<&StructuredOutput>,
    reply: &Reply,
) -> Answer {
    let offered: Vec<String> = request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.pointer("/function/name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let (message, finish) = match &reply.body {
        ReplyBody::Text(text) => {
            // The inverse of the `structured`-without-a-request-for-one refusal
            // below: a request that pinned the *shape* of the answer cannot be
            // answered with free prose, so a script that says otherwise is a
            // harness bug and is answered as one.
            if let Some(pinned) = pinned_answer_shape(request) {
                return mismatch(&format!(
                    "a `text` reply cannot answer a request carrying {pinned}: Chat Completions \
                     answers that with parseable JSON or a tool call, never with free prose. \
                     Script `structured` for the object the agent should produce, or `raw` for a \
                     response generated code must reject"
                ));
            }
            (
                json!({ "role": "assistant", "content": text, "refusal": Value::Null }),
                "stop",
            )
        }
        ReplyBody::Structured(value) => match structured {
            Some(StructuredOutput::JsonSchema { .. }) => (
                json!({
                    "role": "assistant",
                    "content": canonical(value),
                    "refusal": Value::Null,
                }),
                "stop",
            ),
            Some(StructuredOutput::ForcedFunction { name, .. }) => (
                json!({
                    "role": "assistant",
                    "content": Value::Null,
                    "tool_calls": [call_block(sequence, 0, name, value)],
                    "refusal": Value::Null,
                }),
                "tool_calls",
            ),
            _ => {
                return mismatch(
                    "a `structured` reply needs a request that asks for structured output: this \
                     one carries neither `response_format: {type: \"json_schema\"}` nor a forced \
                     function",
                );
            }
        },
        ReplyBody::Tools {
            calls: scripted,
            text,
        } => {
            // The mirror of the two refusals above, in the other direction: a
            // request may make a tool call *impossible* as surely as it can pin
            // one, and a reply Chat Completions could not have sent must not be
            // rendered just because a script asked for one.
            if scripted.is_empty() {
                return mismatch(
                    "a `tools` reply with no calls is not an answer Chat Completions can send: \
                     `finish_reason: \"tool_calls\"` names the calls that ended the turn, and \
                     there would be none. Script `text` for prose, or `raw` for a response \
                     generated code must reject",
                );
            }
            if let Some(forbidden) = forbidden_tool_call(request) {
                return mismatch(&format!(
                    "a `tools` reply cannot answer a request carrying {forbidden}: Chat \
                     Completions never calls a function the request forbade. Script `text` for \
                     prose, or `raw` for a response generated code must reject"
                ));
            }
            let mut calls = Vec::new();
            for (index, call) in scripted.iter().enumerate() {
                if !offered.contains(&call.name) {
                    return mismatch(&format!(
                        "the script calls the function `{}`, which this request does not offer",
                        call.name
                    ));
                }
                let mut block = call_block(sequence, index, &call.name, &call.input);
                if let Some(id) = &call.id {
                    block["id"] = json!(id);
                }
                calls.push(block);
            }
            (
                json!({
                    "role": "assistant",
                    "content": text.clone().map_or(Value::Null, Value::String),
                    "tool_calls": calls,
                    "refusal": Value::Null,
                }),
                "tool_calls",
            )
        }
    };

    let finish_reason = reply
        .stop_reason
        .clone()
        .unwrap_or_else(|| finish.to_string());
    let usage = reply.usage.map_or_else(
        || {
            let input = estimate(&canonical(request));
            let output = estimate(&canonical(&message));
            (input, output)
        },
        |usage| (usage.input_tokens, usage.output_tokens),
    );

    Response::new(
        200,
        json!({
            "id": format!("chatcmpl-mock-{sequence:08}"),
            "object": "chat.completion",
            "created": crate::control::CREATED,
            "model": model,
            "system_fingerprint": "fp_mock",
            "choices": [{
                "index": 0,
                "message": message,
                "logprobs": Value::Null,
                "finish_reason": finish_reason,
            }],
            "usage": {
                "prompt_tokens": usage.0,
                "completion_tokens": usage.1,
                "total_tokens": usage.0 + usage.1,
            },
        }),
    )
    .header("x-request-id", format!("req_mock_{sequence:08}"))
    .after(reply.delay)
    .answer()
}

/// How a request pinned the shape of its answer, if it pinned one.
///
/// `json_schema` guarantees parseable JSON in the content; `tool_choice:
/// "required"` and a forced function guarantee a tool call. `auto`, `none` and
/// `response_format: {type: "text"}` guarantee nothing, and leave prose legal.
/// Read off the request rather than off the parsed [`StructuredOutput`], which
/// is `None` for a forced function that declares no `parameters` — still a
/// pinned answer, just not a pinned schema.
fn pinned_answer_shape(request: &Value) -> Option<String> {
    if request
        .pointer("/response_format/type")
        .and_then(Value::as_str)
        == Some("json_schema")
    {
        return Some("`response_format: {type: \"json_schema\"}`".to_string());
    }
    match request.get("tool_choice")? {
        Value::String(pinned) if pinned == "required" => {
            Some("`tool_choice: \"required\"`".to_string())
        }
        Value::Object(choice) => {
            let name = choice
                .get("function")
                .and_then(|function| function.get("name"))
                .and_then(Value::as_str)
                .unwrap_or("…");
            Some(format!(
                "`tool_choice: {{type: \"function\", function: {{name: \"{name}\"}}}}`"
            ))
        }
        _ => None,
    }
}

/// How a request forbade a tool call, if it forbade one.
///
/// The exact inverse of [`pinned_answer_shape`]'s tool half, and its own
/// function for the same reason the Messages surface's is: `auto` leaves a call
/// *legal*, so only `none` belongs here. A request that offers no functions at
/// all is caught by the `offered.contains` check, which names the function the
/// script invented.
fn forbidden_tool_call(request: &Value) -> Option<String> {
    match request.get("tool_choice")? {
        Value::String(choice) if choice == "none" => Some("`tool_choice: \"none\"`".to_string()),
        _ => None,
    }
}

fn call_block(sequence: u64, index: usize, name: &str, input: &Value) -> Value {
    json!({
        "id": format!("call_mock_{sequence:08}_{index}"),
        "type": "function",
        "function": { "name": name, "arguments": canonical(input) },
    })
}

fn failure_answer(failure: &Failure) -> Answer {
    match failure {
        Failure::RateLimit {
            retry_after_seconds,
        } => {
            let response = error(
                429,
                "requests",
                Some("rate_limit_exceeded"),
                "Rate limit reached for requests. Please try again later.",
            );
            match retry_after_seconds {
                Some(seconds) => response.header("retry-after", seconds.to_string()),
                None => response,
            }
            .answer()
        }
        Failure::Overloaded => error(
            503,
            "server_error",
            None,
            "The server is overloaded or not ready yet.",
        )
        .answer(),
        Failure::ServerError => error(
            500,
            "server_error",
            None,
            "The server had an error while processing your request. Sorry about that!",
        )
        .answer(),
        Failure::Timeout { delay } => Answer::Close(*delay),
    }
}

/// The Chat Completions error envelope.
fn error(status: u16, kind: &str, code: Option<&str>, message: &str) -> Response {
    Response::new(
        status,
        json!({
            "error": {
                "message": message,
                "type": kind,
                "param": Value::Null,
                "code": code.map_or(Value::Null, |code| json!(code)),
            }
        }),
    )
}

/// The answer to a request that failed validation.
///
/// A missing credential outranks everything else in the list and is answered
/// **401** with `code: "invalid_api_key"`, because that is what the service
/// does: authentication is settled before the body is looked at, so the answer
/// names the credential and nothing else, and it carries no `param` — a header
/// is not a request parameter. Every other failure is the 400 the service sends
/// for a body it could not accept.
pub(crate) fn rejected(failures: &[ValidationFailure]) -> Answer {
    let credentials: Vec<&ValidationFailure> = failures
        .iter()
        .filter(|failure| failure.authentication)
        .collect();
    if !credentials.is_empty() {
        let message = credentials
            .iter()
            .map(|failure| failure.message.as_str())
            .collect::<Vec<_>>()
            .join(" ");
        return error(
            401,
            "invalid_request_error",
            Some("invalid_api_key"),
            &message,
        )
        .harness(INVALID)
        .answer();
    }
    let message = failures
        .iter()
        .map(|failure| failure.message.as_str())
        .collect::<Vec<_>>()
        .join(" ");
    let param = failures.first().map(|failure| failure.pointer.clone());
    let mut response = error(400, "invalid_request_error", None, &message).harness(INVALID);
    if let Some(param) = param {
        response.body["error"]["param"] = json!(param);
    }
    response.answer()
}

/// The answer to a request no scripted outcome answered.
pub(crate) fn unscripted(model: &str, reason: &str) -> Answer {
    error(
        HARNESS_STATUS,
        "invalid_request_error",
        Some("mock_provider"),
        &format!(
            "mock provider: no scripted outcome for model `{model}` ({reason}). \
             Enqueue one through POST /_mock/enqueue before the graph runs."
        ),
    )
    .harness(UNSCRIPTED)
    .answer()
}

/// The answer to a script that cannot be rendered into what the request asked
/// for.
fn mismatch(reason: &str) -> Answer {
    error(
        HARNESS_STATUS,
        "invalid_request_error",
        Some("mock_provider"),
        &format!("mock provider: {reason}"),
    )
    .harness(MISMATCH)
    .answer()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::control::{HARNESS_HEADER, ToolCall};

    fn headers() -> BTreeMap<String, String> {
        [
            ("authorization", "Bearer test-key"),
            ("content-type", "application/json"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
    }

    fn parse_direct(body: &Value) -> Parsed {
        parse(Route::Direct, &headers(), "", None, Some(body))
    }

    fn check(body: &Value) -> Vec<String> {
        parse_direct(body)
            .failures
            .into_iter()
            .map(|failure| failure.pointer)
            .collect()
    }

    fn request(extra: Value) -> Value {
        let mut request = json!({
            "model": "gpt-4o-mini",
            "messages": [{ "role": "user", "content": "hi" }],
        });
        for (key, value) in extra.as_object().expect("an object") {
            request[key] = value.clone();
        }
        request
    }

    /// The `response_format` shape of structured output, understood.
    #[test]
    fn a_json_schema_request_is_accepted_and_understood() {
        let body = request(json!({
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "review",
                    "strict": true,
                    "schema": {
                        "type": "object",
                        "properties": { "verdict": { "type": "string" } },
                        "required": ["verdict"],
                        "additionalProperties": false,
                    },
                },
            },
        }));
        let parsed = parse_direct(&body);
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        let Some(StructuredOutput::JsonSchema { name, strict, .. }) = parsed.structured_output
        else {
            panic!("`response_format` is one of the two structured-output surfaces");
        };
        assert_eq!(name, "review");
        assert!(strict);
    }

    /// `strict: true` closes the schema, and a schema that is not closed is the
    /// most common 400 on this surface. A mock that accepted it would let a
    /// Zod-to-JSON-Schema path pass every CI run and fail on the first live
    /// call, which is precisely what this server exists to prevent.
    #[test]
    fn a_strict_schema_must_close_every_object_and_require_every_property() {
        let strict = |schema: Value| {
            request(json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": { "name": "review", "strict": true, "schema": schema },
                },
            }))
        };

        let open = parse_direct(&strict(json!({
            "type": "object",
            "properties": { "verdict": { "type": "string" } },
        })))
        .failures;
        assert_eq!(
            open.iter()
                .map(|failure| failure.message.as_str())
                .collect::<Vec<_>>(),
            [
                "Invalid schema for response_format 'review': In context=(), 'additionalProperties' is required to be supplied and to be false.",
                "Invalid schema for response_format 'review': In context=(), 'required' is required to be supplied and to be an array including every key in properties. Missing 'verdict'.",
            ]
        );
        assert_eq!(open[0].pointer, "response_format.json_schema.schema");

        // The rules go all the way down, and the complaint addresses the object
        // inside the schema rather than the schema.
        let nested = parse_direct(&strict(json!({
            "type": "object",
            "properties": {
                "author": {
                    "type": "object",
                    "properties": { "name": { "type": "string" } },
                    "required": ["name"],
                },
            },
            "required": ["author"],
            "additionalProperties": false,
        })))
        .failures;
        assert_eq!(nested.len(), 1, "{nested:?}");
        assert!(
            nested[0]
                .message
                .contains("In context=('properties', 'author'), 'additionalProperties'"),
            "{}",
            nested[0].message
        );

        // `strict: false` — and an absent `strict` — ask for none of this.
        let lenient = request(json!({
            "response_format": {
                "type": "json_schema",
                "json_schema": {
                    "name": "review",
                    "schema": { "type": "object", "properties": { "verdict": { "type": "string" } } },
                },
            },
        }));
        assert!(
            parse_direct(&lenient).failures.is_empty(),
            "{:?}",
            parse_direct(&lenient).failures
        );
    }

    /// The same rules on a `strict: true` function tool, which is the other
    /// place this surface accepts the flag.
    #[test]
    fn a_strict_function_tool_is_held_to_the_same_schema_rules() {
        let tool = |strict: bool| {
            request(json!({
                "tools": [{
                    "type": "function",
                    "function": {
                        "name": "extract",
                        "strict": strict,
                        "parameters": {
                            "type": "object",
                            "properties": { "verdict": { "type": "string" } },
                        },
                    },
                }],
            }))
        };

        let failures = parse_direct(&tool(true)).failures;
        assert_eq!(failures.len(), 2, "{failures:?}");
        assert_eq!(failures[0].pointer, "tools.0.function.parameters");
        assert!(
            failures[0]
                .message
                .starts_with("Invalid schema for function 'extract': In context=(),"),
            "{}",
            failures[0].message
        );

        assert!(
            parse_direct(&tool(false)).failures.is_empty(),
            "an unstrict function tool takes the schema it is given"
        );
    }

    /// `tool_choice` without a tool surface is refused here too — the same
    /// mirror rule the Messages surface keeps, in this dialect.
    #[test]
    fn a_tool_choice_without_tools_is_refused() {
        for choice in [
            json!("auto"),
            json!("required"),
            json!("none"),
            json!({ "type": "function", "function": { "name": "extract" } }),
        ] {
            let body = request(json!({ "tool_choice": choice.clone() }));
            let failures = parse_direct(&body).failures;
            assert_eq!(failures.len(), 1, "{choice}: {failures:?}");
            assert_eq!(failures[0].pointer, "tool_choice");
            assert!(
                failures[0].message.contains("when 'tools' are provided"),
                "{}",
                failures[0].message
            );
        }

        // The positive half: over a tool surface, every one of those is legal.
        let body = request(json!({
            "tools": [{
                "type": "function",
                "function": { "name": "extract", "parameters": { "type": "object" } },
            }],
            "tool_choice": "auto",
        }));
        assert!(parse_direct(&body).failures.is_empty());
    }

    /// An empty tool list is refused, not ignored: it is what codegen emits for
    /// a tool-less agent if it always writes the key, and the service answers a
    /// 400.
    #[test]
    fn an_empty_tools_array_is_refused() {
        let failures = parse_direct(&request(json!({ "tools": [] }))).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "tools");
        assert_eq!(
            failures[0].message,
            "Invalid 'tools': empty array. Expected an array with minimum length 1."
        );

        // Absent is the correct spelling for "no tools", and it is accepted.
        assert!(parse_direct(&request(json!({}))).failures.is_empty());
    }

    /// `top_k` is accepted here because `openai_compatible` backends take it and
    /// grammar 12.2 lists it in the `settings:` vocabulary — one route serves
    /// three provider kinds and cannot tell them apart.
    #[test]
    fn top_k_is_accepted_for_the_openai_compatible_backends() {
        assert!(
            parse_direct(&request(json!({ "top_k": 40 })))
                .failures
                .is_empty()
        );
    }

    /// The forced-function shape of structured output, understood.
    #[test]
    fn a_forced_function_request_is_accepted_and_understood() {
        let body = request(json!({
            "tools": [{
                "type": "function",
                "function": {
                    "name": "extract",
                    "parameters": { "type": "object", "properties": {} },
                },
            }],
            "tool_choice": { "type": "function", "function": { "name": "extract" } },
        }));
        let parsed = parse_direct(&body);
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        assert_eq!(parsed.tools, ["extract"]);
        assert!(matches!(
            parsed.structured_output,
            Some(StructuredOutput::ForcedFunction { .. })
        ));
    }

    /// The envelope and the auth header.
    #[test]
    fn the_required_envelope_is_required() {
        assert_eq!(check(&json!({ "messages": [] })), ["model", "messages"]);

        let mut anonymous = headers();
        anonymous.remove("authorization");
        let parsed = parse(
            Route::Direct,
            &anonymous,
            "",
            None,
            Some(&request(json!({}))),
        );
        let failures: Vec<String> = parsed
            .failures
            .iter()
            .map(|failure| failure.pointer.clone())
            .collect();
        assert_eq!(failures, ["headers.authorization"]);
        // A credential, so the refusal is a 401 rather than the 400 a bad body
        // draws — the distinction generated code classifies on.
        assert!(parsed.failures[0].authentication);
        let Answer::Respond(response) = rejected(&parsed.failures) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 401);
        assert_eq!(response.body["error"]["code"], "invalid_api_key");

        // …while a content-type complaint is about the request, not the caller.
        let mut plain = headers();
        plain.insert("content-type".to_string(), "text/plain".to_string());
        let parsed = parse(Route::Direct, &plain, "", None, Some(&request(json!({}))));
        assert_eq!(parsed.failures[0].pointer, "headers.content-type");
        assert!(!parsed.failures[0].authentication);
        let Answer::Respond(response) = rejected(&parsed.failures) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);
    }

    /// A `model` that is present and names nothing is its own mistake here too.
    #[test]
    fn an_empty_model_is_its_own_mistake() {
        let mut body = request(json!({}));
        body["model"] = json!("");
        let failures = parse_direct(&body).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "model");

        body["model"] = json!(7);
        let failures = parse_direct(&body).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(
            failures[0].message.contains("expected string"),
            "{}",
            failures[0].message
        );
    }

    /// An unknown argument is the API's own sentence, not a shrug.
    #[test]
    fn an_unknown_request_argument_is_refused() {
        let parsed = parse_direct(&request(json!({ "temperatur": 0.5 })));
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(
            parsed.failures[0].message,
            "Unrecognized request argument supplied: temperatur"
        );
    }

    /// A `tool` message answers a pending call, or it is not a valid message.
    #[test]
    fn a_tool_message_must_answer_a_pending_call() {
        let stray = request(json!({
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "tool", "content": "result", "tool_call_id": "call_1" },
            ],
        }));
        let failures = parse_direct(&stray).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.1.tool_call_id");
        assert!(
            failures[0]
                .message
                .contains("must be a response to a preceding message"),
            "{}",
            failures[0].message
        );
    }

    /// Every pending call must be answered before the conversation moves on.
    #[test]
    fn a_dropped_tool_result_is_refused() {
        let dropped = request(json!({
            "tools": [{ "type": "function", "function": { "name": "web_search" } }],
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "web_search", "arguments": "{}" } },
                    { "id": "call_2", "type": "function", "function": { "name": "web_search", "arguments": "{}" } },
                ]},
                { "role": "tool", "content": "one", "tool_call_id": "call_1" },
                { "role": "user", "content": "carry on" },
            ],
        }));
        let failures = parse_direct(&dropped).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].message.contains("'call_2'"), "{failures:?}");
        assert_eq!(
            failures[0].pointer, "messages.1",
            "the complaint is at the assistant turn that asked, not at the turn that \
             moved on without answering"
        );
    }

    /// The whole loop, correct.
    #[test]
    fn a_well_formed_tool_loop_is_accepted() {
        let body = request(json!({
            "tools": [{ "type": "function", "function": { "name": "web_search", "parameters": { "type": "object" } } }],
            "messages": [
                { "role": "system", "content": "You search." },
                { "role": "user", "content": "go" },
                { "role": "assistant", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "web_search", "arguments": "{\"query\":\"it\"}" } },
                ]},
                { "role": "tool", "content": "found", "tool_call_id": "call_1" },
            ],
        }));
        let parsed = parse_direct(&body);
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    }

    /// `arguments` is a JSON string. An object there is the shape mistake a
    /// generated tool loop makes, and a real request would be refused for it.
    #[test]
    fn tool_call_arguments_travel_as_a_json_string() {
        let body = request(json!({
            "tools": [{ "type": "function", "function": { "name": "web_search" } }],
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "web_search", "arguments": { "query": "it" } } },
                ]},
                { "role": "tool", "content": "found", "tool_call_id": "call_1" },
            ],
        }));
        let failures = parse_direct(&body).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(
            failures[0].pointer,
            "messages.1.tool_calls.0.function.arguments"
        );

        let malformed = request(json!({
            "tools": [{ "type": "function", "function": { "name": "web_search" } }],
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "tool_calls": [
                    { "id": "call_1", "type": "function", "function": { "name": "web_search", "arguments": "{not json" } },
                ]},
                { "role": "tool", "content": "found", "tool_call_id": "call_1" },
            ],
        }));
        let failures = parse_direct(&malformed).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].message.contains("JSON-encoded string"));
    }

    /// An assistant turn must say something: content, or calls.
    #[test]
    fn an_empty_assistant_turn_is_refused() {
        let body = request(json!({
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant" },
            ],
        }));
        let failures = parse_direct(&body).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.1");
    }

    /// The Azure route: the deployment names the model when the body does not,
    /// `api-key` authenticates, and `api-version` is required.
    #[test]
    fn the_azure_route_keys_on_its_deployment() {
        let azure: BTreeMap<String, String> = [
            ("api-key", "test-key"),
            ("content-type", "application/json"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect();
        let body = json!({ "messages": [{ "role": "user", "content": "hi" }] });

        let parsed = parse(
            Route::Azure,
            &azure,
            "api-version=2024-10-21",
            Some("smart-deployment"),
            Some(&body),
        );
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        assert_eq!(parsed.model, "smart-deployment");

        let parsed = parse(
            Route::Azure,
            &azure,
            "",
            Some("smart-deployment"),
            Some(&body),
        );
        assert_eq!(
            parsed
                .failures
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["query.api-version"]
        );

        // A body that names a model outranks the path, so one deployment can
        // serve several scripted model ids.
        let named =
            json!({ "model": "gpt-4o-mini", "messages": [{ "role": "user", "content": "hi" }] });
        let parsed = parse(
            Route::Azure,
            &azure,
            "api-version=2024-10-21",
            Some("smart-deployment"),
            Some(&named),
        );
        assert_eq!(parsed.model, "gpt-4o-mini");

        // The newer `/openai/v1/chat/completions` carries no deployment, so
        // nothing stands in for a body that names no model: it is required
        // there exactly as it is on the direct route (WIRE-NOTES §7). Keyed
        // under the empty string is how a whole run would collapse into one
        // queue and a codegen bug would go unreported.
        let parsed = parse(
            Route::Azure,
            &azure,
            "api-version=preview",
            None,
            Some(&body),
        );
        assert_eq!(
            parsed
                .failures
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["model"]
        );
        let parsed = parse(
            Route::Azure,
            &azure,
            "api-version=preview",
            None,
            Some(&named),
        );
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        assert_eq!(parsed.model, "gpt-4o-mini");
    }

    /// A `json_schema` structured reply is the object, serialized into content.
    #[test]
    fn a_structured_reply_renders_into_the_message_content() {
        let body = request(json!({
            "response_format": {
                "type": "json_schema",
                "json_schema": { "name": "review", "schema": { "type": "object" } },
            },
        }));
        let parsed = parse_direct(&body);
        let outcome = Outcome::structured(json!({ "verdict": "approve" }));
        let Answer::Respond(response) = render(
            7,
            &body,
            &parsed.model,
            parsed.structured_output.as_ref(),
            &outcome,
        ) else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["id"], "chatcmpl-mock-00000007");
        assert_eq!(response.body["created"], crate::control::CREATED);
        assert_eq!(response.body["choices"][0]["finish_reason"], "stop");
        assert_eq!(
            response.body["choices"][0]["message"]["content"],
            "{\"verdict\":\"approve\"}"
        );
        let usage = &response.body["usage"];
        assert_eq!(
            usage["total_tokens"].as_u64(),
            Some(
                usage["prompt_tokens"].as_u64().unwrap()
                    + usage["completion_tokens"].as_u64().unwrap()
            )
        );
    }

    /// A forced-function structured reply is a tool call whose arguments are a
    /// string — the same rule the request side enforces.
    #[test]
    fn a_forced_function_structured_reply_renders_as_a_tool_call() {
        let body = request(json!({
            "tools": [{ "type": "function", "function": { "name": "extract", "parameters": { "type": "object" } } }],
            "tool_choice": { "type": "function", "function": { "name": "extract" } },
        }));
        let parsed = parse_direct(&body);
        let Answer::Respond(response) = render(
            1,
            &body,
            &parsed.model,
            parsed.structured_output.as_ref(),
            &Outcome::structured(json!({ "verdict": "revise" })),
        ) else {
            panic!("a reply is a response");
        };
        let call = &response.body["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call_mock_00000001_0");
        assert_eq!(call["function"]["name"], "extract");
        assert_eq!(call["function"]["arguments"], "{\"verdict\":\"revise\"}");
        assert_eq!(response.body["choices"][0]["finish_reason"], "tool_calls");
    }

    /// A structured reply with nothing asking for structure is a script bug.
    #[test]
    fn a_structured_reply_without_a_request_for_one_is_a_mismatch() {
        let body = request(json!({}));
        let Answer::Respond(response) = render(
            1,
            &body,
            "gpt-4o-mini",
            None,
            &Outcome::structured(json!({ "a": 1 })),
        ) else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[HARNESS_HEADER], MISMATCH);
    }

    /// And the inverse: prose cannot answer a request that pinned the shape of
    /// its answer, on either of this surface's two mechanisms.
    #[test]
    fn a_text_reply_to_a_pinned_answer_shape_is_a_mismatch() {
        let pinned = [
            request(json!({
                "response_format": {
                    "type": "json_schema",
                    "json_schema": {
                        "name": "review",
                        "schema": { "type": "object" },
                    },
                },
            })),
            request(json!({
                "tools": [{
                    "type": "function",
                    "function": { "name": "extract", "parameters": { "type": "object" } },
                }],
                "tool_choice": { "type": "function", "function": { "name": "extract" } },
            })),
            request(json!({
                "tools": [{
                    "type": "function",
                    "function": { "name": "extract", "parameters": { "type": "object" } },
                }],
                "tool_choice": "required",
            })),
        ];
        for body in &pinned {
            let parsed = parse_direct(body);
            assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
            let Answer::Respond(response) = render(
                1,
                body,
                &parsed.model,
                parsed.structured_output.as_ref(),
                &Outcome::text("not json at all"),
            ) else {
                panic!("a mismatch is a response");
            };
            assert_eq!(response.status, HARNESS_STATUS, "{body}");
            assert_eq!(response.headers[HARNESS_HEADER], MISMATCH);
        }

        // The positive half: `auto`, `text`, and a request that pins nothing all
        // leave prose legal.
        let free = [
            request(json!({})),
            request(json!({ "response_format": { "type": "text" } })),
            request(json!({
                "tools": [{
                    "type": "function",
                    "function": { "name": "extract", "parameters": { "type": "object" } },
                }],
                "tool_choice": "auto",
            })),
        ];
        for body in &free {
            let parsed = parse_direct(body);
            assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
            let Answer::Respond(response) = render(
                1,
                body,
                &parsed.model,
                parsed.structured_output.as_ref(),
                &Outcome::text("just talking"),
            ) else {
                panic!("a reply is a response");
            };
            assert_eq!(response.status, 200, "{body}");
            assert_eq!(
                response.body["choices"][0]["message"]["content"],
                "just talking"
            );
            assert_eq!(response.body["choices"][0]["finish_reason"], "stop");
        }
    }

    /// Tool calls the request never offered are refused for the same reason.
    #[test]
    fn a_call_to_an_unoffered_function_is_a_mismatch() {
        let body = request(json!({
            "tools": [{ "type": "function", "function": { "name": "web_search" } }],
        }));
        let Answer::Respond(response) = render(
            1,
            &body,
            "gpt-4o-mini",
            None,
            &Outcome::tool_calls(vec![ToolCall::new("web_serch", json!({}))]),
        ) else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.headers[HARNESS_HEADER], MISMATCH);
    }

    /// And so are a call the request *forbade* and a tool reply with no call in
    /// it — the mirror of the prose refusals, in the other direction.
    #[test]
    fn a_tool_reply_the_request_forbids_or_empties_is_a_mismatch() {
        let with = |choice: Value| {
            request(json!({
                "tools": [{
                    "type": "function",
                    "function": { "name": "lookup", "parameters": { "type": "object" } },
                }],
                "tool_choice": choice,
            }))
        };
        let call = || Outcome::tool_calls(vec![ToolCall::new("lookup", json!({}))]);

        let body = with(json!("none"));
        let parsed = parse_direct(&body);
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        let Answer::Respond(response) = render(1, &body, &parsed.model, None, &call()) else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[HARNESS_HEADER], MISMATCH);

        for body in [with(json!("auto")), request(json!({}))] {
            let Answer::Respond(response) = render(
                1,
                &body,
                "gpt-4o-mini",
                None,
                &Outcome::tool_calls(Vec::new()),
            ) else {
                panic!("a mismatch is a response");
            };
            assert_eq!(response.status, HARNESS_STATUS, "{body}");
            assert_eq!(response.headers[HARNESS_HEADER], MISMATCH);
        }

        // The positive half: `auto` leaves a call legal, and it is rendered.
        let body = with(json!("auto"));
        let Answer::Respond(response) = render(1, &body, "gpt-4o-mini", None, &call()) else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(
            response.body["choices"][0]["message"]["tool_calls"][0]["function"]["name"],
            "lookup"
        );
        assert_eq!(response.body["choices"][0]["finish_reason"], "tool_calls");
    }

    /// The failure shapes generated code classifies on.
    #[test]
    fn the_failure_shapes_are_the_providers_own() {
        let body = request(json!({}));
        let served = |outcome: Outcome| match render(1, &body, "gpt-4o-mini", None, &outcome) {
            Answer::Respond(response) => (response.status, response.body),
            Answer::Close(_) => panic!("this outcome answers"),
        };

        let (status, error) = served(Outcome::rate_limit());
        assert_eq!(status, 429);
        assert_eq!(error["error"]["code"], "rate_limit_exceeded");

        let (status, error) = served(Outcome::overloaded());
        assert_eq!(status, 503, "OpenAI's overload status is 503, not 529");
        assert_eq!(error["error"]["type"], "server_error");

        assert_eq!(served(Outcome::server_error()).0, 500);

        assert!(matches!(
            render(
                1,
                &body,
                "gpt-4o-mini",
                None,
                &Outcome::timeout(std::time::Duration::from_millis(1))
            ),
            Answer::Close(_)
        ));
    }

    /// A rejection carries the first bad field in `param`, where a client looks.
    #[test]
    fn a_rejection_points_at_a_parameter() {
        let Answer::Respond(response) = rejected(&[ValidationFailure::new(
            "messages.0.role",
            "Invalid value: 'wizard'.",
        )]) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);
        assert_eq!(response.body["error"]["param"], "messages.0.role");
        assert_eq!(response.headers[HARNESS_HEADER], INVALID);
    }
}
