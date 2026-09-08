//! OpenAI's Responses API: `POST /v1/responses`.
//!
//! The wire an `openai` provider moves to when it declares `server_tools:`
//! (grammar 12.1, Decision D122), and the only OpenAI surface that carries the
//! built-in tool suite — web search, file search, code interpreter, image
//! generation — at all. A compiled graph reaches it for **every** call that
//! provider serves, structured-output calls included: one provider, one wire.
//!
//! # What it is not
//!
//! Not a route of [`crate::openai`]. Four things differ, and each is a shape a
//! codegen bug takes:
//!
//! * the conversation is a list of **items**, not messages: a user turn is a
//!   `message` item, a function call is a `function_call` item beside it rather
//!   than a field on it, and a result is a `function_call_output` keyed by
//!   `call_id`;
//! * the system prompt is `instructions`, a top-level string, so it is not the
//!   first item;
//! * a tool is **flat** — `{type: "function", name, parameters}` — where Chat
//!   Completions nests it under `function`;
//! * structured output is `text.format`, carrying the same `json_schema` shape
//!   Chat Completions carries under `response_format`.
//!
//! What it **shares** is the error envelope and the credential rules, which are
//! the connection's rather than the surface's: [`crate::openai::rejected`] and
//! [`crate::openai::unscripted`] answer this route too.
//!
//! # Structured output
//!
//! Two mechanisms, like every other wire since PRD §9 resolved q53:
//! `text.format.type: "json_schema"`, which shapes the turn's final message and
//! is the rung a compiled graph prefers, and a **flat** forced function —
//! `tool_choice: {type: "function", name}` over a function carrying the same
//! schema — which is where it falls when an endpoint will not take the first.
//! Either way q16's posture holds on this wire: what is constrained is exactly
//! what is parsed.
//!
//! `WIRE-NOTES.md` records the concessions.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::control::{
    Outcome, Reply, ReplyBody, ServerToolUse, StructuredOutput, ValidationFailure, canonical,
    estimate, request_id,
};
use crate::openai::{Tools, check_root_schema, check_strict_schema, error, well_formed};
use crate::strict::{Checker, Dialect, Kind, at, listed};
use crate::wire::{Answer, HARNESS_STATUS, MISMATCH, Response};

/// The top-level keys this surface accepts.
///
/// The union of what the service takes and what grammar 12.2's `settings:`
/// vocabulary can put on the wire, minus the two knobs Responses does not have
/// (`stop`, `seed`) — those are refused, which is the point: an author who wrote
/// one on a provider that speaks this wire has declared a knob the service will
/// not read, and the run says so rather than quietly doing something else.
const REQUEST_KEYS: &[&str] = &[
    "model",
    "input",
    "instructions",
    "tools",
    "tool_choice",
    "text",
    "temperature",
    "top_p",
    "top_k",
    "max_output_tokens",
    "parallel_tool_calls",
    "reasoning",
    "metadata",
    "store",
    "previous_response_id",
    "stream",
    "truncation",
    "include",
];

/// The item types this surface accepts in `input`.
///
/// Deliberately a **closed** list of the ones a compiled graph composes, plus
/// the ones the service itself produced and a replay sends back. An item type
/// outside it is a codegen bug, and `reasoning` is on the list because a model
/// with `reasoning:` set answers with one and requires it back.
const INPUT_ITEMS: &[&str] = &[
    "message",
    "function_call",
    "function_call_output",
    "reasoning",
];

/// What the surface understood about a request.
pub(crate) struct Parsed {
    pub(crate) model: String,
    pub(crate) failures: Vec<ValidationFailure>,
    pub(crate) tools: Vec<String>,
    pub(crate) server_tools: Vec<String>,
    pub(crate) structured_output: Option<StructuredOutput>,
}

/// Read and check one request.
pub(crate) fn parse(headers: &BTreeMap<String, String>, body: Option<&Value>) -> Parsed {
    let mut checker = Checker::new(Dialect::OpenAi);
    crate::openai::check_direct_headers(&mut checker, headers);

    let Some(body) = body.and_then(Value::as_object) else {
        checker.fail(
            "",
            "We could not parse the JSON body of your request. (HINT: This likely means you aren't using your HTTP library correctly.)",
        );
        return Parsed {
            model: String::new(),
            failures: checker.into_failures(),
            tools: Vec::new(),
            server_tools: Vec::new(),
            structured_output: None,
        };
    };

    checker.closed("", body, REQUEST_KEYS);
    let model = checker
        .required_string("", body, "model")
        .unwrap_or_default()
        .to_string();
    if body.get("model").and_then(Value::as_str) == Some("") {
        checker.fail("model", "Invalid value: ''. 'model' must name a model.");
    }
    checker.optional("", body, "instructions", Kind::String);
    check_settings(&mut checker, body);
    let tools = check_tools(&mut checker, body);
    check_input(&mut checker, body, &tools.names);
    let forced = check_tool_choice(&mut checker, body, &tools.names);
    let format = check_text_format(&mut checker, body);
    check_streaming(&mut checker, body);

    // This wire's two structured-output mechanisms, and the pin wins where a
    // body somehow carries both — the same rule [`crate::openai`]'s
    // `structured_destination` keeps, for the same reason: `text.format` shapes
    // the turn's text and a forced function decides whether the turn has any
    // text at all (PRD §9 resolved q53). A compiled graph sends one.
    let structured_output = forced
        .and_then(|name| {
            let schema = function_schema(body, &name)?;
            Some(StructuredOutput::ForcedFunction { name, schema })
        })
        .or(format);

    Parsed {
        model,
        failures: checker.into_failures(),
        tools: tools.names,
        server_tools: tools.server,
        structured_output,
    }
}

/// `tool_choice`, and the function it forces if it forces one.
///
/// This wire's **other** structured-output mechanism (PRD §9 resolved q53), and
/// the rung a compiled graph falls to when an endpoint refuses `text.format`.
/// Flat, like every other tool reference here: `{type: "function", name}`, where
/// Chat Completions nests the name under `function`.
///
/// The same rule the other two surfaces keep about the key itself: `tool_choice`
/// says how the model may use `tools`, so a request that offers none has nothing
/// for it to say.
fn check_tool_choice(
    checker: &mut Checker,
    body: &Map<String, Value>,
    offered: &[String],
) -> Option<String> {
    let choice = body.get("tool_choice")?;
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
        Value::Object(chosen) => {
            checker.closed("tool_choice", chosen, &["type", "name"]);
            if let Some(kind) = checker.required("tool_choice", chosen, "type") {
                checker.one_of("tool_choice.type", kind, &["function"]);
            }
            let name = checker.required_string("tool_choice", chosen, "name")?;
            let name = name.to_string();
            if !offered.contains(&name) {
                checker.fail(
                    "tool_choice.name",
                    format!("Invalid value: 'tool_choice' names the function '{name}', which this request does not offer."),
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

/// The `parameters` a named function declares in this request's `tools`.
fn function_schema(body: &Map<String, Value>, name: &str) -> Option<Value> {
    body.get("tools")?
        .as_array()?
        .iter()
        .find(|tool| {
            tool.get("type").and_then(Value::as_str) == Some("function")
                && tool.get("name").and_then(Value::as_str) == Some(name)
        })?
        .get("parameters")
        .cloned()
}

/// The sampling knobs, by type and by range.
///
/// The same argument the other two surfaces make: a key list that only asks
/// whether the *name* is known accepts `temperature: "hot"`, which the service
/// answers 400. `max_output_tokens` is the one knob spelled differently here,
/// and it is checked under that spelling precisely so a runtime that forgot to
/// translate `max_tokens` is caught by the closed key list above.
fn check_settings(checker: &mut Checker, body: &Map<String, Value>) {
    checker.bounded_number("", body, "temperature", 0.0..=2.0);
    checker.bounded_number("", body, "top_p", 0.0..=1.0);
    checker.bounded_integer("", body, "top_k", 1..=i64::MAX);
    checker.bounded_integer("", body, "max_output_tokens", 1..=i64::MAX);
    checker.optional("", body, "parallel_tool_calls", Kind::Boolean);
    checker.optional("", body, "store", Kind::Boolean);
    if let Some(reasoning) = checker.optional("", body, "reasoning", Kind::Object)
        && let Some(reasoning) = reasoning.as_object()
    {
        checker.closed("reasoning", reasoning, &["effort", "summary"]);
        if let Some(effort) = checker.optional("reasoning", reasoning, "effort", Kind::String) {
            checker.one_of(
                "reasoning.effort",
                effort,
                &["minimal", "low", "medium", "high"],
            );
        }
    }
}

/// The tool surface, split by who runs each one.
fn check_tools(checker: &mut Checker, body: &Map<String, Value>) -> Tools {
    let mut names = Vec::new();
    let mut server = Vec::new();
    let Some(tools) = checker.optional("", body, "tools", Kind::Array) else {
        return Tools { names, server };
    };
    if tools.as_array().is_some_and(Vec::is_empty) {
        checker.fail(
            "tools",
            "Invalid 'tools': empty array. Expected an array with minimum length 1.",
        );
        return Tools { names, server };
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
        let Some(kind) = checker.required_string(&pointer, tool, "type") else {
            continue;
        };
        // A **server tool** (grammar 12.1, Decision D122): the service runs it,
        // its config keys are the vendor's, and this server cannot know which
        // are legal for a tool it may predate. Recorded and carried.
        // `WIRE-NOTES` (22).
        if kind != "function" {
            server.push(kind.to_string());
            continue;
        }
        checker.closed(
            &pointer,
            tool,
            &["type", "name", "description", "parameters", "strict"],
        );
        let Some(name) = checker.required_string(&pointer, tool, "name") else {
            continue;
        };
        let name = name.to_string();
        if !well_formed(&name) {
            checker.fail(
                &at(&pointer, "name"),
                format!(
                    "Invalid 'tools[{index}].name': string does not match pattern. Expected a string that matches the pattern '^[a-zA-Z0-9_-]+$'."
                ),
            );
        }
        if !seen.insert(name.clone()) {
            checker.fail(
                &at(&pointer, "name"),
                format!("Invalid 'tools': duplicate function name '{name}'."),
            );
        }
        let parameters = checker.optional(&pointer, tool, "parameters", Kind::Object);
        if let Some(parameters) = parameters {
            check_root_schema(
                checker,
                &at(&pointer, "parameters"),
                &format!("function '{name}'"),
                parameters,
            );
        }
        let strict = checker
            .optional(&pointer, tool, "strict", Kind::Boolean)
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
    Tools { names, server }
}

/// The conversation, as a list of items.
///
/// The correlation rule is the one that matters and it is this wire's own: every
/// `function_call` must be answered by a `function_call_output` carrying its
/// `call_id`, and every output must answer a call that is there. Chat
/// Completions enforces the same pairing between an assistant turn's
/// `tool_calls` and the `tool` messages after it; here the two are siblings in
/// one list, so the check is over the list rather than between messages.
fn check_input(checker: &mut Checker, body: &Map<String, Value>, tools: &[String]) {
    let Some(input) = checker.required_array("", body, "input") else {
        return;
    };
    if input.is_empty() {
        checker.fail(
            "input",
            "Invalid 'input': empty array. Expected at least one item.",
        );
        return;
    }
    let mut asked: Vec<String> = Vec::new();
    let mut answered: BTreeSet<String> = BTreeSet::new();
    for (index, item) in input.iter().enumerate() {
        let pointer = at("input", index);
        let Some(item) = checker
            .typed(&pointer, item, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(kind) = checker.required(&pointer, item, "type") else {
            continue;
        };
        // A server-tool item the service produced, replayed back: a
        // `web_search_call`, a `file_search_call`, an `image_generation_call`.
        // A graph must send these back unaltered and must not answer them
        // (Decision D122), so what is checked is that they arrived at all.
        // `WIRE-NOTES` (22).
        if kind.as_str().is_some_and(|name| name.ends_with("_call"))
            && kind.as_str() != Some("function_call")
        {
            continue;
        }
        let Some(kind) = checker
            .one_of(&at(&pointer, "type"), kind, INPUT_ITEMS)
            .map(str::to_string)
        else {
            continue;
        };
        match kind.as_str() {
            "message" => check_message_item(checker, &pointer, item),
            "function_call" => {
                let Some(call_id) = checker.required_string(&pointer, item, "call_id") else {
                    continue;
                };
                let call_id = call_id.to_string();
                if let Some(name) = checker.required_string(&pointer, item, "name")
                    && !tools.is_empty()
                    && !tools.contains(&name.to_string())
                {
                    // Unlike Chat Completions, this surface does **not** refuse a
                    // history that names an undeclared function — the items are
                    // an echo of what the service produced, not a re-declaration
                    // — so this is not a failure. It is left alone deliberately,
                    // and `WIRE-NOTES` (21) says so.
                }
                let _ = checker.required_string(&pointer, item, "arguments");
                asked.push(call_id);
            }
            "function_call_output" => {
                let Some(call_id) = checker.required_string(&pointer, item, "call_id") else {
                    continue;
                };
                let call_id = call_id.to_string();
                if !asked.contains(&call_id) {
                    checker.fail(
                        &at(&pointer, "call_id"),
                        format!(
                            "Invalid value: '{call_id}'. There is no function call with that id in this conversation."
                        ),
                    );
                }
                answered.insert(call_id);
                let _ = checker.required(&pointer, item, "output");
            }
            _ => {}
        }
    }
    for call_id in &asked {
        if !answered.contains(call_id) {
            checker.fail(
                "input",
                format!(
                    "No output found for function call '{call_id}'. Every function call must be answered."
                ),
            );
        }
    }
}

/// One `message` item: a role and content parts.
fn check_message_item(checker: &mut Checker, pointer: &str, item: &Map<String, Value>) {
    let Some(role) = checker.required(pointer, item, "role") else {
        return;
    };
    let Some(role) = checker
        .one_of(
            &at(pointer, "role"),
            role,
            &["system", "developer", "user", "assistant"],
        )
        .map(str::to_string)
    else {
        return;
    };
    let Some(content) = checker.required(pointer, item, "content") else {
        return;
    };
    // A bare string is legal on this surface too, and is what a graph sends for
    // the simplest turn.
    if content.is_string() {
        return;
    }
    let Some(parts) = checker.typed(&at(pointer, "content"), content, Kind::Array) else {
        return;
    };
    let Some(parts) = parts.as_array() else {
        return;
    };
    if parts.is_empty() {
        checker.fail(
            &at(pointer, "content"),
            format!("Invalid '{}': empty array.", at(pointer, "content")),
        );
        return;
    }
    // The part vocabulary is per direction: what a client writes is `input_*`,
    // what the service produced is `output_text` or a `refusal`, and an assistant
    // turn replayed back carries the latter.
    let allowed: &[&str] = if role == "assistant" {
        &["output_text", "refusal", "input_text"]
    } else {
        &["input_text", "input_image", "input_file"]
    };
    for (index, part) in parts.iter().enumerate() {
        let pointer = at(&at(pointer, "content"), index);
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
            .one_of(&at(&pointer, "type"), kind, allowed)
            .map(str::to_string)
        else {
            continue;
        };
        if kind == "input_text" || kind == "output_text" {
            let _ = checker.required_string(&pointer, part, "text");
        }
    }
}

/// `text.format`, and the structured output it asks for.
fn check_text_format(checker: &mut Checker, body: &Map<String, Value>) -> Option<StructuredOutput> {
    let text = checker.optional("", body, "text", Kind::Object)?;
    let text = text.as_object()?;
    checker.closed("text", text, &["format", "verbosity"]);
    let format = checker.optional("text", text, "format", Kind::Object)?;
    let format = format.as_object()?;
    let kind = checker.required("text.format", format, "type")?;
    let kind = checker
        .one_of(
            "text.format.type",
            kind,
            &["text", "json_object", "json_schema"],
        )?
        .to_string();
    if kind != "json_schema" {
        return None;
    }
    checker.closed(
        "text.format",
        format,
        &["type", "name", "schema", "strict", "description"],
    );
    let name = checker
        .required_string("text.format", format, "name")?
        .to_string();
    if !well_formed(&name) {
        checker.fail(
            "text.format.name",
            "Invalid 'text.format.name': string does not match pattern. Expected a string that matches the pattern '^[a-zA-Z0-9_-]+$'.",
        );
    }
    let declared = checker
        .required_object("text.format", format, "schema")?
        .clone();
    let declared = Value::Object(declared);
    check_root_schema(
        checker,
        "text.format.schema",
        &format!("text.format '{name}'"),
        &declared,
    );
    let strict = checker
        .optional("text.format", format, "strict", Kind::Boolean)
        .and_then(Value::as_bool)
        .unwrap_or(false);
    if strict {
        check_strict_schema(
            checker,
            "text.format.schema",
            &format!("text.format '{name}'"),
            &declared,
        );
    }
    Some(StructuredOutput::JsonSchema {
        name,
        schema: declared,
        strict,
    })
}

/// Streaming is out of scope, and a request that asks for it is told so by name.
fn check_streaming(checker: &mut Checker, body: &Map<String, Value>) {
    if body.get("stream").and_then(Value::as_bool) == Some(true) {
        checker.fail(
            "stream",
            "mock provider: streaming responses are not implemented. See WIRE-NOTES.md.",
        );
    }
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

/// Render one scripted outcome into a Responses answer.
pub(crate) fn render(
    sequence: u64,
    request: &Value,
    model: &str,
    structured: Option<&StructuredOutput>,
    outcome: &Outcome,
) -> Answer {
    match outcome {
        Outcome::Failure(failure) => crate::openai::failure_answer(sequence, failure),
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

/// The statuses a response ends with.
const STATUSES: &[&str] = &["completed", "incomplete", "failed"];

/// The reasons an incomplete response gives.
const INCOMPLETE_REASONS: &[&str] = &["max_output_tokens", "content_filter"];

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
                .filter(|tool| tool.get("type").and_then(Value::as_str) == Some("function"))
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut output = match server_tool_items(sequence, request, &reply.server_tools) {
        Ok(items) => items,
        Err(refusal) => return refusal,
    };

    match &reply.body {
        ReplyBody::Text(text) => {
            if let Some(asked) = structured {
                return mismatch(
                    sequence,
                    &match asked {
                        StructuredOutput::ForcedFunction { name, .. } => format!(
                            "a `text` reply cannot answer a request pinning `tool_choice: {{type: \
                             \"function\", name: \"{name}\"}}`: the Responses API answers a forced \
                             function with a call to it, never with prose and no call. Script \
                             `structured` for the object the agent should produce, or `raw` for a \
                             response generated code must reject"
                        ),
                        _ => "a `text` reply cannot answer a request carrying `text.format` of \
                              type `json_schema`: the Responses API answers that with parseable \
                              JSON, never with free prose. Script `structured` for the object the \
                              agent should produce, or `raw` for a response generated code must \
                              reject"
                            .to_string(),
                    },
                );
            }
            output.push(message_item(sequence, output.len(), text));
        }
        // Where the object goes is the request's decision (PRD §9 resolved
        // q53): the shaped final message under `text.format`, and the pinned
        // call's `arguments` under a forced function. One script, both wires'
        // mechanisms.
        ReplyBody::Structured(value) => match structured {
            None => {
                return mismatch(
                    sequence,
                    "a `structured` reply cannot answer a request that asked for no structured \
                     output: the Responses API shapes an answer only where `text.format` names a \
                     schema or `tool_choice` forces a function. Script `text`, or `raw` for a \
                     response generated code must reject",
                );
            }
            Some(StructuredOutput::ForcedFunction { name, .. }) => {
                output.push(json!({
                    "type": "function_call",
                    "id": format!("fc_mock_{sequence:08}_0"),
                    "call_id": format!("call_mock_{sequence:08}_0"),
                    "name": name,
                    "arguments": canonical(value),
                    "status": "completed",
                }));
            }
            Some(_) => output.push(message_item(sequence, output.len(), &canonical(value))),
        },
        ReplyBody::Tools {
            calls: scripted,
            text,
        } => {
            if scripted.is_empty() {
                return mismatch(
                    sequence,
                    "a `tools` reply with no calls is not an answer the Responses API can send: \
                     the turn would carry no `function_call` item at all. Script `text` for prose, \
                     or `raw` for a response generated code must reject",
                );
            }
            if let Some(text) = text {
                output.push(message_item(sequence, output.len(), text));
            }
            for (index, call) in scripted.iter().enumerate() {
                if !offered.contains(&call.name) {
                    return mismatch(
                        sequence,
                        &format!(
                            "the script calls the function `{}`, which this request does not offer",
                            call.name
                        ),
                    );
                }
                // Offered is not enough against a pin: `tool_choice: {type:
                // "function", name: X}` is a promise that X is what gets
                // called, so a call to a sibling is as impossible an answer as
                // prose is — the same rule the Messages surface keeps for its
                // own pin (PRD §9 resolved q53).
                if let Some(StructuredOutput::ForcedFunction { name, .. }) = structured
                    && &call.name != name
                {
                    return mismatch(
                        sequence,
                        &format!(
                            "the script calls the function `{}`, but this request pins \
                             `tool_choice: {{type: \"function\", name: \"{name}\"}}`: the \
                             Responses API answers a forced function with a call to *that* \
                             function and no other. Script a call to `{name}` — or `structured`, \
                             which is rendered as its arguments — or `raw` for a response \
                             generated code must reject",
                            call.name
                        ),
                    );
                }
                let call_id = call
                    .id
                    .clone()
                    .unwrap_or_else(|| format!("call_mock_{sequence:08}_{index}"));
                output.push(json!({
                    "type": "function_call",
                    "id": format!("fc_mock_{sequence:08}_{index}"),
                    "call_id": call_id,
                    "name": call.name,
                    "arguments": canonical(&call.input),
                    "status": "completed",
                }));
            }
        }
    }

    let (status, incomplete) = match checked_status(sequence, reply) {
        Ok(ended) => ended,
        Err(refusal) => return refusal,
    };
    let output = Value::Array(output);
    let usage = reply.usage.map_or_else(
        || (estimate(&canonical(request)), estimate(&canonical(&output))),
        |usage| (usage.input_tokens, usage.output_tokens),
    );

    Response::new(
        200,
        json!({
            "id": format!("resp_mock_{sequence:08}"),
            "object": "response",
            "created_at": crate::control::CREATED,
            "status": status,
            "model": model,
            "output": output,
            "incomplete_details": incomplete,
            "parallel_tool_calls": true,
            "usage": {
                "input_tokens": usage.0,
                "output_tokens": usage.1,
                // Saturating for the reason every other total on this server is:
                // a count that wrapped would panic the connection task, and a
                // dropped connection is PRD 5.9's timeout — a harness bug must
                // never arrive at generated code wearing a failover condition.
                "total_tokens": usage.0.saturating_add(usage.1),
            },
        }),
    )
    .header("x-request-id", request_id(sequence))
    .after(reply.delay)
    .answer()
}

/// One assistant `message` item carrying text.
///
/// `index` is the item's own position in the turn, and it is in the id because a
/// turn can carry **more than one** `message`: a preamble before a server tool
/// ran and the answer after it are two items, and two items of one response
/// sharing an id would be a shape no service sends.
fn message_item(sequence: u64, index: usize, text: &str) -> Value {
    json!({
        "type": "message",
        "id": format!("msg_mock_{sequence:08}_{index}"),
        "status": "completed",
        "role": "assistant",
        "content": [{ "type": "output_text", "text": text, "annotations": [] }],
    })
}

/// The items a scripted server-tool use becomes on this wire (Decision D122).
///
/// One item per use — `<type>_call` — carrying the status the service reports
/// and, where the script named one, what it found. There is no separate result
/// item on this surface: the Responses API folds the call and its outcome into
/// one item, which is the shape a compiled graph replays back.
///
/// Held to the request the same way the Messages wire's is: a provider runs only
/// the server tools its `tools` array carries.
///
/// A use that scripted a **preamble** ([`ServerToolUse::preamble`]) puts a
/// `message` item in front of its own: what the model said before the tool ran.
/// That is what makes a turn carry two `message` items, which is the shape a
/// native structured-output reader has to get right — `text.format` shapes the
/// **last** message and says nothing about the ones before it (PRD §9 resolved
/// q53).
fn server_tool_items(
    sequence: u64,
    request: &Value,
    uses: &[ServerToolUse],
) -> Result<Vec<Value>, Answer> {
    let mut items = Vec::new();
    for (index, use_) in uses.iter().enumerate() {
        if let Some(said) = &use_.preamble {
            if said.is_empty() {
                return Err(mismatch(
                    sequence,
                    "the script runs a server tool preceded by the empty string: a `message` item \
                     whose only `output_text` is empty is not a turn this API sends. Drop the \
                     preamble, or script a `raw` response",
                ));
            }
            items.push(message_item(sequence, items.len(), said));
        }
        if !declares_server_tool(request, &use_.type_name) {
            return Err(mismatch(
                sequence,
                &format!(
                    "the script runs the server tool `{}`, which this request does not declare: a \
                     provider runs only the server tools its `tools` array carries. Declare it on \
                     the provider's `server_tools:`, or script a `raw` response",
                    use_.type_name
                ),
            ));
        }
        let id = use_
            .id
            .clone()
            .unwrap_or_else(|| format!("srv_mock_{sequence:08}_{index}"));
        let mut item = json!({
            "type": format!("{}_call", use_.type_name),
            "id": id,
            "status": "completed",
        });
        if let Some(input) = &use_.input {
            item["action"] = input.clone();
        }
        if let Some(result) = &use_.result {
            item["results"] = result.clone();
        }
        items.push(item);
    }
    Ok(items)
}

fn declares_server_tool(request: &Value, type_name: &str) -> bool {
    request
        .get("tools")
        .and_then(Value::as_array)
        .is_some_and(|tools| {
            tools
                .iter()
                .any(|tool| tool.get("type").and_then(Value::as_str) == Some(type_name))
        })
}

/// The status this reply is served with, and the `incomplete_details` beside it.
///
/// A script's `stop_reason` is the field a compiled agent branches on, so an
/// override is held to what the API could have sent: either a status of its own,
/// or one of the reasons an incomplete response gives — which is the far more
/// useful one to script, since `max_output_tokens` is how a cut-off answer is
/// tested.
fn checked_status(sequence: u64, reply: &Reply) -> Result<(String, Value), Answer> {
    let Some(scripted) = reply.stop_reason.as_deref() else {
        return Ok(("completed".to_string(), Value::Null));
    };
    if INCOMPLETE_REASONS.contains(&scripted) {
        return Ok(("incomplete".to_string(), json!({ "reason": scripted })));
    }
    if STATUSES.contains(&scripted) {
        return Ok((scripted.to_string(), Value::Null));
    }
    Err(mismatch(
        sequence,
        &format!(
            "`{scripted}` is neither a `status` the Responses API ends with ({}) nor a reason it \
             gives for an incomplete one ({}). Script `raw` for a response generated code must \
             reject",
            listed(STATUSES),
            listed(INCOMPLETE_REASONS)
        ),
    ))
}

/// The answer to a script that cannot be rendered into what the request asked
/// for.
fn mismatch(sequence: u64, reason: &str) -> Answer {
    error(
        sequence,
        HARNESS_STATUS,
        "invalid_request_error",
        Some("mock_provider"),
        &format!("mock provider: {reason}"),
    )
    .harness(MISMATCH)
    .answer()
}
