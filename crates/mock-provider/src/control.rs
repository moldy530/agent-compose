//! The control plane: what a test scripts, what the server records, and the
//! store that holds both.
//!
//! Everything a run does is decided here and nowhere else. A request arrives,
//! [`Store::serve`] validates it (the surface modules decide *how*), takes the
//! first outcome in the model's queue that matches it, and appends a
//! [`RecordedRequest`]. There is one lock and one arrival order, so the
//! transcript a test reads back is the order the server acted in.
//!
//! # Determinism
//!
//! The harness is the acceptance rig for every M1 PR, so a scripted run has to
//! answer the same way twice:
//!
//! * **No randomness.** Every generated id is derived from the request's own
//!   arrival sequence — `msg_mock_00000003` is the third request's answer — so
//!   a transcript reads back as a fixed document and points at what produced it.
//! * **No wall clock.** Response timestamps are the frozen [`CREATED`] constant.
//!   The one clock this server reads is a *scripted* [`Delay`], because two of
//!   the things the harness must be able to stage are temporal: a provider that
//!   times out (PRD 5.9 `route_on: [timeout]`) and a fan-out whose instances
//!   complete out of source order (PRD 5.6's index-tagged reducers).
//! * **Matched FIFO.** Queues are per model id and served front to back. An
//!   outcome may carry a [`Match`] narrowing which request it answers, and the
//!   server takes the first entry that matches — plain FIFO is the case where
//!   nothing narrows. The narrowing exists because concurrent `map` instances
//!   share one model id and reach the server in scheduler order: without it a
//!   fan-out test would be scripting against arrival order, which is exactly the
//!   nondeterminism the harness is supposed to remove.
//!
//! # Validation comes first
//!
//! A request that fails validation is answered with the provider's own error and
//! **consumes no outcome**. A queue is a test's statement about the calls a graph
//! makes; letting one malformed call eat the answer meant for the next one would
//! turn a single codegen bug into a cascade of unrelated failures.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Mutex;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// The `created` timestamp every OpenAI-surface response carries.
///
/// Frozen, because a golden transcript that embedded the wall clock would differ
/// from itself on every run. 2023-11-14T22:13:20Z, chosen only for being round.
pub const CREATED: u64 = 1_700_000_000;

/// The header a response carries when the *harness* refused the request rather
/// than a provider having failed.
///
/// A test that sees it knows the run never reached a scripted outcome: the
/// request was malformed, unscripted, or asked for something the script could
/// not render. Generated code classifies provider failures by status (5.9), so
/// these answers deliberately carry a status no failover condition claims.
pub const HARNESS_HEADER: &str = "x-mock-provider-error";

/// The status every harness refusal carries.
///
/// Deliberately not a provider status. PRD 5.9 routes on *infrastructure*
/// conditions — 429, 5xx, no answer — so a harness refusal dressed as one of
/// those would be silently failed over instead of failing the test that has a
/// bug in it. 409 belongs to no failover condition, and [`HARNESS_HEADER`] names
/// which refusal it is.
pub const HARNESS_STATUS: u16 = 409;

/// [`HARNESS_HEADER`] on a request the harness refused because it was malformed.
pub const REFUSED_INVALID: &str = "invalid-request";
/// [`HARNESS_HEADER`] on a request no scripted outcome answered.
pub const REFUSED_UNSCRIPTED: &str = "unscripted-request";
/// [`HARNESS_HEADER`] on a request whose scripted outcome could not be rendered
/// into what the request asked for.
pub const REFUSED_MISMATCH: &str = "script-mismatch";

/// Which provider surface a request arrived on.
///
/// The six provider kinds of grammar 12.1 funnel into two HTTP protocols:
/// `anthropic` speaks the Messages API, and `openai` / `openai_compatible` /
/// `azure_openai` all speak Chat Completions, Azure differing by route and
/// auth header rather than by body. `bedrock` and `vertex` are reached through
/// cloud SDKs and have no HTTP surface this server can stand in for — they are
/// out of scope for v0, and `WIRE-NOTES.md` says so.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// `POST /v1/messages`.
    Anthropic,
    /// `POST /v1/chat/completions`.
    OpenAi,
    /// `POST /openai/deployments/<deployment>/chat/completions` and the newer
    /// `POST /openai/v1/chat/completions`.
    AzureOpenAi,
}

impl Surface {
    /// The name this surface is spelled with in a control-plane document.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::AzureOpenAi => "azure_openai",
        }
    }
}

/// One scripted outcome, and the requests it is willing to answer.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Script {
    /// The model id whose queue this outcome joins. On the Azure surface a
    /// request that names no model in its body is keyed by its deployment.
    pub model: String,
    /// What the provider does when this outcome is served.
    pub outcome: Outcome,
    /// How many requests this outcome answers before it leaves the queue.
    #[serde(default = "once")]
    pub times: u32,
    /// Which requests it is willing to answer.
    #[serde(default, rename = "match")]
    pub matcher: Match,
}

fn once() -> u32 {
    1
}

impl Script {
    /// A one-shot outcome for `model`, matching any request.
    #[must_use]
    pub fn new(model: impl Into<String>, outcome: Outcome) -> Self {
        Self {
            model: model.into(),
            outcome,
            times: 1,
            matcher: Match::default(),
        }
    }

    /// Answer this many requests rather than one.
    #[must_use]
    pub fn times(mut self, times: u32) -> Self {
        self.times = times;
        self
    }

    /// Answer only requests whose body contains `needle`.
    #[must_use]
    pub fn matching(mut self, needle: impl Into<String>) -> Self {
        self.matcher = Match::BodyContains {
            body_contains: needle.into(),
        };
        self
    }
}

/// Which requests an outcome will answer.
///
/// Deliberately crude: one substring test over the request body, serialized
/// canonically. A richer predicate language would be a second expression
/// language in a project that already has one (CEL, PRD 5.5), and the only
/// question a fan-out test needs to ask — "the call carrying *this* item" — is
/// answered by the item's own text appearing in the user turn.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// Any request for the model. The default, and plain FIFO.
    #[default]
    Any,
    /// Only requests whose canonically serialized body contains this substring.
    BodyContains { body_contains: String },
}

impl Match {
    /// Whether this matcher accepts `body`, already canonically serialized.
    fn accepts(&self, body: &str) -> bool {
        match self {
            Self::Any => true,
            Self::BodyContains { body_contains } => body.contains(body_contains.as_str()),
        }
    }
}

/// What the provider does when an outcome is served.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Outcome {
    /// The model answers. Rendered into the surface's own wire shape.
    Reply(Reply),
    /// The provider refuses, in one of the shapes PRD 5.9's `route_on`
    /// conditions are classified from.
    Failure(Failure),
    /// A body served verbatim, at a status of the script's choosing.
    ///
    /// The escape hatch for what the two shapes above deliberately cannot
    /// express: a response the *generated code* must reject — a missing
    /// `content`, a tool call whose arguments are not JSON, a status no
    /// provider sends. Nothing about it is validated on the way out.
    Raw(RawOutcome),
}

impl Outcome {
    /// Structured output: rendered through whichever mechanism the request asked
    /// for (a forced tool on the Anthropic surface, `response_format` or a
    /// forced function on the OpenAI one).
    #[must_use]
    pub fn structured(value: Value) -> Self {
        Self::Reply(Reply::new(ReplyBody::Structured(value)))
    }

    /// Plain assistant text.
    #[must_use]
    pub fn text(text: impl Into<String>) -> Self {
        Self::Reply(Reply::new(ReplyBody::Text(text.into())))
    }

    /// Tool calls, which is how an agent's tool loop is driven: the graph is
    /// expected to run the tools and call back with their results.
    #[must_use]
    pub fn tool_calls(calls: Vec<ToolCall>) -> Self {
        Self::Reply(Reply::new(ReplyBody::Tools { calls, text: None }))
    }

    /// HTTP 429 with the provider's rate-limit body and a `retry-after` header.
    #[must_use]
    pub fn rate_limit() -> Self {
        Self::Failure(Failure::RateLimit {
            retry_after_seconds: Some(1),
        })
    }

    /// The provider's overload status (529 on Anthropic, 503 on OpenAI).
    #[must_use]
    pub fn overloaded() -> Self {
        Self::Failure(Failure::Overloaded)
    }

    /// HTTP 500.
    #[must_use]
    pub fn server_error() -> Self {
        Self::Failure(Failure::ServerError)
    }

    /// Accept the request, answer nothing, and close the connection after
    /// `delay`. See [`Failure::Timeout`].
    #[must_use]
    pub fn timeout(delay: Duration) -> Self {
        Self::Failure(Failure::Timeout {
            delay: Delay::from(delay),
        })
    }

    /// A verbatim body at a chosen status.
    #[must_use]
    pub fn raw(status: u16, body: Value) -> Self {
        Self::Raw(RawOutcome {
            status,
            body,
            headers: BTreeMap::new(),
            delay: Delay::none(),
        })
    }

    /// Answer only after `delay`.
    ///
    /// The scheduling knob a fan-out test needs: with completion order forced,
    /// "appended results are reordered by source-item index" (PRD 5.6) becomes
    /// an assertion rather than a coin flip.
    #[must_use]
    pub fn after(self, delay: Duration) -> Self {
        let delay = Delay::from(delay);
        match self {
            Self::Reply(reply) => Self::Reply(Reply { delay, ..reply }),
            Self::Raw(raw) => Self::Raw(RawOutcome { delay, ..raw }),
            Self::Failure(failure) => Self::Failure(match failure {
                Failure::Timeout { .. } => Failure::Timeout { delay },
                other => other,
            }),
        }
    }

    /// How this outcome is named in a recorded request's `served` field.
    fn served(&self) -> &'static str {
        match self {
            Self::Reply(reply) => match reply.body {
                ReplyBody::Structured(_) => "reply.structured",
                ReplyBody::Text(_) => "reply.text",
                ReplyBody::Tools { .. } => "reply.tools",
            },
            Self::Failure(failure) => failure.served(),
            Self::Raw(_) => "raw",
        }
    }
}

/// A model's answer, before it is rendered into a surface's wire shape.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Reply {
    /// What the model produced.
    pub body: ReplyBody,
    /// Token counts. Defaults to a deterministic estimate over the rendered
    /// request and response (see `usage`), which is what lets a test assert that
    /// accounting reaches the trace at all without pinning a real tokenizer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Overrides the stop reason the body implies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// How long to wait before answering.
    #[serde(default, skip_serializing_if = "Delay::is_none")]
    pub delay: Delay,
}

impl Reply {
    fn new(body: ReplyBody) -> Self {
        Self {
            body,
            usage: None,
            stop_reason: None,
            delay: Delay::none(),
        }
    }
}

/// What the model produced.
///
/// Externally tagged, so a scripted body reads as `{"text": "…"}` and
/// `{"structured": {…}}` — the spelling a TypeScript harness will write by hand.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ReplyBody {
    /// A structured-output object. The request must have asked for structured
    /// output; a script that says otherwise is a harness error rather than a
    /// reply, and is answered as one.
    Structured(Value),
    /// Assistant text.
    Text(String),
    /// Tool calls, optionally preceded by text.
    Tools {
        calls: Vec<ToolCall>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        text: Option<String>,
    },
}

/// One tool call in a scripted reply.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ToolCall {
    /// The tool's name, as the request offered it.
    pub name: String,
    /// The arguments, as an object. The surface decides how it travels: an
    /// object under `input` on the Anthropic surface, a JSON *string* under
    /// `function.arguments` on the OpenAI one.
    pub input: Value,
    /// Overrides the derived call id, for a test that wants to pin the
    /// `tool_result` correlation itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl ToolCall {
    /// A call to `name` with `input`, its id derived from the request.
    #[must_use]
    pub fn new(name: impl Into<String>, input: Value) -> Self {
        Self {
            name: name.into(),
            input,
            id: None,
        }
    }
}

/// Token counts, as both surfaces report them.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
}

/// How a provider refuses.
///
/// The variants are exactly PRD 5.9's failover conditions plus the
/// `server_error` grammar 12.2 adds, because what generated code must classify
/// is what the harness must be able to stage.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum Failure {
    /// 429. Carries `retry-after` when the script asks for it.
    RateLimit {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        retry_after_seconds: Option<u32>,
    },
    /// The provider's overload status: 529 on Anthropic, 503 on OpenAI.
    Overloaded,
    /// 500.
    ServerError,
    /// No answer at all: the connection is closed after `delay`.
    ///
    /// A client with a request timeout below the delay sees the timeout it is
    /// being tested for; a client with none sees a closed connection instead of
    /// hanging until the suite is killed, which is why the delay ends in a close
    /// rather than in a hang.
    Timeout { delay: Delay },
}

impl Failure {
    fn served(&self) -> &'static str {
        match self {
            Self::RateLimit { .. } => "failure.rate_limit",
            Self::Overloaded => "failure.overloaded",
            Self::ServerError => "failure.server_error",
            Self::Timeout { .. } => "failure.timeout",
        }
    }
}

/// A body served verbatim.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RawOutcome {
    pub status: u16,
    pub body: Value,
    /// Extra response headers, on top of `content-type`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Delay::is_none")]
    pub delay: Delay,
}

/// A scripted wait, in milliseconds.
///
/// The one clock this server reads. It is a *scripted* value, so a run that
/// declares no delay reads no clock at all and two runs of it are the same run.
#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize)]
#[serde(transparent)]
pub struct Delay {
    milliseconds: u64,
}

impl Delay {
    /// No wait.
    #[must_use]
    pub fn none() -> Self {
        Self { milliseconds: 0 }
    }

    /// Whether this is no wait at all.
    #[must_use]
    pub fn is_none(&self) -> bool {
        self.milliseconds == 0
    }

    /// The wait, as a duration.
    #[must_use]
    pub fn duration(self) -> Duration {
        Duration::from_millis(self.milliseconds)
    }
}

impl From<Duration> for Delay {
    fn from(duration: Duration) -> Self {
        Self {
            milliseconds: u64::try_from(duration.as_millis()).unwrap_or(u64::MAX),
        }
    }
}

/// One thing wrong with a request.
///
/// `pointer` is the harness's uniform address — a dotted path into the request
/// body, `messages.1.content.0.type` — so a test can assert on where the mistake
/// is without knowing which surface's dialect the `message` is written in.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ValidationFailure {
    pub pointer: String,
    pub message: String,
}

impl ValidationFailure {
    pub(crate) fn new(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            pointer: pointer.into(),
            message: message.into(),
        }
    }
}

/// How a request came out of validation.
///
/// Internally tagged, and flattened into the recorded request, so a transcript
/// reads `"verdict": "invalid", "failures": [ … ]` rather than nesting one
/// object inside another for a reader that only wants to know which it was.
#[derive(Clone, Debug, Serialize)]
#[serde(tag = "verdict", rename_all = "snake_case")]
pub enum Verdict {
    /// Nothing was wrong with it.
    Valid,
    /// These things were.
    Invalid { failures: Vec<ValidationFailure> },
}

impl Verdict {
    /// The failures, empty when the request was valid.
    #[must_use]
    pub fn failures(&self) -> &[ValidationFailure] {
        match self {
            Self::Valid => &[],
            Self::Invalid { failures } => failures,
        }
    }

    /// Whether the request passed.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        matches!(self, Self::Valid)
    }
}

/// How a request asked for structured output.
///
/// Projected out of the body at record time because it is the load-bearing
/// surface of PRD 5.2 — every agent declares an output schema, and codegen has
/// to ask the provider for it in the provider's own way. A test asserts on this
/// rather than re-walking the request body it already knows the shape of.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredOutput {
    /// Anthropic: a tool the request forces the model to call, whose
    /// `input_schema` is the agent's output schema.
    ForcedTool { name: String, schema: Value },
    /// OpenAI: `response_format: { type: json_schema, json_schema: {…} }`.
    JsonSchema {
        name: String,
        schema: Value,
        strict: bool,
    },
    /// OpenAI: a function the request forces the model to call.
    ForcedFunction { name: String, schema: Value },
}

impl StructuredOutput {
    /// The schema the request asked the model to fill.
    #[must_use]
    pub fn schema(&self) -> &Value {
        match self {
            Self::ForcedTool { schema, .. }
            | Self::JsonSchema { schema, .. }
            | Self::ForcedFunction { schema, .. } => schema,
        }
    }

    /// The name the request gave it.
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Self::ForcedTool { name, .. }
            | Self::JsonSchema { name, .. }
            | Self::ForcedFunction { name, .. } => name,
        }
    }
}

/// One request, as the server saw it.
#[derive(Clone, Debug, Serialize)]
pub struct RecordedRequest {
    /// 1-based arrival order. Also the seed of every id in the answer, so a
    /// transcript's `msg_mock_00000003` points back at request 3.
    pub sequence: u64,
    /// Which provider surface it arrived on.
    pub surface: Surface,
    pub method: String,
    pub path: String,
    /// The query string, without the `?`.
    pub query: String,
    /// Every request header, lowercased, in sorted order.
    pub headers: BTreeMap<String, String>,
    /// The model whose queue the request drew from.
    pub model: String,
    /// The body, parsed. `None` when it was not JSON at all.
    pub body: Option<Value>,
    /// The body as it arrived, kept because a body that failed to parse has
    /// nothing else to show and a byte-level assertion needs the original.
    pub body_text: String,
    /// What was wrong with it, if anything.
    #[serde(flatten)]
    pub verdict: Verdict,
    /// The names of the tools the request offered, in request order. Includes
    /// the tools codegen synthesizes from an agent's attached stores (11.5), so
    /// `agent_access` narrowing is observable here.
    pub tools: Vec<String>,
    /// How the request asked for structured output, if it did.
    pub structured_output: Option<StructuredOutput>,
    /// What the server did about it.
    pub served: String,
}

impl RecordedRequest {
    /// Whether the request passed validation.
    #[must_use]
    pub fn is_valid(&self) -> bool {
        self.verdict.is_valid()
    }

    /// What was wrong with it.
    #[must_use]
    pub fn failures(&self) -> &[ValidationFailure] {
        self.verdict.failures()
    }

    /// The parsed body, for a test that has already asserted it is JSON.
    ///
    /// # Panics
    ///
    /// If the body was not JSON.
    #[must_use]
    pub fn body(&self) -> &Value {
        self.body
            .as_ref()
            .expect("the recorded request carries a JSON body")
    }
}

/// A count of everything the store is holding.
#[derive(Clone, Debug, Serialize)]
pub struct Snapshot {
    /// Queue depth per model, counting repetitions: an outcome with
    /// `times: 3` that has answered once counts 2.
    pub queues: BTreeMap<String, u32>,
    /// How many requests have been recorded.
    pub requests: usize,
    /// How many of them failed validation.
    pub invalid: usize,
    /// How many of them found no scripted outcome.
    pub unscripted: usize,
}

impl Snapshot {
    /// Whether every scripted outcome was consumed and nothing was refused —
    /// the shape a finished acceptance run should have.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.queues.is_empty() && self.invalid == 0 && self.unscripted == 0
    }
}

/// What the server should do about a request, once the store has decided.
#[derive(Clone, Debug)]
pub enum Decision {
    /// Serve this outcome.
    Serve(Outcome),
    /// Refuse: the request was malformed. The surface renders the failures into
    /// its own error shape.
    Rejected(Vec<ValidationFailure>),
    /// Refuse: the model's queue had nothing for this request.
    Unscripted { model: String, reason: String },
}

/// Everything the server holds, behind one lock.
#[derive(Debug, Default)]
pub struct Store {
    state: Mutex<State>,
}

#[derive(Debug, Default)]
struct State {
    queues: BTreeMap<String, VecDeque<Script>>,
    requests: Vec<RecordedRequest>,
}

/// What a surface module hands the store about a request it has parsed.
pub(crate) struct Incoming {
    pub(crate) surface: Surface,
    pub(crate) method: String,
    pub(crate) path: String,
    pub(crate) query: String,
    pub(crate) headers: BTreeMap<String, String>,
    pub(crate) model: String,
    pub(crate) body: Option<Value>,
    pub(crate) body_text: String,
    pub(crate) failures: Vec<ValidationFailure>,
    pub(crate) tools: Vec<String>,
    pub(crate) structured_output: Option<StructuredOutput>,
}

impl Store {
    /// A store holding nothing.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an outcome to the back of its model's queue.
    pub fn enqueue(&self, script: Script) {
        let mut state = self.lock();
        state
            .queues
            .entry(script.model.clone())
            .or_default()
            .push_back(script);
    }

    /// Every request the server has seen, in arrival order.
    #[must_use]
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.lock().requests.clone()
    }

    /// Forget every request and discard every queued outcome.
    ///
    /// Returns what was discarded, because "nothing was left over" is a thing an
    /// acceptance test asserts and a reset that answered nothing would make the
    /// assertion unavailable after the fact.
    pub fn reset(&self) -> Snapshot {
        let mut state = self.lock();
        let snapshot = Self::snapshot_of(&state);
        state.queues.clear();
        state.requests.clear();
        snapshot
    }

    /// A count of everything the store is holding.
    #[must_use]
    pub fn snapshot(&self) -> Snapshot {
        Self::snapshot_of(&self.lock())
    }

    fn snapshot_of(state: &State) -> Snapshot {
        Snapshot {
            queues: state
                .queues
                .iter()
                .filter(|(_, queue)| !queue.is_empty())
                .map(|(model, queue)| {
                    (model.clone(), queue.iter().map(|script| script.times).sum())
                })
                .collect(),
            requests: state.requests.len(),
            invalid: state
                .requests
                .iter()
                .filter(|request| !request.is_valid())
                .count(),
            unscripted: state
                .requests
                .iter()
                .filter(|request| request.served == "unscripted")
                .count(),
        }
    }

    /// Record a request and decide what to answer it with.
    ///
    /// The whole decision happens under one lock: the arrival sequence, the
    /// queue take, and the transcript entry are one atomic step, so two
    /// concurrent `map` instances cannot interleave into a transcript that shows
    /// an answer before the request that drew it.
    pub(crate) fn serve(&self, incoming: Incoming) -> (Decision, u64) {
        let mut state = self.lock();
        let sequence = state.requests.len() as u64 + 1;

        let decision = if incoming.failures.is_empty() {
            Self::take(&mut state.queues, &incoming.model, &incoming.body_text)
        } else {
            Decision::Rejected(incoming.failures.clone())
        };

        let verdict = if incoming.failures.is_empty() {
            Verdict::Valid
        } else {
            Verdict::Invalid {
                failures: incoming.failures,
            }
        };

        let served = match &decision {
            Decision::Serve(outcome) => outcome.served().to_string(),
            Decision::Rejected(_) => "rejected".to_string(),
            Decision::Unscripted { .. } => "unscripted".to_string(),
        };

        state.requests.push(RecordedRequest {
            sequence,
            surface: incoming.surface,
            method: incoming.method,
            path: incoming.path,
            query: incoming.query,
            headers: incoming.headers,
            model: incoming.model,
            body: incoming.body,
            body_text: incoming.body_text,
            verdict,
            tools: incoming.tools,
            structured_output: incoming.structured_output,
            served,
        });

        (decision, sequence)
    }

    /// Take the first outcome in `model`'s queue that matches this body.
    fn take(queues: &mut BTreeMap<String, VecDeque<Script>>, model: &str, body: &str) -> Decision {
        let Some(queue) = queues.get_mut(model) else {
            return Decision::Unscripted {
                model: model.to_string(),
                reason: format!("no outcome has ever been enqueued for model `{model}`"),
            };
        };
        let found = queue.iter().position(|script| script.matcher.accepts(body));
        let Some(at) = found else {
            let reason = if queue.is_empty() {
                format!("model `{model}`'s queue is empty")
            } else {
                format!(
                    "none of the {} outcome(s) left in model `{model}`'s queue matches this request",
                    queue.len()
                )
            };
            return Decision::Unscripted {
                model: model.to_string(),
                reason,
            };
        };
        let script = &mut queue[at];
        let outcome = script.outcome.clone();
        script.times -= 1;
        if script.times == 0 {
            queue.remove(at);
        }
        // The emptied queue stays: "this model was scripted and is drained" and
        // "this model was never scripted" are different mistakes, and the
        // refusal a test reads should say which one it is. `Snapshot` filters
        // the empties back out, so nothing else sees the difference.
        Decision::Serve(outcome)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        // A panic inside a handler must not turn every later request into a
        // panic of its own: the transcript is the evidence a test reads, and it
        // is more useful poisoned than unreachable.
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

/// The deterministic token estimate a reply falls back to.
///
/// Four characters to the token over the serialized text, which is the estimate
/// every provider's own documentation offers as a rule of thumb. It is not a
/// tokenizer and does not pretend to be one: what it buys is a number that moves
/// when the prompt moves, so a test asserting "usage reached the trace" is
/// asserting about accounting rather than about a constant.
pub(crate) fn estimate(text: &str) -> u64 {
    text.len().div_ceil(4) as u64
}

/// The canonical serialization of a body, used for matching and for the estimate.
pub(crate) fn canonical(body: &Value) -> String {
    serde_json::to_string(body).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn store_with(scripts: Vec<Script>) -> Store {
        let store = Store::new();
        for script in scripts {
            store.enqueue(script);
        }
        store
    }

    fn take(store: &Store, model: &str, body: &str) -> Decision {
        let mut state = store.lock();
        Store::take(&mut state.queues, model, body)
    }

    /// The queue is FIFO per model, and one model's queue is not the other's.
    #[test]
    fn outcomes_are_served_front_to_back_per_model() {
        let store = store_with(vec![
            Script::new("fast", Outcome::text("first")),
            Script::new("fast", Outcome::text("second")),
            Script::new("smart", Outcome::text("other")),
        ]);

        let served: Vec<String> = (0..2)
            .map(|_| match take(&store, "fast", "{}") {
                Decision::Serve(Outcome::Reply(Reply {
                    body: ReplyBody::Text(text),
                    ..
                })) => text,
                other => panic!("expected a text reply, got {other:?}"),
            })
            .collect();
        assert_eq!(served, ["first", "second"]);

        assert!(
            matches!(take(&store, "fast", "{}"), Decision::Unscripted { .. }),
            "the queue is drained"
        );
        assert!(
            matches!(take(&store, "smart", "{}"), Decision::Serve(_)),
            "the other model's queue is untouched"
        );
    }

    /// `times: N` answers N requests from one entry, and the entry leaves the
    /// queue on the last one.
    #[test]
    fn a_repeated_outcome_answers_that_many_requests() {
        let store = store_with(vec![Script::new("fast", Outcome::text("again")).times(3)]);
        for _ in 0..3 {
            assert!(matches!(take(&store, "fast", "{}"), Decision::Serve(_)));
        }
        assert!(matches!(
            take(&store, "fast", "{}"),
            Decision::Unscripted { .. }
        ));
        assert!(
            store.snapshot().queues.is_empty(),
            "a spent entry leaves the queue"
        );
    }

    /// A matcher takes the first *matching* entry rather than the first entry,
    /// which is what makes a concurrent fan-out scriptable: the instances share
    /// one model id and arrive in scheduler order.
    #[test]
    fn a_matcher_selects_the_first_entry_that_accepts_the_request() {
        let store = store_with(vec![
            Script::new("fast", Outcome::text("for-alpha")).matching("alpha"),
            Script::new("fast", Outcome::text("for-beta")).matching("beta"),
            Script::new("fast", Outcome::text("for-anything")),
        ]);

        let served = |body: &str| match take(&store, "fast", body) {
            Decision::Serve(Outcome::Reply(Reply {
                body: ReplyBody::Text(text),
                ..
            })) => text,
            other => panic!("expected a text reply, got {other:?}"),
        };

        assert_eq!(served("{\"m\":\"beta\"}"), "for-beta");
        assert_eq!(served("{\"m\":\"alpha\"}"), "for-alpha");
        assert_eq!(
            served("{\"m\":\"gamma\"}"),
            "for-anything",
            "an unnarrowed entry matches anything"
        );
    }

    /// An unmatched request does not consume the entries it did not match.
    #[test]
    fn an_unmatched_request_leaves_the_queue_alone() {
        let store = store_with(vec![
            Script::new("fast", Outcome::text("for-alpha")).matching("alpha"),
        ]);
        let Decision::Unscripted { model, reason } = take(&store, "fast", "{\"m\":\"beta\"}")
        else {
            panic!("no entry matches");
        };
        assert_eq!(model, "fast");
        assert!(
            reason.contains("none of the 1 outcome(s) left"),
            "the refusal says the queue is not empty, only unmatched: {reason}"
        );
        assert_eq!(store.snapshot().queues["fast"], 1, "nothing was consumed");
    }

    /// An empty queue and a model nobody ever scripted are different mistakes,
    /// and the refusal says which.
    #[test]
    fn an_unscripted_model_and_a_drained_queue_read_differently() {
        let store = store_with(vec![Script::new("fast", Outcome::text("once"))]);
        assert!(matches!(take(&store, "fast", "{}"), Decision::Serve(_)));

        let Decision::Unscripted { reason, .. } = take(&store, "fast", "{}") else {
            panic!("the queue is drained");
        };
        assert!(reason.contains("queue is empty"), "{reason}");

        let Decision::Unscripted { reason, .. } = take(&store, "smart", "{}") else {
            panic!("the model was never scripted");
        };
        assert!(reason.contains("has ever been enqueued"), "{reason}");
    }

    /// The snapshot counts repetitions, not entries, and drops drained models.
    #[test]
    fn the_snapshot_counts_what_is_left_to_serve() {
        let store = store_with(vec![
            Script::new("fast", Outcome::text("a")).times(2),
            Script::new("fast", Outcome::text("b")),
            Script::new("smart", Outcome::text("c")),
        ]);
        assert_eq!(store.snapshot().queues["fast"], 3);
        assert!(matches!(take(&store, "smart", "{}"), Decision::Serve(_)));
        let snapshot = store.snapshot();
        assert!(
            !snapshot.queues.contains_key("smart"),
            "a drained model leaves the snapshot"
        );
        assert!(!snapshot.is_drained(), "`fast` still holds three");
    }

    /// A control-plane document rejects a key it does not know, so a typo in a
    /// scripted outcome is an error rather than a silently ignored field.
    #[test]
    fn an_unknown_key_in_a_script_is_refused() {
        let error = serde_json::from_value::<Script>(json!({
            "model": "fast",
            "outcome": { "reply": { "body": { "text": "hi" } } },
            "timez": 2
        }))
        .expect_err("`timez` is not a key of `Script`");
        assert!(error.to_string().contains("timez"), "{error}");

        let error = serde_json::from_value::<Script>(json!({
            "model": "fast",
            "outcome": { "reply": { "body": { "txet": "hi" } } }
        }))
        .expect_err("`txet` is not a reply body");
        assert!(error.to_string().contains("txet"), "{error}");
    }

    /// The estimate moves with the text and is stable for the same text.
    #[test]
    fn the_token_estimate_is_a_function_of_the_text() {
        assert_eq!(estimate(""), 0);
        assert_eq!(estimate("abcd"), 1);
        assert_eq!(estimate("abcde"), 2);
        assert_eq!(estimate("a longer prompt"), estimate("a longer prompt"));
    }

    /// `after` moves the delay onto whichever outcome shape carries one.
    #[test]
    fn a_delay_attaches_to_every_outcome_shape() {
        let delay = Duration::from_millis(25);
        let Outcome::Reply(reply) = Outcome::text("hi").after(delay) else {
            panic!("a reply stays a reply");
        };
        assert_eq!(reply.delay.duration(), delay);

        let Outcome::Failure(Failure::Timeout { delay: scripted }) =
            Outcome::timeout(Duration::from_millis(1)).after(delay)
        else {
            panic!("a timeout stays a timeout");
        };
        assert_eq!(scripted.duration(), delay);

        let Outcome::Raw(raw) = Outcome::raw(402, json!({})).after(delay) else {
            panic!("a raw outcome stays raw");
        };
        assert_eq!(raw.delay.duration(), delay);

        // A failure with no delay of its own is delivered at once: only a
        // timeout has a wait to move.
        assert!(Outcome::rate_limit().after(delay).served() == "failure.rate_limit");
    }
}
