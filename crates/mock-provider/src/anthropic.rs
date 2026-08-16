//! The Anthropic Messages surface: `POST /v1/messages`.
//!
//! This is the surface an `anthropic` provider (grammar 12.1) reaches, and it is
//! where PRD 5.2's load-bearing promise is kept: an agent's structured output
//! arrives as **forced tool use**. The request carries the agent's output schema
//! as a tool's `input_schema` and pins `tool_choice: {type: "tool", name: …}`;
//! the answer is one `tool_use` block whose `input` is the structured object. A
//! script that says `Outcome::structured(…)` is rendered exactly that way, so a
//! test writes the object the agent should produce and never writes wire shapes.
//!
//! # What is checked
//!
//! Everything the real API refuses, as far as this file knows it, and nothing it
//! accepts. The checks that matter most are the ones a compiled graph can get
//! wrong on its own:
//!
//! * the **tool loop's message list** — roles alternate, `tool_result` blocks
//!   lead the user turn that answers an assistant's `tool_use` blocks, every
//!   `tool_use` id is answered, and no `tool_result` answers an id nothing
//!   asked;
//! * the **tool surface** — names unique and well formed, `input_schema` an
//!   object schema, and `tools` present whenever a tool block appears anywhere
//!   in the conversation;
//! * the **required envelope** — `model`, `messages`, `max_tokens`, the
//!   `x-api-key` and `anthropic-version` headers, and a JSON content type.
//!
//! Assumptions about the wire shape that could not be confirmed without a live
//! call are listed in `WIRE-NOTES.md`, each with what would confirm it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::control::{
    Failure, Outcome, Reply, ReplyBody, StructuredOutput, ValidationFailure, canonical, estimate,
};
use crate::strict::{Checker, Dialect, Kind, at};
use crate::wire::{Answer, HARNESS_STATUS, INVALID, MISMATCH, Response, UNSCRIPTED};

/// The top-level keys the Messages API accepts.
///
/// A curated subset: every key a compiled graph could send, plus the sampling
/// knobs grammar 12.2's `settings:` vocabulary can put on the wire. Adding one
/// is a line here; accepting an unknown one silently is the bug this list
/// exists to prevent.
const REQUEST_KEYS: &[&str] = &[
    "model",
    "messages",
    "max_tokens",
    "system",
    "temperature",
    "top_p",
    "top_k",
    "stop_sequences",
    "stream",
    "metadata",
    "tools",
    "tool_choice",
    "thinking",
    "service_tier",
];

/// What the surface understood about a request.
pub(crate) struct Parsed {
    pub(crate) model: String,
    pub(crate) failures: Vec<ValidationFailure>,
    pub(crate) tools: Vec<String>,
    pub(crate) structured_output: Option<StructuredOutput>,
}

/// Read and check one request.
pub(crate) fn parse(headers: &BTreeMap<String, String>, body: Option<&Value>) -> Parsed {
    let mut checker = Checker::new(Dialect::Anthropic);
    check_headers(&mut checker, headers);

    let Some(body) = body.and_then(Value::as_object) else {
        checker.fail(
            "",
            "The request body is not a JSON object: could not parse the request body as JSON.",
        );
        return Parsed {
            model: String::new(),
            failures: checker.into_failures(),
            tools: Vec::new(),
            structured_output: None,
        };
    };

    checker.closed("", body, REQUEST_KEYS);
    let model = checker
        .required_string("", body, "model")
        .unwrap_or_default()
        .to_string();
    if model.is_empty() && body.contains_key("model") {
        checker.fail("model", "model: Field required");
    }
    check_max_tokens(&mut checker, body);
    check_system(&mut checker, body);
    let tools = check_tools(&mut checker, body);
    let forced = check_tool_choice(&mut checker, body, &tools);
    let uses_tool_blocks = check_messages(&mut checker, body);
    if uses_tool_blocks && !body.contains_key("tools") {
        checker.fail(
            "tools",
            "messages: Requests which include `tool_use` or `tool_result` blocks must define tools.",
        );
    }
    check_streaming(&mut checker, body);

    let structured_output = forced.and_then(|name| {
        let schema = tool_schema(body, &name)?;
        Some(StructuredOutput::ForcedTool { name, schema })
    });

    Parsed {
        model,
        failures: checker.into_failures(),
        tools: tools.into_iter().collect(),
        structured_output,
    }
}

/// The headers the API requires. A generated client that forgets one is a
/// codegen bug the first live call would find; this finds it in CI instead.
fn check_headers(checker: &mut Checker, headers: &BTreeMap<String, String>) {
    let present = |name: &str| headers.get(name).is_some_and(|value| !value.is_empty());
    if !present("x-api-key") {
        checker.fail(
            "headers.x-api-key",
            "x-api-key header is required: authentication failed.",
        );
    }
    if !present("anthropic-version") {
        checker.fail(
            "headers.anthropic-version",
            "anthropic-version header is required.",
        );
    }
    let json = headers
        .get("content-type")
        .is_some_and(|value| value.starts_with("application/json"));
    if !json {
        checker.fail(
            "headers.content-type",
            "content-type header must be application/json.",
        );
    }
}

fn check_max_tokens(checker: &mut Checker, body: &Map<String, Value>) {
    let Some(value) = checker.required("", body, "max_tokens") else {
        return;
    };
    let Some(max_tokens) = checker
        .typed("max_tokens", value, Kind::Integer)
        .and_then(Value::as_i64)
    else {
        return;
    };
    if max_tokens < 1 {
        checker.fail(
            "max_tokens",
            "max_tokens: Input should be greater than or equal to 1",
        );
    }
}

/// `system` is a string or a list of text blocks, and nothing else.
fn check_system(checker: &mut Checker, body: &Map<String, Value>) {
    match body.get("system") {
        None | Some(Value::Null) | Some(Value::String(_)) => {}
        Some(Value::Array(blocks)) => {
            for (index, block) in blocks.iter().enumerate() {
                let pointer = at("system", index);
                let Some(block) = checker
                    .typed(&pointer, block, Kind::Object)
                    .and_then(Value::as_object)
                else {
                    continue;
                };
                checker.closed(&pointer, block, &["type", "text", "cache_control"]);
                if let Some(kind) = checker.required(&pointer, block, "type") {
                    checker.one_of(&at(&pointer, "type"), kind, &["text"]);
                }
                checker.required_string(&pointer, block, "text");
            }
        }
        Some(other) => {
            checker.fail(
                "system",
                format!(
                    "system: Input should be a valid string or list of text blocks, not {}",
                    shape(other)
                ),
            );
        }
    }
}

/// The tool surface, returned in request order so a test can assert on which
/// tools an agent's `tools:` and `stores:` lists put on the wire (11.5).
fn check_tools(checker: &mut Checker, body: &Map<String, Value>) -> Vec<String> {
    let mut names = Vec::new();
    let Some(tools) = checker.optional("", body, "tools", Kind::Array) else {
        return names;
    };
    let mut seen = BTreeSet::new();
    for (index, tool) in tools.as_array().into_iter().flatten().enumerate() {
        let pointer = at("tools", index);
        let Some(tool) = checker
            .typed(&pointer, tool, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        checker.closed(
            &pointer,
            tool,
            &[
                "name",
                "description",
                "input_schema",
                "cache_control",
                "type",
            ],
        );
        let Some(name) = checker.required_string(&pointer, tool, "name") else {
            continue;
        };
        let name = name.to_string();
        if !well_formed(&name) {
            checker.fail(
                &at(&pointer, "name"),
                format!(
                    "{}: String should match pattern '^[a-zA-Z0-9_-]{{1,64}}$'",
                    at(&pointer, "name")
                ),
            );
        }
        if !seen.insert(name.clone()) {
            checker.fail(
                &at(&pointer, "name"),
                format!("tools: Duplicate tool name `{name}`."),
            );
        }
        if let Some(schema) = checker.required_object(&pointer, tool, "input_schema") {
            let kind = schema.get("type").and_then(Value::as_str);
            if kind != Some("object") {
                checker.fail(
                    &at(&pointer, "input_schema.type"),
                    format!(
                        "{}: Input should be 'object'",
                        at(&pointer, "input_schema.type")
                    ),
                );
            }
        }
        names.push(name);
    }
    names
}

/// `tool_choice`, and the name of the tool it forces if it forces one.
fn check_tool_choice(
    checker: &mut Checker,
    body: &Map<String, Value>,
    tools: &[String],
) -> Option<String> {
    let choice = checker.optional("", body, "tool_choice", Kind::Object)?;
    let choice = choice.as_object()?;
    checker.closed(
        "tool_choice",
        choice,
        &["type", "name", "disable_parallel_tool_use"],
    );
    let kind = checker.required("tool_choice", choice, "type")?;
    let kind = checker
        .one_of("tool_choice.type", kind, &["auto", "any", "tool", "none"])?
        .to_string();

    let named = match choice.get("name") {
        Some(name) if kind == "tool" => checker
            .typed("tool_choice.name", name, Kind::String)
            .and_then(Value::as_str)
            .map(str::to_string),
        Some(_) => {
            checker.fail(
                "tool_choice.name",
                format!("tool_choice.name: Extra inputs are not permitted with type `{kind}`"),
            );
            None
        }
        None if kind == "tool" => {
            checker.fail("tool_choice.name", "tool_choice.name: Field required");
            None
        }
        None => None,
    };

    if let Some(name) = &named
        && !tools.contains(name)
    {
        checker.fail(
            "tool_choice.name",
            format!("tool_choice: `{name}` is not one of the tools this request defines."),
        );
        return None;
    }

    match kind.as_str() {
        // A forced tool is how structured output is asked for.
        "tool" => named,
        // `any` forces *some* tool; with exactly one on offer it is the same
        // request. With several it is a genuine choice, and no schema is pinned.
        "any" if tools.len() == 1 => tools.first().cloned(),
        _ => None,
    }
}

/// The schema a named tool declares.
fn tool_schema(body: &Map<String, Value>, name: &str) -> Option<Value> {
    body.get("tools")?
        .as_array()?
        .iter()
        .find(|tool| tool.get("name").and_then(Value::as_str) == Some(name))?
        .get("input_schema")
        .cloned()
}

/// The message list. Returns whether any tool block appears in it, which is what
/// makes `tools:` required at the top level.
fn check_messages(checker: &mut Checker, body: &Map<String, Value>) -> bool {
    let Some(messages) = checker.required_array("", body, "messages") else {
        return false;
    };
    if messages.is_empty() {
        checker.fail("messages", "messages: at least one message is required");
        return false;
    }

    let mut uses_tool_blocks = false;
    let mut previous_role: Option<String> = None;
    // The `tool_use` ids the previous assistant turn is waiting on.
    let mut awaiting: Vec<String> = Vec::new();

    for (index, message) in messages.iter().enumerate() {
        let pointer = at("messages", index);
        let Some(message) = checker
            .typed(&pointer, message, Kind::Object)
            .and_then(Value::as_object)
        else {
            previous_role = None;
            continue;
        };
        checker.closed(&pointer, message, &["role", "content"]);
        let Some(role) = checker.required(&pointer, message, "role") else {
            continue;
        };
        let Some(role) = checker
            .one_of(&at(&pointer, "role"), role, &["user", "assistant"])
            .map(str::to_string)
        else {
            continue;
        };

        if index == 0 && role != "user" {
            checker.fail(
                &at(&pointer, "role"),
                "messages: first message must use the \"user\" role",
            );
        }
        if previous_role.as_deref() == Some(role.as_str()) {
            checker.fail(
                &at(&pointer, "role"),
                format!(
                    "messages: roles must alternate between \"user\" and \"assistant\", but found multiple \"{role}\" roles in a row"
                ),
            );
        }

        let blocks = check_content(checker, &pointer, message, &role);
        uses_tool_blocks |= blocks.uses_tool_blocks;

        // The correlation rule, in both directions: an assistant turn's
        // `tool_use` ids must be answered by the very next message, and a
        // `tool_result` must answer an id that turn actually asked for.
        if role == "user" {
            for (answered, block) in &blocks.tool_results {
                if !awaiting.contains(answered) {
                    checker.fail(
                        &at(&pointer, format!("content.{block}.tool_use_id")),
                        format!(
                            "messages.{index}: `tool_result` block(s) provided for `tool_use_id` `{answered}`, which no preceding `tool_use` block asked for."
                        ),
                    );
                }
            }
            let answered: BTreeSet<&String> =
                blocks.tool_results.iter().map(|(id, _)| id).collect();
            let unanswered: Vec<&String> = awaiting
                .iter()
                .filter(|id| !answered.contains(id))
                .collect();
            if !unanswered.is_empty() {
                report_unanswered(checker, index.saturating_sub(1), &unanswered);
            }
            awaiting.clear();
        } else {
            if !awaiting.is_empty() {
                let unanswered: Vec<&String> = awaiting.iter().collect();
                report_unanswered(checker, index.saturating_sub(1), &unanswered);
            }
            awaiting = blocks.tool_uses;
        }

        previous_role = Some(role);
    }

    // A conversation that ends with unanswered `tool_use` ids is the tool loop
    // handing the model back its own question.
    if !awaiting.is_empty() {
        let unanswered: Vec<&String> = awaiting.iter().collect();
        report_unanswered(checker, messages.len() - 1, &unanswered);
    }

    uses_tool_blocks
}

fn report_unanswered(checker: &mut Checker, message: usize, ids: &[&String]) {
    let list = ids
        .iter()
        .map(|id| format!("`{id}`"))
        .collect::<Vec<_>>()
        .join(", ");
    checker.fail(
        &at("messages", message),
        format!(
            "messages.{message}: `tool_use` ids were found without `tool_result` blocks immediately after: {list}. Each `tool_use` block must have a corresponding `tool_result` block in the next message."
        ),
    );
}

/// What one message's content turned out to hold.
#[derive(Default)]
struct Blocks {
    uses_tool_blocks: bool,
    /// The ids of this message's `tool_use` blocks, in order.
    tool_uses: Vec<String>,
    /// The ids its `tool_result` blocks answer, with each block's index.
    tool_results: Vec<(String, usize)>,
}

fn check_content(
    checker: &mut Checker,
    pointer: &str,
    message: &Map<String, Value>,
    role: &str,
) -> Blocks {
    let mut blocks = Blocks::default();
    let Some(content) = checker.required(pointer, message, "content") else {
        return blocks;
    };
    let content = match content {
        // A string is shorthand for one text block, and always legal.
        Value::String(_) => return blocks,
        Value::Array(blocks) => blocks,
        other => {
            checker.fail(
                &at(pointer, "content"),
                format!(
                    "{}: Input should be a valid string or list of content blocks, not {}",
                    at(pointer, "content"),
                    shape(other)
                ),
            );
            return blocks;
        }
    };
    if content.is_empty() {
        checker.fail(
            &at(pointer, "content"),
            format!(
                "{}: List should have at least 1 item",
                at(pointer, "content")
            ),
        );
        return blocks;
    }

    // `tool_result` blocks lead the turn that answers them: the API requires it,
    // and a tool loop that appends results after the next user text is exactly
    // the codegen mistake this catches.
    let mut seen_other = false;

    for (index, block) in content.iter().enumerate() {
        let pointer = at(pointer, format!("content.{index}"));
        let Some(block) = checker
            .typed(&pointer, block, Kind::Object)
            .and_then(Value::as_object)
        else {
            continue;
        };
        let Some(kind) = checker.required(&pointer, block, "type") else {
            continue;
        };
        let Some(kind) = checker
            .one_of(
                &at(&pointer, "type"),
                kind,
                &["text", "tool_use", "tool_result", "thinking"],
            )
            .map(str::to_string)
        else {
            continue;
        };

        match kind.as_str() {
            "text" => {
                seen_other = true;
                checker.closed(
                    &pointer,
                    block,
                    &["type", "text", "cache_control", "citations"],
                );
                checker.required_string(&pointer, block, "text");
            }
            "thinking" => {
                seen_other = true;
                checker.closed(&pointer, block, &["type", "thinking", "signature"]);
                checker.required_string(&pointer, block, "thinking");
            }
            "tool_use" => {
                seen_other = true;
                blocks.uses_tool_blocks = true;
                checker.closed(
                    &pointer,
                    block,
                    &["type", "id", "name", "input", "cache_control"],
                );
                if role != "assistant" {
                    checker.fail(
                        &pointer,
                        format!(
                            "{pointer}: `tool_use` blocks are only valid in an assistant turn."
                        ),
                    );
                }
                if let Some(id) = checker.required_string(&pointer, block, "id") {
                    blocks.tool_uses.push(id.to_string());
                }
                checker.required_string(&pointer, block, "name");
                checker.required_object(&pointer, block, "input");
            }
            "tool_result" => {
                blocks.uses_tool_blocks = true;
                checker.closed(
                    &pointer,
                    block,
                    &[
                        "type",
                        "tool_use_id",
                        "content",
                        "is_error",
                        "cache_control",
                    ],
                );
                if role != "user" {
                    checker.fail(
                        &pointer,
                        format!("{pointer}: `tool_result` blocks are only valid in a user turn."),
                    );
                }
                if seen_other {
                    checker.fail(
                        &pointer,
                        format!(
                            "{pointer}: Expected `tool_result` block(s) at the beginning of this message."
                        ),
                    );
                }
                if let Some(id) = checker.required_string(&pointer, block, "tool_use_id") {
                    blocks.tool_results.push((id.to_string(), index));
                }
                check_tool_result_content(checker, &pointer, block);
                checker.optional(&pointer, block, "is_error", Kind::Boolean);
            }
            _ => unreachable!("`one_of` admitted only the four kinds above"),
        }
    }

    blocks
}

/// A `tool_result`'s content: a string, or a list of text blocks. Absent is
/// legal and means an empty result.
fn check_tool_result_content(checker: &mut Checker, pointer: &str, block: &Map<String, Value>) {
    match block.get("content") {
        None | Some(Value::Null) | Some(Value::String(_)) => {}
        Some(Value::Array(parts)) => {
            for (index, part) in parts.iter().enumerate() {
                let pointer = at(pointer, format!("content.{index}"));
                let Some(part) = checker
                    .typed(&pointer, part, Kind::Object)
                    .and_then(Value::as_object)
                else {
                    continue;
                };
                if let Some(kind) = checker.required(&pointer, part, "type") {
                    checker.one_of(&at(&pointer, "type"), kind, &["text"]);
                }
            }
        }
        Some(other) => {
            checker.fail(
                &at(pointer, "content"),
                format!(
                    "{}: Input should be a valid string or list of content blocks, not {}",
                    at(pointer, "content"),
                    shape(other)
                ),
            );
        }
    }
}

/// Streaming is out of scope, and saying so loudly beats answering a request
/// this server cannot answer correctly.
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

fn shape(value: &Value) -> &'static str {
    match value {
        Value::Null => "null",
        Value::Bool(_) => "a boolean",
        Value::Number(_) => "a number",
        Value::String(_) => "a string",
        Value::Array(_) => "a list",
        Value::Object(_) => "an object",
    }
}

/// Render a served outcome into the Messages API's wire shape.
pub(crate) fn render(
    sequence: u64,
    request: &Value,
    parsed_structured: Option<&StructuredOutput>,
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
        Outcome::Reply(reply) => reply_answer(sequence, request, parsed_structured, reply),
    }
}

fn reply_answer(
    sequence: u64,
    request: &Value,
    structured: Option<&StructuredOutput>,
    reply: &Reply,
) -> Answer {
    let offered: Vec<String> = request
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let (content, implied) = match &reply.body {
        ReplyBody::Text(text) => (vec![text_block(text)], "end_turn"),
        ReplyBody::Structured(value) => {
            let Some(StructuredOutput::ForcedTool { name, .. }) = structured else {
                return mismatch(
                    "a `structured` reply needs a request that forces a tool: this one carries no \
                     `tool_choice: {type: \"tool\", name: …}`, so there is no schema to answer",
                );
            };
            (vec![tool_use_block(sequence, 0, name, value)], "tool_use")
        }
        ReplyBody::Tools { calls, text } => {
            let mut content = Vec::new();
            if let Some(text) = text {
                content.push(text_block(text));
            }
            for call in calls {
                if !offered.contains(&call.name) {
                    return mismatch(&format!(
                        "the script calls the tool `{}`, which this request does not offer",
                        call.name
                    ));
                }
                let mut block = tool_use_block(sequence, content.len(), &call.name, &call.input);
                if let Some(id) = &call.id {
                    block["id"] = json!(id);
                }
                content.push(block);
            }
            (content, "tool_use")
        }
    };

    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let stop_reason = reply
        .stop_reason
        .clone()
        .unwrap_or_else(|| implied.to_string());
    let content = Value::Array(content);
    let usage = reply.usage.map_or_else(
        || {
            json!({
                "input_tokens": estimate(&canonical(request)),
                "output_tokens": estimate(&canonical(&content)),
            })
        },
        |usage| json!({ "input_tokens": usage.input_tokens, "output_tokens": usage.output_tokens }),
    );

    Response::new(
        200,
        json!({
            "id": format!("msg_mock_{sequence:08}"),
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": content,
            "stop_reason": stop_reason,
            "stop_sequence": Value::Null,
            "usage": usage,
        }),
    )
    .header("request-id", format!("req_mock_{sequence:08}"))
    .after(reply.delay)
    .answer()
}

fn text_block(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

fn tool_use_block(sequence: u64, index: usize, name: &str, input: &Value) -> Value {
    json!({
        "type": "tool_use",
        "id": format!("toolu_mock_{sequence:08}_{index}"),
        "name": name,
        "input": input,
    })
}

fn failure_answer(failure: &Failure) -> Answer {
    match failure {
        Failure::RateLimit {
            retry_after_seconds,
        } => {
            let response = error(
                429,
                "rate_limit_error",
                "Number of requests has exceeded your per-minute rate limit.",
            );
            match retry_after_seconds {
                Some(seconds) => response.header("retry-after", seconds.to_string()),
                None => response,
            }
            .answer()
        }
        Failure::Overloaded => error(529, "overloaded_error", "Overloaded").answer(),
        Failure::ServerError => error(
            500,
            "api_error",
            "Internal server error. Please try again later.",
        )
        .answer(),
        Failure::Timeout { delay } => Answer::Close(*delay),
    }
}

/// The Messages API's error envelope.
fn error(status: u16, kind: &str, message: &str) -> Response {
    Response::new(
        status,
        json!({
            "type": "error",
            "error": { "type": kind, "message": message },
        }),
    )
}

/// The answer to a request that failed validation.
pub(crate) fn rejected(failures: &[ValidationFailure]) -> Answer {
    let message = failures
        .iter()
        .map(|failure| failure.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    error(400, "invalid_request_error", &message)
        .harness(INVALID)
        .answer()
}

/// The answer to a request no scripted outcome answered.
pub(crate) fn unscripted(model: &str, reason: &str) -> Answer {
    error(
        HARNESS_STATUS,
        "invalid_request_error",
        &format!(
            "mock provider: no scripted outcome for model `{model}` ({reason}). \
             Enqueue one through POST /_mock/enqueue before the graph runs."
        ),
    )
    .harness(UNSCRIPTED)
    .answer()
}

/// The answer to a script that cannot be rendered into what the request asked
/// for. A test bug, and loud about being one.
fn mismatch(reason: &str) -> Answer {
    error(
        HARNESS_STATUS,
        "invalid_request_error",
        &format!("mock provider: {reason}"),
    )
    .harness(MISMATCH)
    .answer()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers() -> BTreeMap<String, String> {
        [
            ("x-api-key", "test-key"),
            ("anthropic-version", "2023-06-01"),
            ("content-type", "application/json"),
        ]
        .into_iter()
        .map(|(name, value)| (name.to_string(), value.to_string()))
        .collect()
    }

    fn check(body: Value) -> Vec<String> {
        parse(&headers(), Some(&body))
            .failures
            .into_iter()
            .map(|failure| failure.pointer)
            .collect()
    }

    fn messages(body: Value) -> Value {
        let mut request = json!({
            "model": "claude-sonnet-4-6",
            "max_tokens": 1024,
            "messages": [],
        });
        for (key, value) in body.as_object().expect("an object") {
            request[key] = value.clone();
        }
        request
    }

    /// The shape a compiled graph's first model call takes: a system prompt, one
    /// user turn, the agent's output schema as a forced tool.
    #[test]
    fn a_structured_output_request_is_accepted_and_understood() {
        let request = messages(json!({
            "system": "You are a meticulous technical reviewer.",
            "messages": [{ "role": "user", "content": "{\"goal\":\"ship it\"}" }],
            "tools": [{
                "name": "extract",
                "description": "The agent's structured output.",
                "input_schema": { "type": "object", "properties": { "verdict": { "type": "string" } } },
            }],
            "tool_choice": { "type": "tool", "name": "extract" },
        }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        assert_eq!(parsed.model, "claude-sonnet-4-6");
        assert_eq!(parsed.tools, ["extract"]);
        let Some(StructuredOutput::ForcedTool { name, schema }) = parsed.structured_output else {
            panic!("the forced tool is the structured-output surface");
        };
        assert_eq!(name, "extract");
        assert_eq!(schema["type"], "object");
    }

    /// The envelope: what the API requires and this server refuses to guess.
    #[test]
    fn the_required_envelope_is_required() {
        assert_eq!(
            check(json!({ "messages": [{ "role": "user", "content": "hi" }] })),
            ["model", "max_tokens"]
        );
        assert_eq!(check(messages(json!({}))), ["messages"]);
    }

    /// Headers are part of the request, and a missing one is a codegen bug the
    /// first live call would find.
    #[test]
    fn the_required_headers_are_required() {
        let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let mut without = headers();
        without.remove("anthropic-version");
        without.insert("content-type".to_string(), "text/plain".to_string());
        let failures: Vec<String> = parse(&without, Some(&request))
            .failures
            .into_iter()
            .map(|failure| failure.pointer)
            .collect();
        assert_eq!(
            failures,
            ["headers.anthropic-version", "headers.content-type"]
        );
    }

    /// An unknown top-level key is a refusal, not an ignored setting.
    #[test]
    fn an_unknown_request_key_is_refused() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "temperatur": 0.2,
        }));
        let parsed = parse(&headers(), Some(&request));
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(parsed.failures[0].pointer, "temperatur");
        assert_eq!(
            parsed.failures[0].message,
            "temperatur: Extra inputs are not permitted"
        );
    }

    /// Roles alternate, and the conversation opens on the user.
    #[test]
    fn roles_alternate_and_open_on_the_user() {
        assert_eq!(
            check(messages(json!({
                "messages": [{ "role": "assistant", "content": "hi" }],
            }))),
            ["messages.0.role"]
        );
        assert_eq!(
            check(messages(json!({
                "messages": [
                    { "role": "user", "content": "one" },
                    { "role": "user", "content": "two" },
                ],
            }))),
            ["messages.1.role"]
        );
    }

    /// The tool loop's message list, correct: assistant asks, the next user turn
    /// answers with `tool_result` blocks first.
    #[test]
    fn a_well_formed_tool_loop_is_accepted() {
        let request = messages(json!({
            "messages": [
                { "role": "user", "content": "search for it" },
                { "role": "assistant", "content": [
                    { "type": "text", "text": "looking" },
                    { "type": "tool_use", "id": "toolu_1", "name": "web_search", "input": { "query": "it" } },
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "found" },
                    { "type": "text", "text": "carry on" },
                ]},
            ],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
    }

    /// A `tool_result` that does not lead its turn: the ordering mistake a tool
    /// loop makes when it appends results after the next user text.
    #[test]
    fn a_tool_result_must_lead_the_turn_that_answers() {
        let request = messages(json!({
            "messages": [
                { "role": "user", "content": "search for it" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {} },
                ]},
                { "role": "user", "content": [
                    { "type": "text", "text": "carry on" },
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "found" },
                ]},
            ],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.2.content.1");
        assert!(
            failures[0]
                .message
                .contains("at the beginning of this message"),
            "{}",
            failures[0].message
        );
    }

    /// An unanswered `tool_use` id, and a `tool_result` answering an id nothing
    /// asked for: the two halves of the correlation rule.
    #[test]
    fn tool_uses_and_tool_results_must_correspond() {
        let unanswered = messages(json!({
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {} },
                ]},
                { "role": "user", "content": [{ "type": "text", "text": "never mind" }] },
            ],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }));
        let failures = parse(&headers(), Some(&unanswered)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert!(failures[0].message.contains("`toolu_1`"), "{failures:?}");

        let stray = messages(json!({
            "messages": [
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_9", "content": "found" },
                ]},
            ],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }));
        let failures = parse(&headers(), Some(&stray)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.0.content.0.tool_use_id");
    }

    /// Tool blocks anywhere in the conversation require a tool surface.
    #[test]
    fn tool_blocks_require_tools() {
        let request = messages(json!({
            "messages": [
                { "role": "user", "content": "go" },
                { "role": "assistant", "content": [
                    { "type": "tool_use", "id": "toolu_1", "name": "web_search", "input": {} },
                ]},
                { "role": "user", "content": [
                    { "type": "tool_result", "tool_use_id": "toolu_1", "content": "ok" },
                ]},
            ],
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "tools");
    }

    /// The tool surface itself: names well formed and unique, schemas objects.
    #[test]
    fn the_tool_surface_is_checked() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [
                { "name": "web search", "input_schema": { "type": "object" } },
                { "name": "docs_search", "input_schema": { "type": "string" } },
                { "name": "docs_search", "input_schema": { "type": "object" } },
                { "name": "no_schema" },
            ],
        }));
        let failures: Vec<String> = parse(&headers(), Some(&request))
            .failures
            .into_iter()
            .map(|failure| failure.pointer)
            .collect();
        assert_eq!(
            failures,
            [
                "tools.0.name",
                "tools.1.input_schema.type",
                "tools.2.name",
                "tools.3.input_schema",
            ]
        );
    }

    /// `tool_choice` must name a tool the request actually offers — the codegen
    /// bug where the structured-output tool is renamed in one place only.
    #[test]
    fn a_forced_tool_must_be_on_offer() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "extract", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "tool", "name": "extractt" },
        }));
        let parsed = parse(&headers(), Some(&request));
        assert_eq!(parsed.failures.len(), 1);
        assert_eq!(parsed.failures[0].pointer, "tool_choice.name");
        assert!(parsed.structured_output.is_none());
    }

    /// `tool_choice: any` with one tool on offer forces that tool, which is the
    /// other shape a structured-output request takes.
    #[test]
    fn tool_choice_any_over_one_tool_is_a_forced_tool() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "extract", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "any" },
        }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        assert_eq!(
            parsed
                .structured_output
                .map(|output| output.name().to_string()),
            Some("extract".to_string())
        );
    }

    /// Streaming is out of scope and says so, rather than being answered wrong.
    #[test]
    fn streaming_is_refused_by_name() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "stream": true,
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].pointer, "stream");
        assert!(failures[0].message.contains("does not implement streaming"));
    }

    /// A structured reply becomes a `tool_use` block under the forced tool's
    /// name, with deterministic ids and a derived usage.
    #[test]
    fn a_structured_reply_renders_as_forced_tool_use() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "extract", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "tool", "name": "extract" },
        }));
        let parsed = parse(&headers(), Some(&request));
        let outcome = Outcome::structured(json!({ "verdict": "approve" }));
        let Answer::Respond(response) =
            render(3, &request, parsed.structured_output.as_ref(), &outcome)
        else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["id"], "msg_mock_00000003");
        assert_eq!(response.body["stop_reason"], "tool_use");
        assert_eq!(response.body["content"][0]["type"], "tool_use");
        assert_eq!(response.body["content"][0]["id"], "toolu_mock_00000003_0");
        assert_eq!(response.body["content"][0]["name"], "extract");
        assert_eq!(response.body["content"][0]["input"]["verdict"], "approve");
        assert_eq!(response.body["model"], "claude-sonnet-4-6");
        assert!(response.body["usage"]["input_tokens"].as_u64().unwrap() > 0);
    }

    /// A structured reply to a request that forced nothing is a *script* bug,
    /// and is refused as one rather than answered as a text block.
    #[test]
    fn a_structured_reply_without_a_forced_tool_is_a_script_mismatch() {
        let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let Answer::Respond(response) =
            render(1, &request, None, &Outcome::structured(json!({ "a": 1 })))
        else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
    }

    /// A scripted tool call the request never offered is the same kind of bug.
    #[test]
    fn a_tool_call_the_request_did_not_offer_is_a_script_mismatch() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "web_search", "input_schema": { "type": "object" } }],
        }));
        let outcome = Outcome::tool_calls(vec![crate::control::ToolCall::new(
            "web_serch",
            json!({ "query": "x" }),
        )]);
        let Answer::Respond(response) = render(1, &request, None, &outcome) else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert!(
            response.body["error"]["message"]
                .as_str()
                .unwrap()
                .contains("web_serch")
        );
    }

    /// Each failure shape carries the status and body the SDK classifies on.
    #[test]
    fn the_failure_shapes_are_the_providers_own() {
        let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let served = |outcome: Outcome| match render(1, &request, None, &outcome) {
            Answer::Respond(response) => (response.status, response.body, response.headers),
            Answer::Close(_) => panic!("this outcome answers"),
        };

        let (status, body, headers) = served(Outcome::rate_limit());
        assert_eq!(status, 429);
        assert_eq!(body["error"]["type"], "rate_limit_error");
        assert_eq!(headers["retry-after"], "1");

        let (status, body, _) = served(Outcome::overloaded());
        assert_eq!(status, 529, "Anthropic's overload status is 529, not 503");
        assert_eq!(body["error"]["type"], "overloaded_error");

        let (status, body, _) = served(Outcome::server_error());
        assert_eq!(status, 500);
        assert_eq!(body["error"]["type"], "api_error");

        let Answer::Close(delay) = render(
            1,
            &request,
            None,
            &Outcome::timeout(std::time::Duration::from_millis(5)),
        ) else {
            panic!("a timeout is delivered as no answer at all");
        };
        assert_eq!(delay.duration().as_millis(), 5);
    }

    /// A rejection carries every complaint, joined, in the API's error envelope.
    #[test]
    fn a_rejection_names_everything_that_was_wrong() {
        let failures = vec![
            ValidationFailure::new("model", "model: Field required"),
            ValidationFailure::new("max_tokens", "max_tokens: Field required"),
        ];
        let Answer::Respond(response) = rejected(&failures) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);
        assert_eq!(response.body["type"], "error");
        assert_eq!(
            response.body["error"]["message"],
            "model: Field required; max_tokens: Field required"
        );
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], INVALID);
    }

    /// The unscripted answer names the model and the reason, and carries a
    /// status no failover condition claims.
    #[test]
    fn an_unscripted_answer_names_the_model() {
        let Answer::Respond(response) = unscripted("model.fast", "the queue is empty") else {
            panic!("a refusal is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], UNSCRIPTED);
        let message = response.body["error"]["message"].as_str().unwrap();
        assert!(message.contains("model.fast"), "{message}");
        assert!(message.contains("the queue is empty"), "{message}");
    }
}
