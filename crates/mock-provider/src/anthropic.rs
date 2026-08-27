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
//!   `anthropic-version` header, and a JSON content type. Not the *presence* of
//!   `x-api-key`: a keyless provider behind a gateway is a legal composition and
//!   sends none (grammar 12.1, Decision D120), so its absence is a wire shape
//!   rather than a codegen bug — WIRE-NOTES (12). Its *shape* is still checked,
//!   because an empty `x-api-key` is neither posture;
//! * the **sampling knobs** — grammar 12.2's `settings:` vocabulary, checked for
//!   type *and* range, because the compiler range-checks them and nothing else
//!   watches what reaches the wire.
//!
//! Assumptions about the wire shape that could not be confirmed without a live
//! call are listed in `WIRE-NOTES.md`, each with what would confirm it.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Map, Value, json};

use crate::control::{
    Failure, Outcome, Reply, ReplyBody, StructuredOutput, ValidationFailure, canonical, estimate,
    request_id,
};
use crate::strict::{Checker, Dialect, Kind, at, listed};
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
    pub(crate) server_tools: Vec<String>,
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
            server_tools: Vec::new(),
            structured_output: None,
        };
    };

    checker.closed("", body, REQUEST_KEYS);
    let model = checker
        .required_string("", body, "model")
        .unwrap_or_default()
        .to_string();
    // An empty `model` is a *present* key that names nothing — the shape a
    // codegen bug takes when an id is interpolated from something absent. It is
    // asked separately from the required/typed checks above so it cannot double
    // up on a `model` that was missing or was not a string at all.
    if body.get("model").and_then(Value::as_str) == Some("") {
        checker.fail("model", "model: String should have at least 1 character");
    }
    check_max_tokens(&mut checker, body);
    check_settings(&mut checker, body);
    check_system(&mut checker, body);
    let tools = check_tools(&mut checker, body);
    let forced = check_tool_choice(&mut checker, body, &tools.names);
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
        tools: tools.names,
        server_tools: tools.server,
        structured_output,
    }
}

/// The headers the API requires. A generated client that forgets one is a
/// codegen bug the first live call would find; this finds it in CI instead.
///
/// **`x-api-key` is no longer required to be *there***, and that is a decision
/// rather than an omission: see WIRE-NOTES (12). A `base_url:` points a
/// connection at whatever it names, and grammar 12.1 lets an `anthropic`
/// provider that names one declare no `api_key:` at all (Decision D120) — a
/// gateway injects the credential server-side and the compiled graph sends no
/// authentication header. This server stands in for that endpoint as much as for
/// the vendor's, so an unauthenticated request is a wire shape it has to accept.
///
/// **Only the presence requirement was dropped.** A credential that *is* on the
/// wire is still held to its shape, because the keyless posture the runtime
/// promises is a header that is *absent*, not one that is empty: `x-api-key: ""`
/// is a request that claims to authenticate and fails, which a gateway may
/// refuse before injecting anything and a forwarding gateway turns into a 401 at
/// the vendor. So an empty `x-api-key` is the codegen bug this check is now for,
/// and it is answered the way a live 401 is.
///
/// What this surface cannot decide is a credential sent under the *wrong*
/// header. `authorization: Bearer …` on the Messages wire is the documented
/// gateway shape (`docs/topics/models.md`, "Keyless providers behind a
/// gateway"), so it is indistinguishable here from a proxy token a composition
/// declared through `headers:`. Where it *is* decidable is the acceptance suite,
/// which reads the recorded request against the spec that produced it — see
/// `a_provider_with_no_key_sends_no_authentication_header_on_either_wire`.
fn check_headers(checker: &mut Checker, headers: &BTreeMap<String, String>) {
    let present = |name: &str| headers.get(name).is_some_and(|value| !value.is_empty());
    // A credential, not a field: answered 401 rather than 400 (see `rejected`),
    // because that is the status the SDK's `AuthenticationError` comes from and
    // generated code may well classify the two apart.
    if headers.contains_key("x-api-key") && !present("x-api-key") {
        checker.credential(
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

/// Everything else the envelope carries, by type and by range.
///
/// These are the keys a `settings:` block (grammar 12.2) puts on the wire, and
/// the compiler is the only thing that has checked them so far: it bounds them
/// at *compile* time against the literal in the spec, which says nothing about
/// what a template or a codegen bug serializes at run time. A key list that only
/// asked whether the name is known would accept `temperature: "hot"` and
/// `top_p: 5` — both 400s on the real API, and both a graph that passes CI and
/// fails on its first live call.
///
/// `stream` is the sharpest of them: checked as a boolean here so that the
/// *string* `"true"` is refused as the type error it is, rather than slipping
/// past [`check_streaming`]'s `true` comparison and being answered as a
/// non-streaming call.
fn check_settings(checker: &mut Checker, body: &Map<String, Value>) {
    checker.bounded_number("", body, "temperature", 0.0..=1.0);
    checker.bounded_number("", body, "top_p", 0.0..=1.0);
    checker.bounded_integer("", body, "top_k", 0..=i64::MAX);
    checker.optional("", body, "stream", Kind::Boolean);
    checker.optional("", body, "metadata", Kind::Object);
    checker.optional("", body, "thinking", Kind::Object);
    checker.optional("", body, "service_tier", Kind::String);
    let Some(stops) = checker.optional("", body, "stop_sequences", Kind::Array) else {
        return;
    };
    for (index, stop) in stops.as_array().into_iter().flatten().enumerate() {
        checker.typed(&at("stop_sequences", index), stop, Kind::String);
    }
}

/// `system` is a string or a list of text blocks, and nothing else.
///
/// The string is the same shorthand a message's `content` takes, so an empty one
/// is the same empty text block and is refused at the same normalised address.
/// Grammar 5.4 requires an agent's `prompt:` to be a non-empty string, so no
/// correct composition can produce `system: ""` — it is a prompt that rendered
/// to nothing, which is the codegen bug this rule is for.
fn check_system(checker: &mut Checker, body: &Map<String, Value>) {
    match body.get("system") {
        None | Some(Value::Null) => {}
        Some(Value::String(text)) => {
            if text.is_empty() {
                report_empty_text(checker, "system.0.text");
            }
        }
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
                check_text(checker, &pointer, block);
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
fn check_tools(checker: &mut Checker, body: &Map<String, Value>) -> Tools {
    let mut names = Vec::new();
    let mut server = Vec::new();
    let Some(tools) = checker.optional("", body, "tools", Kind::Array) else {
        return Tools { names, server };
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
        // A **server tool**: a tool the service runs on its own side, named by a
        // `type:` out of Anthropic's own vocabulary (grammar 12.1, Decision
        // D122). Its config keys are the vendor's, not this API's closed entry
        // shape, and this server cannot know which are legal for a dated type it
        // may predate — so the entry is recorded and carried, and only the one
        // thing every tool entry needs is required: a `name`. `WIRE-NOTES` (22).
        //
        // That name goes into the same `seen` set a client tool's does, because
        // the array is one namespace: the API refuses a request offering two
        // tools under one name whichever side runs them, and a server that let
        // `code_execution_20250522` sit beside `code_execution_20250825` would
        // accept the one request the compiler's canonical-name pinning exists
        // to stop (grammar 12.1, `check::providers`'s `suite_collisions`).
        if let Some(kind) = tool.get("type").and_then(Value::as_str)
            && kind != "custom"
        {
            server.push(kind.to_string());
            if let Some(name) = checker.required_string(&pointer, tool, "name") {
                let name = name.to_string();
                if !seen.insert(name.clone()) {
                    checker.fail(
                        &at(&pointer, "name"),
                        format!("tools: Duplicate tool name `{name}`."),
                    );
                }
            }
            continue;
        }
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
    Tools { names, server }
}

/// What a request's `tools` array holds, split by who runs them.
pub(crate) struct Tools {
    /// The client tools, by name, in request order — what the graph dispatches.
    pub(crate) names: Vec<String>,
    /// The server tools, by `type:`, in request order — what the provider runs
    /// itself (grammar 12.1, Decision D122).
    pub(crate) server: Vec<String>,
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

    // The mirror of the rule tool *blocks* obey: a request that says how the
    // model must use its tools has to have some. Reported here and answered
    // early, so a `{type: "tool", name: X}` with no tool surface is one
    // complaint about the missing surface rather than two about X not being in
    // it.
    if !body.contains_key("tools") {
        checker.fail(
            "tools",
            "tool_choice: Requests which specify `tool_choice` must also define tools.",
        );
        return None;
    }

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
        // A string is shorthand for one text block — the API normalises it into
        // one and addresses its complaints at the *normalised* block, which is
        // why the refusal below is reported at `content.0.text` and not at
        // `content`. It is the spelling a compiled graph actually sends, so it
        // is the spelling the empty-text rule has to reach: a bound input that
        // rendered to nothing arrives here, not as an explicit empty block.
        Value::String(text) => {
            if text.is_empty() {
                report_empty_text(checker, &at(pointer, "content.0.text"));
            }
            return blocks;
        }
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
        // A **server-tool** block, replayed back to the wire that produced it
        // (grammar 12.1, Decision D122): the `server_tool_use` the service filed
        // and the `<name>_tool_result` it answered itself with. A compiled graph
        // must send these back unaltered and must **not** answer them — the call
        // already happened, on the provider's side — so what is checked here is
        // that they arrived at all, and their innards are the vendor's rather
        // than this closed block vocabulary. `WIRE-NOTES` (22).
        if let Some(name) = kind.as_str()
            && (name == "server_tool_use"
                || name.ends_with("_tool_result") && name != "tool_result")
        {
            seen_other = true;
            blocks.uses_tool_blocks = true;
            continue;
        }
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
                check_text(checker, &pointer, block);
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

/// One text block's `text`: required, and **not empty**.
///
/// The API refuses an empty one, and an empty text block is what a prompt or a
/// bound input that rendered to nothing looks like on the wire — a codegen bug
/// whose only symptom would otherwise be a model answering a blank turn.
fn check_text(checker: &mut Checker, pointer: &str, block: &Map<String, Value>) {
    if checker.required_string(pointer, block, "text") == Some("") {
        report_empty_text(checker, &at(pointer, "text"));
    }
}

/// The API's complaint about a text block with nothing in it, at `pointer` — the
/// address of the block's `text`.
///
/// One function because the same rule is reached three ways: an explicit block,
/// a `tool_result` part, and the single block a string `content` normalises into
/// (whose address is `content.0.text` on a message that never spelled a block).
fn report_empty_text(checker: &mut Checker, pointer: &str) {
    checker.fail(
        pointer,
        format!("{pointer}: text content blocks must be non-empty"),
    );
}

/// A `tool_result`'s content: a string, or a list of text blocks.
///
/// The key is optional here, unlike a message's `content`, so "the tool returned
/// nothing" is a thing this position can say: absent, the empty string and the
/// empty list all say it, and all three are legal. What is not legal is a text
/// **block** with nothing in it — that is a block claiming to carry text and
/// carrying none, which is what a tool whose result binding rendered to nothing
/// produces, and it is held to the rule every other text block obeys.
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
                if let Some(kind) = checker.required(&pointer, part, "type")
                    && checker
                        .one_of(&at(&pointer, "type"), kind, &["text"])
                        .is_some()
                {
                    check_text(checker, &pointer, part);
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
        Outcome::Failure(failure) => failure_answer(sequence, failure),
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
        ReplyBody::Text(text) => {
            // A request that pins tool use cannot be answered with prose. This
            // is the inverse of the `structured`-without-a-forced-tool refusal
            // below, and it exists for the same reason: a codegen PR that
            // scripted `Outcome::text` for an agent node would be testing its
            // structured-output parser against an answer the Messages API
            // cannot send, and passing.
            if let Some(pinned) = pinned_tool_use(request) {
                return mismatch(
                    sequence,
                    &format!(
                        "a `text` reply cannot answer a request carrying {pinned}: the \
                         Messages API answers a pinned tool with a `tool_use` block, never with \
                         `end_turn` and no call. Script `structured` for the object the agent \
                         should produce, or `raw` for a response generated code must reject"
                    ),
                );
            }
            (vec![text_block(text)], "end_turn")
        }
        ReplyBody::Structured(value) => {
            let Some(StructuredOutput::ForcedTool { name, .. }) = structured else {
                return mismatch(
                    sequence,
                    "a `structured` reply needs a request that forces a tool: this one carries no \
                     `tool_choice: {type: \"tool\", name: …}`, so there is no schema to answer",
                );
            };
            (vec![tool_use_block(sequence, 0, name, value)], "tool_use")
        }
        ReplyBody::Tools { calls, text } => {
            // The mirror of the two refusals above, in the other direction: a
            // request may make tool use *impossible* as surely as it can pin it,
            // and a reply the Messages API could not have sent must not be
            // rendered just because a script asked for one.
            if calls.is_empty() {
                return mismatch(
                    sequence,
                    "a `tools` reply with no calls is not an answer the Messages API can send: \
                     `stop_reason: \"tool_use\"` names the block that ended the turn, and there \
                     would be none. Script `text` for prose, or `raw` for a response generated \
                     code must reject",
                );
            }
            if let Some(forbidden) = forbidden_tool_use(request) {
                return mismatch(
                    sequence,
                    &format!(
                        "a `tools` reply cannot answer a request carrying {forbidden}: the \
                         Messages API never calls a tool the request forbade. Script `text` for \
                         prose, or `raw` for a response generated code must reject"
                    ),
                );
            }
            let pinned = pinned_tool_name(request);
            let mut content = Vec::new();
            if let Some(text) = text {
                content.push(text_block(text));
            }
            for call in calls {
                if !offered.contains(&call.name) {
                    return mismatch(
                        sequence,
                        &format!(
                            "the script calls the tool `{}`, which this request does not offer",
                            call.name
                        ),
                    );
                }
                // Offered is not enough when the request pinned one of them:
                // `tool_choice: {type: "tool", name: X}` is a promise that X is
                // what gets called, so a call to a *sibling* tool is as
                // impossible an answer as prose is, and refusing it is the same
                // rule as the two above rather than a new one.
                if let Some(pinned) = &pinned
                    && &call.name != pinned
                {
                    return mismatch(
                        sequence,
                        &format!(
                            "the script calls the tool `{}`, but this request pins `tool_choice: \
                         {{type: \"tool\", name: \"{pinned}\"}}`: the Messages API answers a \
                         pinned tool with a call to *that* tool and no other. Script a call to \
                         `{pinned}` — or `structured`, which is rendered under the pinned name — \
                         or `raw` for a response generated code must reject",
                            call.name
                        ),
                    );
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

    // Server-tool activity goes **in front** of whatever the model then said:
    // the provider ran the tool inside this turn, so its record precedes the
    // text or the calls that were written knowing what it found (grammar 12.1,
    // Decision D122).
    let content = match server_tool_blocks(sequence, request, &reply.server_tools) {
        Ok(blocks) => blocks.into_iter().chain(content).collect::<Vec<Value>>(),
        Err(refusal) => return refusal,
    };

    let model = request
        .get("model")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let (stop_reason, stop_sequence) = match checked_stop_reason(sequence, request, reply, implied)
    {
        Ok(ended) => ended,
        Err(refusal) => return refusal,
    };
    let content = Value::Array(content);
    let (input_tokens, output_tokens) = reply.usage.map_or_else(
        || {
            (
                estimate(&canonical(request)),
                estimate(&canonical(&content)),
            )
        },
        |usage| (usage.input_tokens, usage.output_tokens),
    );
    // The two cache counters are on **every** live Messages response, whether or
    // not the request asked for caching, so WIRE-NOTES §9's rule applies to them
    // as much as to `stop_sequence: null`: no client here reads them, and their
    // absence is what a strict client library trips over. Zero, because nothing
    // this server serves is cached.
    let usage = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "cache_creation_input_tokens": 0,
        "cache_read_input_tokens": 0,
    });

    Response::new(
        200,
        json!({
            "id": format!("msg_mock_{sequence:08}"),
            "type": "message",
            "role": "assistant",
            "model": model,
            "content": content,
            "stop_reason": stop_reason,
            "stop_sequence": stop_sequence,
            "usage": usage,
        }),
    )
    .header("request-id", request_id(sequence))
    .after(reply.delay)
    .answer()
}

/// The `stop_reason`s the Messages API ends a turn with.
const STOP_REASONS: &[&str] = &[
    "end_turn",
    "max_tokens",
    "stop_sequence",
    "tool_use",
    "pause_turn",
    "refusal",
];

/// The Messages API's name for a turn a `tool_use` block ended.
const TOOL_USE: &str = "tool_use";

/// How the turn ended: the `stop_reason` this reply is served with, and the
/// `stop_sequence` that goes with it.
///
/// The two travel together because the API sends them together — `stop_sequence`
/// carries the sequence that ended the turn and is `null` in every other case,
/// so a response that named one reason and the other's member would be a
/// document no live call produces.
///
/// A scripted `stop_reason` overrides the one the body implies, and is held to
/// the rule the rest of a `reply` is held to — it is what the API could have
/// sent, and anything else is [`Outcome::raw`]. Three ways it can be something
/// else:
///
/// * a value outside the closed set: `"banana"`, or Chat Completions'
///   vocabulary (`"stop"`, `"tool_calls"`) reached for out of habit;
/// * a value that contradicts the content it accompanies. `tool_use` names the
///   block that ended the turn — the sentence [`ReplyBody::Tools`] already
///   refuses an empty call list with — and `end_turn` says the model chose to
///   stop talking; neither is sayable about the other content;
/// * `stop_sequence` on a request that declared none. The API only matches
///   sequences the request supplied, so there would be nothing to name, and a
///   `stop_sequence: null` beside that reason is the same incoherence one field
///   over. When the request *does* declare them, the first is the one named:
///   the reply's text is scripted rather than generated, so there is no sequence
///   to have matched, and a deterministic choice is what a golden transcript
///   needs.
///
/// `max_tokens`, `pause_turn` and `refusal` stay legal over either content:
/// truncation, a paused turn and a refusal each cut a tool call as readily as
/// prose.
fn checked_stop_reason(
    sequence: u64,
    request: &Value,
    reply: &Reply,
    implied: &str,
) -> Result<(String, Value), Answer> {
    let Some(scripted) = reply.stop_reason.as_deref() else {
        return Ok((implied.to_string(), Value::Null));
    };
    if !STOP_REASONS.contains(&scripted) {
        return Err(mismatch(
            sequence,
            &format!(
                "`{scripted}` is not a `stop_reason` the Messages API sends: it ends a turn with \
                 one of {}. Script `raw` for a response generated code must reject",
                listed(STOP_REASONS)
            ),
        ));
    }
    if scripted == TOOL_USE && implied != TOOL_USE {
        return Err(mismatch(
            sequence,
            "a `stop_reason` of `tool_use` names the block that ended the turn, and this reply \
             carries none: the Messages API never sends it beside content with no `tool_use` \
             block. Script a `tools` reply for a call, or `raw` for a response generated code \
             must reject",
        ));
    }
    if scripted == "end_turn" && implied == TOOL_USE {
        return Err(mismatch(
            sequence,
            "a reply that carries a `tool_use` block is not ended by `stop_reason: \"end_turn\"`: \
             the Messages API names the block that ended the turn. Script `text` for prose, or \
             `raw` for a response generated code must reject",
        ));
    }
    if scripted == "stop_sequence" {
        let Some(matched) = request
            .get("stop_sequences")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .find_map(Value::as_str)
        else {
            return Err(mismatch(
                sequence,
                "a `stop_reason` of `stop_sequence` names the sequence that ended the turn, and \
                 this request declares no `stop_sequences`: the Messages API only matches ones it \
                 was given. Add them to the request, or script `raw` for a response generated \
                 code must reject",
            ));
        };
        return Ok((scripted.to_string(), Value::String(matched.to_string())));
    }
    Ok((scripted.to_string(), Value::Null))
}

/// How a request obliged the model to use a tool, if it did.
///
/// `auto` and `none` leave prose on the table; `tool` and `any` do not — both
/// guarantee the answer carries a `tool_use` block. Read off the request rather
/// than off the parsed [`StructuredOutput`] because the two differ: `any` over
/// several tools pins no *schema* (so there is nothing to render a `structured`
/// reply into) while still pinning that a tool is called.
fn pinned_tool_use(request: &Value) -> Option<String> {
    let choice = request.get("tool_choice")?.as_object()?;
    match choice.get("type").and_then(Value::as_str)? {
        "tool" => {
            let name = choice.get("name").and_then(Value::as_str).unwrap_or("…");
            Some(format!(
                "`tool_choice: {{type: \"tool\", name: \"{name}\"}}`"
            ))
        }
        "any" => Some("`tool_choice: {type: \"any\"}`".to_string()),
        _ => None,
    }
}

/// The tool a request pinned **by name**, if it pinned one.
///
/// Narrower than [`pinned_tool_use`] on purpose: `any` guarantees *a* call and
/// so rules out prose, while `{type: "tool", name: X}` additionally says which
/// tool the call is to. Read here so a `tools` reply can be held to it.
fn pinned_tool_name(request: &Value) -> Option<String> {
    let choice = request.get("tool_choice")?.as_object()?;
    (choice.get("type").and_then(Value::as_str)? == "tool")
        .then(|| choice.get("name").and_then(Value::as_str))
        .flatten()
        .map(str::to_string)
}

/// How a request forbade tool use, if it forbade it.
///
/// The exact inverse of [`pinned_tool_use`], and the reason it is its own
/// function: `auto` leaves tool use *legal*, so only `none` belongs here. A
/// request that offers no tools at all is caught further down by the
/// `offered.contains` check, which names the tool the script invented.
fn forbidden_tool_use(request: &Value) -> Option<String> {
    let choice = request.get("tool_choice")?.as_object()?;
    (choice.get("type").and_then(Value::as_str)? == "none")
        .then(|| "`tool_choice: {type: \"none\"}`".to_string())
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

/// The blocks a scripted server-tool use becomes on the Messages wire
/// (grammar 12.1, Decision D122).
///
/// **Two blocks per use, and both are the answer**: a `server_tool_use` saying
/// what the service was asked, and the `<name>_tool_result` that answers it.
/// The service ran the tool itself, so the result is already here — this is the
/// shape a compiled graph must carry through its loop *without* dispatching
/// anything, which is what the acceptance suite reads it for.
///
/// A use is held to the request the same way a scripted tool call is: a provider
/// does not run a tool it was not given, so a `type:` this request's `tools`
/// array does not declare is a `script-mismatch` rather than an answer. The
/// tool's `name` comes off that same declaration — the Messages wire carries
/// both, and the result block is named after the name rather than the dated
/// type.
fn server_tool_blocks(
    sequence: u64,
    request: &Value,
    uses: &[crate::control::ServerToolUse],
) -> Result<Vec<Value>, Answer> {
    let mut blocks = Vec::new();
    for (index, use_) in uses.iter().enumerate() {
        let Some(name) = declared_server_tool(request, &use_.type_name) else {
            return Err(mismatch(
                sequence,
                &format!(
                    "the script runs the server tool `{}`, which this request does not declare: a \
                     provider runs only the server tools its `tools` array carries. Declare it on \
                     the provider's `server_tools:`, or script a `raw` response",
                    use_.type_name
                ),
            ));
        };
        let id = use_
            .id
            .clone()
            .unwrap_or_else(|| format!("srvtoolu_mock_{sequence:08}_{index}"));
        blocks.push(json!({
            "type": "server_tool_use",
            "id": id,
            "name": name,
            "input": use_.input.clone().unwrap_or_else(|| json!({})),
        }));
        blocks.push(json!({
            "type": format!("{name}_tool_result"),
            "tool_use_id": id,
            "content": use_.result.clone().unwrap_or(Value::Null),
        }));
    }
    Ok(blocks)
}

/// The `name` this request declared for a server tool of this `type`, if it
/// declared one at all.
fn declared_server_tool(request: &Value, type_name: &str) -> Option<String> {
    request
        .get("tools")
        .and_then(Value::as_array)?
        .iter()
        .find(|tool| tool.get("type").and_then(Value::as_str) == Some(type_name))
        .and_then(|tool| tool.get("name").and_then(Value::as_str))
        .map(str::to_string)
}

fn failure_answer(sequence: u64, failure: &Failure) -> Answer {
    match failure {
        Failure::RateLimit {
            retry_after_seconds,
        } => {
            let response = error(
                sequence,
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
        Failure::Overloaded => error(sequence, 529, "overloaded_error", "Overloaded").answer(),
        Failure::ServerError => error(
            sequence,
            500,
            "api_error",
            "Internal server error. Please try again later.",
        )
        .answer(),
        Failure::Timeout { delay } => Answer::Close(*delay),
    }
}

/// The Messages API's error envelope.
///
/// It carries the request id twice — as the `request-id` header and as the
/// envelope's own `request_id` member — because the real API does, on **errors
/// as well as answers**: it is the identifier an SDK's error object surfaces and
/// the one a transcript reader correlates by (WIRE-NOTES §9). An error path that
/// dropped it would be the one place this server answered less than the provider
/// does.
fn error(sequence: u64, status: u16, kind: &str, message: &str) -> Response {
    let id = request_id(sequence);
    Response::new(
        status,
        json!({
            "type": "error",
            "error": { "type": kind, "message": message },
            "request_id": id.clone(),
        }),
    )
    .header("request-id", id)
}

/// The answer to a request that failed validation.
///
/// A missing credential outranks everything else in the list and is answered
/// **401 `authentication_error`**, because that is what the Messages API does:
/// authentication is settled before the body is looked at, so the answer names
/// the credential and nothing else. Every other failure is the 400 the API sends
/// for a body it could not accept.
pub(crate) fn rejected(sequence: u64, failures: &[ValidationFailure]) -> Answer {
    let (status, kind, reported) = classify(failures);
    let message = reported
        .iter()
        .map(|failure| failure.message.as_str())
        .collect::<Vec<_>>()
        .join("; ");
    error(sequence, status, kind, &message)
        .harness(INVALID)
        .answer()
}

/// The status, error type, and the failures a refusal reports — see
/// [`rejected`].
fn classify(failures: &[ValidationFailure]) -> (u16, &'static str, Vec<&ValidationFailure>) {
    let credentials: Vec<&ValidationFailure> = failures
        .iter()
        .filter(|failure| failure.authentication)
        .collect();
    if credentials.is_empty() {
        (400, "invalid_request_error", failures.iter().collect())
    } else {
        (401, "authentication_error", credentials)
    }
}

/// The answer to a request no scripted outcome answered.
pub(crate) fn unscripted(sequence: u64, model: &str, reason: &str) -> Answer {
    error(
        sequence,
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
fn mismatch(sequence: u64, reason: &str) -> Answer {
    error(
        sequence,
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

    /// A `model` that is present and names nothing is one mistake, and a `model`
    /// of the wrong type is a different one — never both at once.
    #[test]
    fn an_empty_model_is_its_own_mistake() {
        let mut request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        request["model"] = json!("");
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(
            failures[0].message,
            "model: String should have at least 1 character"
        );

        request["model"] = json!(7);
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].message, "model: Input should be a valid string");
    }

    /// Headers are part of the request, and a missing one is a codegen bug the
    /// first live call would find.
    #[test]
    fn the_required_headers_are_required() {
        let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let mut without = headers();
        without.remove("anthropic-version");
        without.insert("content-type".to_string(), "text/plain".to_string());
        let parsed = parse(&without, Some(&request));
        let failures: Vec<String> = parsed
            .failures
            .iter()
            .map(|failure| failure.pointer.clone())
            .collect();
        assert_eq!(
            failures,
            ["headers.anthropic-version", "headers.content-type"]
        );
        // Neither is a credential, so both draw the 400 a bad request draws.
        assert!(
            parsed
                .failures
                .iter()
                .all(|failure| !failure.authentication)
        );
        let Answer::Respond(response) = rejected(1, &parsed.failures) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 400);
        assert_eq!(response.body["error"]["type"], "invalid_request_error");

        // The *presence* of `x-api-key` is not among them: a provider that names
        // a `base_url:` may declare no `api_key:` and then sends no header at
        // all (grammar 12.1, Decision D120, WIRE-NOTES (12)), so its absence is
        // a wire shape this server accepts rather than a codegen bug it reports.
        let mut anonymous = headers();
        anonymous.remove("x-api-key");
        assert!(
            parse(&anonymous, Some(&request)).failures.is_empty(),
            "a keyless request is a legal shape on this surface"
        );

        // Its *shape* still is. An empty header is neither posture — not the
        // keyless request D120 admits and not a credential — so it is refused,
        // and refused as a credential: 401, the status the SDK raises
        // `AuthenticationError` from.
        let mut empty = headers();
        empty.insert("x-api-key".to_string(), String::new());
        let parsed = parse(&empty, Some(&request));
        assert_eq!(parsed.failures[0].pointer, "headers.x-api-key");
        assert!(parsed.failures[0].authentication);
        let Answer::Respond(response) = rejected(1, &parsed.failures) else {
            panic!("a rejection is a response");
        };
        assert_eq!(response.status, 401);
        assert_eq!(response.body["error"]["type"], "authentication_error");
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

    /// `tool_choice` says how the model must use tools, so a request that
    /// carries one without a tool surface is refused — once, naming the missing
    /// surface rather than the name that is not in it.
    #[test]
    fn a_tool_choice_without_tools_is_refused() {
        for choice in [
            json!({ "type": "auto" }),
            json!({ "type": "any" }),
            json!({ "type": "none" }),
            json!({ "type": "tool", "name": "extract" }),
        ] {
            let request = messages(json!({
                "messages": [{ "role": "user", "content": "hi" }],
                "tool_choice": choice.clone(),
            }));
            let parsed = parse(&headers(), Some(&request));
            assert_eq!(parsed.failures.len(), 1, "{choice}: {:?}", parsed.failures);
            assert_eq!(parsed.failures[0].pointer, "tools");
            assert!(
                parsed.failures[0]
                    .message
                    .contains("must also define tools"),
                "{}",
                parsed.failures[0].message
            );
            assert!(parsed.structured_output.is_none());
        }

        // The positive half: the same `tool_choice` over a tool surface is the
        // request every agent node sends.
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "extract", "input_schema": { "type": "object" } }],
            "tool_choice": { "type": "auto" },
        }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
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

    /// The sampling knobs are checked for **type and range**, not just for
    /// having a name the API knows: `temperature: "hot"` and `top_p: 5` are
    /// both 400s, and both are what a `settings:` block (grammar 12.2) looks
    /// like when codegen stringifies it or interpolates the wrong value.
    #[test]
    fn the_sampling_knobs_are_checked_for_type_and_range() {
        let complaints = |extra: Value| {
            let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
            let mut request = request;
            for (key, value) in extra.as_object().expect("an object") {
                request[key] = value.clone();
            }
            parse(&headers(), Some(&request))
                .failures
                .into_iter()
                .map(|failure| (failure.pointer, failure.message))
                .collect::<Vec<_>>()
        };

        assert_eq!(
            complaints(json!({ "temperature": "hot" })),
            [(
                "temperature".to_string(),
                "temperature: Input should be a valid number".to_string()
            )]
        );
        assert_eq!(
            complaints(json!({ "top_p": 5 })),
            [(
                "top_p".to_string(),
                "top_p: Input should be less than or equal to 1".to_string()
            )]
        );
        assert_eq!(
            complaints(json!({ "temperature": -3 })),
            [(
                "temperature".to_string(),
                "temperature: Input should be greater than or equal to 0".to_string()
            )]
        );
        assert_eq!(
            complaints(json!({ "top_k": 1.5 }))
                .into_iter()
                .map(|(pointer, _)| pointer)
                .collect::<Vec<_>>(),
            ["top_k"]
        );
        assert_eq!(
            complaints(json!({ "stop_sequences": ["ok", 7] }))
                .into_iter()
                .map(|(pointer, _)| pointer)
                .collect::<Vec<_>>(),
            ["stop_sequences.1"]
        );
        for wrong in [
            json!({ "metadata": "u1" }),
            json!({ "thinking": true }),
            json!({ "service_tier": 1 }),
        ] {
            assert_eq!(complaints(wrong.clone()).len(), 1, "{wrong}");
        }

        // The positive half: every knob at a legal value passes, including the
        // integral spelling of a fractional one.
        assert_eq!(
            complaints(json!({
                "temperature": 1,
                "top_p": 0.95,
                "top_k": 40,
                "stop_sequences": ["STOP"],
                "stream": false,
                "metadata": { "user_id": "u1" },
                "thinking": { "type": "enabled", "budget_tokens": 1024 },
                "service_tier": "auto",
            })),
            []
        );
    }

    /// `stream: "true"` is the string, not the flag — a type error, and the one
    /// that would otherwise walk straight past the streaming refusal below,
    /// which compares against the boolean `true`.
    #[test]
    fn a_stringly_typed_stream_flag_is_refused_as_a_type_error() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "stream": "true",
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "stream");
        assert_eq!(
            failures[0].message,
            "stream: Input should be a valid boolean"
        );
    }

    /// An empty text block is refused: it is what a prompt or a bound input that
    /// rendered to nothing looks like on the wire.
    #[test]
    fn an_empty_text_block_is_refused() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "" }] }],
            "system": [{ "type": "text", "text": "" }],
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.pointer.as_str())
                .collect::<Vec<_>>(),
            ["system.0.text", "messages.0.content.0.text"]
        );
        assert!(
            failures[0].message.contains("must be non-empty"),
            "{}",
            failures[0].message
        );

        // A block with something in it is the ordinary case, and it passes.
        let request = messages(json!({
            "messages": [{ "role": "user", "content": [{ "type": "text", "text": "hi" }] }],
        }));
        assert!(parse(&headers(), Some(&request)).failures.is_empty());
    }

    /// The same rule reaches the spelling a compiled graph actually sends: a
    /// **string** `content`, which the API normalises into one text block and
    /// refuses when it is empty — at the normalised block's address, which is
    /// why the pointer names a block the request never spelled.
    ///
    /// This is the shape the empty-text rule exists for. Every acceptance
    /// fixture sends its user turn as a plain string, so a codegen bug that
    /// bound an input to nothing arrives here and nowhere else.
    #[test]
    fn an_empty_string_content_is_refused_as_the_block_it_normalizes_into() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "" }],
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.0.content.0.text");
        assert_eq!(
            failures[0].message,
            "messages.0.content.0.text: text content blocks must be non-empty"
        );

        // `system` takes the same shorthand and answers the same way — an
        // agent's `prompt:` is a non-empty string (grammar 5.4), so an empty
        // system is a prompt that rendered to nothing.
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "system": "",
        }));
        let failures = parse(&headers(), Some(&request)).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "system.0.text");

        // …and both string spellings with something in them are the ordinary
        // request, which must stay accepted: the rule is about emptiness, not
        // about the shorthand.
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "review it" }],
            "system": "You are a meticulous technical reviewer.",
        }));
        assert!(parse(&headers(), Some(&request)).failures.is_empty());
    }

    /// A `tool_result`'s text parts obey the rule too. A tool whose result
    /// binding rendered to nothing produces exactly this — a part with no `text`
    /// at all, or an empty one — and it is a block the real API refuses.
    #[test]
    fn a_tool_result_part_is_held_to_the_empty_text_rule() {
        let loop_request = |content: Value| {
            messages(json!({
                "messages": [
                    { "role": "user", "content": "look it up" },
                    { "role": "assistant", "content": [{
                        "type": "tool_use",
                        "id": "toolu_1",
                        "name": "lookup",
                        "input": { "query": "a fact" },
                    }] },
                    { "role": "user", "content": [{
                        "type": "tool_result",
                        "tool_use_id": "toolu_1",
                        "content": content,
                    }] },
                ],
                "tools": [{ "name": "lookup", "input_schema": { "type": "object" } }],
            }))
        };

        let failures = parse(&headers(), Some(&loop_request(json!([{ "type": "text" }])))).failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.2.content.0.content.0.text");
        assert_eq!(
            failures[0].message,
            "messages.2.content.0.content.0.text: Field required"
        );

        let failures = parse(
            &headers(),
            Some(&loop_request(json!([{ "type": "text", "text": "" }]))),
        )
        .failures;
        assert_eq!(failures.len(), 1, "{failures:?}");
        assert_eq!(failures[0].pointer, "messages.2.content.0.content.0.text");
        assert!(
            failures[0].message.contains("must be non-empty"),
            "{}",
            failures[0].message
        );

        // The shapes a real tool loop sends stay accepted: a filled text part,
        // the string spelling, and an absent `content` — the last two are how a
        // tool that returned nothing says so, which is legal on this surface.
        for content in [
            json!([{ "type": "text", "text": "the fact" }]),
            json!("the fact"),
            json!(""),
        ] {
            assert!(
                parse(&headers(), Some(&loop_request(content.clone())))
                    .failures
                    .is_empty(),
                "a `tool_result` content of {content} is legal"
            );
        }
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
        // Every live response carries the two cache counters whether or not the
        // request asked for caching, and WIRE-NOTES §9's rule is to send the
        // always-present members a strict client might read.
        assert_eq!(response.body["usage"]["cache_creation_input_tokens"], 0);
        assert_eq!(response.body["usage"]["cache_read_input_tokens"], 0);

        // A scripted `usage` overrides the estimate and leaves the counters
        // where they are: a test pinning token accounting is not a test asking
        // for a different response shape.
        let Outcome::Reply(reply) = &outcome else {
            panic!("a structured outcome is a reply");
        };
        let scripted = Outcome::Reply(Reply {
            usage: Some(crate::control::Usage::new(11, 22)),
            ..reply.clone()
        });
        let Answer::Respond(response) =
            render(3, &request, parsed.structured_output.as_ref(), &scripted)
        else {
            panic!("a reply is a response");
        };
        assert_eq!(
            response.body["usage"],
            json!({
                "input_tokens": 11,
                "output_tokens": 22,
                "cache_creation_input_tokens": 0,
                "cache_read_input_tokens": 0,
            })
        );
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

    /// The inverse refusal: a `text` reply to a request that pinned tool use is
    /// an answer the Messages API cannot send, so it is a script bug too — and
    /// a codegen PR that scripted one would be testing its structured-output
    /// parser against an input no provider will ever produce.
    #[test]
    fn a_text_reply_to_a_pinned_tool_use_is_a_script_mismatch() {
        let pinned = |choice: Value| {
            messages(json!({
                "messages": [{ "role": "user", "content": "hi" }],
                "tools": [{ "name": "extract", "input_schema": { "type": "object" } }],
                "tool_choice": choice,
            }))
        };

        for choice in [
            json!({ "type": "tool", "name": "extract" }),
            json!({ "type": "any" }),
        ] {
            let request = pinned(choice.clone());
            let parsed = parse(&headers(), Some(&request));
            assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
            let Answer::Respond(response) = render(
                1,
                &request,
                parsed.structured_output.as_ref(),
                &Outcome::text("I am not going to call the tool"),
            ) else {
                panic!("a mismatch is a response");
            };
            assert_eq!(response.status, HARNESS_STATUS, "{choice}");
            assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
        }

        // The positive half: `auto` and `none` leave prose on the table, and a
        // request that pins nothing at all is answered as it always was.
        for choice in [json!({ "type": "auto" }), json!({ "type": "none" })] {
            let request = pinned(choice.clone());
            let Answer::Respond(response) =
                render(1, &request, None, &Outcome::text("just talking"))
            else {
                panic!("a reply is a response");
            };
            assert_eq!(response.status, 200, "{choice}");
            assert_eq!(response.body["content"][0]["text"], "just talking");
            assert_eq!(response.body["stop_reason"], "end_turn");
        }
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

    /// And so is a tool call the request *forbade*, or a tool reply with no call
    /// in it at all — the two answers the Messages API cannot send in this
    /// direction, mirroring the prose refusals above.
    #[test]
    fn a_tool_reply_the_request_forbids_or_empties_is_a_script_mismatch() {
        let with = |choice: Value| {
            messages(json!({
                "messages": [{ "role": "user", "content": "hi" }],
                "tools": [{ "name": "lookup", "input_schema": { "type": "object" } }],
                "tool_choice": choice,
            }))
        };
        let call = || Outcome::tool_calls(vec![crate::control::ToolCall::new("lookup", json!({}))]);

        // `tool_choice: none` says the model must not call a tool.
        let request = with(json!({ "type": "none" }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);
        let Answer::Respond(response) = render(1, &request, None, &call()) else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);

        // An empty call list renders `content: []` with a `tool_use` stop
        // reason, which is not a shape the API has.
        for request in [
            with(json!({ "type": "auto" })),
            messages(json!({ "messages": [{ "role": "user", "content": "hi" }] })),
        ] {
            let Answer::Respond(response) = render(1, &request, None, &Outcome::tool_calls(vec![]))
            else {
                panic!("a mismatch is a response");
            };
            assert_eq!(response.status, HARNESS_STATUS);
            assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
        }

        // The positive half: `auto` leaves a call legal, and it is rendered.
        let request = with(json!({ "type": "auto" }));
        let Answer::Respond(response) = render(1, &request, None, &call()) else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["content"][0]["type"], "tool_use");
        assert_eq!(response.body["stop_reason"], "tool_use");
    }

    /// A scripted call to a tool the request offered but did **not** pin is a
    /// script bug too: a pinned `tool_choice` promises a call to *that* tool, so
    /// a call to a sibling is a shape the Messages API cannot send — the hole
    /// between "the tool is on offer" and "the tool is the one that was forced".
    #[test]
    fn a_tool_call_beside_the_pinned_one_is_a_script_mismatch() {
        let request = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [
                { "name": "lookup", "input_schema": { "type": "object" } },
                { "name": "reviewer_output", "input_schema": { "type": "object" } },
            ],
            "tool_choice": { "type": "tool", "name": "reviewer_output" },
        }));
        let parsed = parse(&headers(), Some(&request));
        assert!(parsed.failures.is_empty(), "{:?}", parsed.failures);

        let outcome = Outcome::tool_calls(vec![crate::control::ToolCall::new("lookup", json!({}))]);
        let Answer::Respond(response) =
            render(1, &request, parsed.structured_output.as_ref(), &outcome)
        else {
            panic!("a mismatch is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
        let message = response.body["error"]["message"].as_str().unwrap();
        assert!(message.contains("lookup"), "{message}");
        assert!(message.contains("reviewer_output"), "{message}");

        // The positive half, twice over: a call to the tool that *was* pinned is
        // the answer the API sends, and with nothing pinned either tool is fair
        // game — the rule narrows exactly one shape and nothing else.
        let pinned = Outcome::tool_calls(vec![crate::control::ToolCall::new(
            "reviewer_output",
            json!({ "verdict": "approve" }),
        )]);
        let Answer::Respond(response) =
            render(1, &request, parsed.structured_output.as_ref(), &pinned)
        else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["content"][0]["name"], "reviewer_output");

        let mut unpinned = request.clone();
        unpinned["tool_choice"] = json!({ "type": "auto" });
        let Answer::Respond(response) = render(1, &unpinned, None, &outcome) else {
            panic!("a reply is a response");
        };
        assert_eq!(response.status, 200);
        assert_eq!(response.body["content"][0]["name"], "lookup");
    }

    /// A scripted `stop_reason` is held to the closed set, to the content it
    /// accompanies, and — for `stop_sequence` — to the request that could have
    /// matched one.
    ///
    /// It is the field a compiled agent's tool loop branches on, so a script
    /// that could name anything could stage a turn the API never ends that way,
    /// and a codegen PR would watch its loop take the branch it wanted and pass
    /// a criterion no live call would have let it reach.
    #[test]
    fn a_scripted_stop_reason_must_be_one_the_api_sends() {
        let plain = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let with_tools = messages(json!({
            "messages": [{ "role": "user", "content": "hi" }],
            "tools": [{ "name": "lookup", "input_schema": { "type": "object" } }],
        }));
        let ended = |request: &Value, outcome: Outcome, reason: &str| {
            let Outcome::Reply(reply) = outcome else {
                panic!("a reply is a reply");
            };
            let scripted = Outcome::Reply(Reply {
                stop_reason: Some(reason.to_string()),
                ..reply
            });
            let Answer::Respond(response) = render(1, request, None, &scripted) else {
                panic!("an override answers");
            };
            response
        };
        let call = || Outcome::tool_calls(vec![crate::control::ToolCall::new("lookup", json!({}))]);

        // Outside the closed set: an invented value, and Chat Completions'
        // vocabulary reached for out of habit.
        for reason in ["banana", "stop", "tool_calls", "length"] {
            let response = ended(&plain, Outcome::text("just prose"), reason);
            assert_eq!(response.status, HARNESS_STATUS, "{reason}");
            assert_eq!(
                response.headers[crate::control::HARNESS_HEADER],
                MISMATCH,
                "{reason}"
            );
            let message = response.body["error"]["message"].as_str().unwrap();
            assert!(message.contains(reason), "{message}");
        }

        // In the set, but contradicting the content: `tool_use` names a block
        // prose does not carry, and `end_turn` says the model stopped talking
        // when a `tool_use` block ended the turn.
        let response = ended(&plain, Outcome::text("just prose"), "tool_use");
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
        let response = ended(&with_tools, call(), "end_turn");
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);

        // `stop_sequence` names the sequence that ended the turn, and the API
        // only matches ones the request supplied — so it is refused without
        // them, and served *with* the one it names when they are there.
        let response = ended(&plain, Outcome::text("cut"), "stop_sequence");
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], MISMATCH);
        let mut stopped = plain.clone();
        stopped["stop_sequences"] = json!(["<END>", "<HALT>"]);
        assert!(
            parse(&headers(), Some(&stopped)).failures.is_empty(),
            "the request itself is legal"
        );
        let response = ended(&stopped, Outcome::text("cut"), "stop_sequence");
        assert_eq!(response.status, 200);
        assert_eq!(response.body["stop_reason"], "stop_sequence");
        assert_eq!(response.body["stop_sequence"], "<END>");

        // The positive half: truncation, a paused turn and a refusal each cut
        // either content, and leave `stop_sequence` where it always was.
        for (request, outcome) in [(&plain, Outcome::text("cut off")), (&with_tools, call())] {
            for reason in ["max_tokens", "pause_turn", "refusal"] {
                let response = ended(request, outcome.clone(), reason);
                assert_eq!(response.status, 200, "{reason}");
                assert_eq!(response.body["stop_reason"], reason);
                assert!(response.body["stop_sequence"].is_null(), "{reason}");
            }
        }
        let Answer::Respond(response) = render(1, &plain, None, &Outcome::text("hi")) else {
            panic!("a reply is a response");
        };
        assert_eq!(response.body["stop_reason"], "end_turn");
        assert!(response.body["stop_sequence"].is_null());
    }

    /// Every answer carries the request id, **errors included** — the header on
    /// each, and the `request_id` member the error envelope has.
    #[test]
    fn every_answer_carries_its_request_id() {
        let request = messages(json!({ "messages": [{ "role": "user", "content": "hi" }] }));
        let carried = |answer: Answer| match answer {
            Answer::Respond(response) => (
                response.headers.get("request-id").cloned(),
                response.body.get("request_id").cloned(),
            ),
            Answer::Close(_) => panic!("this outcome answers"),
        };

        // A 200, for the contrast: it has carried the header all along.
        assert_eq!(
            carried(render(3, &request, None, &Outcome::text("ok"))).0,
            Some("req_mock_00000003".to_string())
        );

        for answer in [
            render(3, &request, None, &Outcome::rate_limit()),
            render(3, &request, None, &Outcome::server_error()),
            render(3, &request, None, &Outcome::structured(json!({ "a": 1 }))),
            rejected(
                3,
                &[ValidationFailure::new("model", "model: Field required")],
            ),
            unscripted(3, "model.fast", "the queue is empty"),
        ] {
            assert_eq!(
                carried(answer),
                (
                    Some("req_mock_00000003".to_string()),
                    Some(json!("req_mock_00000003"))
                )
            );
        }
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
        let Answer::Respond(response) = rejected(4, &failures) else {
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
        let Answer::Respond(response) = unscripted(9, "model.fast", "the queue is empty") else {
            panic!("a refusal is a response");
        };
        assert_eq!(response.status, HARNESS_STATUS);
        assert_eq!(response.headers[crate::control::HARNESS_HEADER], UNSCRIPTED);
        let message = response.body["error"]["message"].as_str().unwrap();
        assert!(message.contains("model.fast"), "{message}");
        assert!(message.contains("the queue is empty"), "{message}");
    }
}
