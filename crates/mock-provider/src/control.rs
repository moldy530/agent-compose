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
//!   The corollary is worth stating, because it is how a queue is written
//!   wrong: [`Match::Any`] accepts everything, so an **unnarrowed entry shadows
//!   every narrowed entry behind it** for as long as its `times` last. A queue
//!   whose entries answer *different* calls on one model id must narrow all of
//!   them, not just the ones that look ambiguous.
//!
//! # Validation comes first
//!
//! A request that fails validation is answered with the provider's own error and
//! **consumes no outcome**. A queue is a test's statement about the calls a graph
//! makes; letting one malformed call eat the answer meant for the next one would
//! turn a single codegen bug into a cascade of unrelated failures.
//!
//! The two refusals that arrive *after* that point — an outcome the surface
//! cannot render into what the request asked for, and one that cannot be put on
//! the wire — do consume theirs, because the take has already happened by the
//! time the answer is built. What they must not do is look like an answer:
//! [`Store::refused`] rewrites the record so `served` names the refusal, and
//! [`Snapshot::refused`] counts it, so [`Snapshot::is_drained`] cannot call a
//! refused run clean.

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

/// The header a response carries when this server decided the answer itself,
/// rather than a script having asked for one.
///
/// A test that sees it knows the run never reached a scripted outcome: the
/// request was malformed, unscripted, asked for something the script could not
/// render, or asked this endpoint for a structured-output mechanism its
/// [`Personality`] does not carry. The **value** is what says which of those,
/// and a reader has to look at it rather than at the header's presence — the
/// four say different things about whose bug it is, and only
/// [`REFUSED_UNSUPPORTED`] is not a bug at all. Generated code classifies
/// provider failures by status (5.9), so the three harness refusals deliberately
/// carry a status no failover condition claims.
pub const HARNESS_HEADER: &str = "x-mock-provider-error";

/// The status every harness refusal carries.
///
/// Deliberately not a provider status, and deliberately not a *retried* one.
/// Two different mechanisms would otherwise swallow a harness refusal:
///
/// * PRD 5.9 routes on *infrastructure* conditions — 429, 5xx, no answer — so a
///   refusal dressed as one of those would be silently failed over instead of
///   failing the test that has a bug in it;
/// * both official client SDKs (`@anthropic-ai/sdk` and `openai`) retry
///   **408, 409, 429 and 5xx** by default, twice, so a refusal at 409 would be
///   sent three times before the run saw it — three transcript entries, three
///   `unscripted` counts, and a failure whose shape does not match its cause.
///
/// 422 is outside both sets: no failover condition claims it and neither SDK
/// retries it. [`HARNESS_HEADER`] names which refusal it is.
pub const HARNESS_STATUS: u16 = 422;

/// [`HARNESS_HEADER`] on a request the harness refused because it was malformed
/// or unauthenticated — the two cases a *real* provider refuses too, so they
/// carry the provider's own status (400 and 401) rather than [`HARNESS_STATUS`],
/// and the header is what says the refusal came from this server.
pub const REFUSED_INVALID: &str = "invalid-request";
/// [`HARNESS_HEADER`] on a request no scripted outcome answered.
pub const REFUSED_UNSCRIPTED: &str = "unscripted-request";
/// [`HARNESS_HEADER`] on a request whose scripted outcome could not be rendered
/// into an answer the surface could have sent — either because it does not fit
/// what the request asked for, or because the outcome contradicts itself (a
/// stop reason no answer with that body carries).
pub const REFUSED_MISMATCH: &str = "script-mismatch";
/// [`HARNESS_HEADER`] on a scripted outcome that could not be put on the wire at
/// all — a `raw` outcome carrying a header name or value HTTP cannot carry.
pub const REFUSED_UNSENDABLE: &str = "unsendable-response";
/// [`HARNESS_HEADER`] on a **well-formed** request asking this endpoint for a
/// structured-output mechanism its [`Personality`] does not carry (PRD §9
/// resolved q53).
///
/// Apart from [`REFUSED_INVALID`] because the two say opposite things about the
/// generated code: a malformed request is a codegen bug, and this is the
/// endpoint being what a test staged — the same request is recorded with
/// [`Verdict::Valid`] beside `unsupported: Some(_)`, and what the graph does
/// about it is send the call again the other way. Labelling both `invalid-request`
/// would put the wire response and the transcript in contradiction, and would
/// make "no request in this run was malformed" — the natural way to assert that
/// every request was composed correctly — fail on every correct laddering run.
///
/// It rides the *provider's* own 400 rather than [`HARNESS_STATUS`], because
/// unlike the other three this refusal is one a real endpoint sends; the header
/// is only how a reader of a transcript tells a staged one from a scripted
/// `raw` 400.
pub const REFUSED_UNSUPPORTED: &str = "unsupported-mechanism";

/// The two refusals that happen **after** an outcome has been taken from its
/// queue, spelled as [`RecordedRequest::served`] records them.
///
/// They are the pair a transcript could most easily lie about: the queue take
/// has already happened when the refusal is decided, so a `served` written from
/// the taken outcome would say `reply.text` about a call that received a 422 and
/// no reply. Counted apart from `invalid` and `unscripted` in [`Snapshot`] for
/// the same reason those two are counted at all — [`Snapshot::is_drained`] is
/// the suite's clean-run assertion, and a refusal it could not see would let a
/// failed run report a clean one.
const REFUSED_AFTER_TAKING: &[&str] = &[REFUSED_MISMATCH, REFUSED_UNSENDABLE];

/// Which provider surface a request arrived on.
///
/// The six provider kinds of grammar 12.1 funnel into two HTTP protocols:
/// `anthropic` speaks the Messages API, and `openai` / `openai_compatible` /
/// `azure_openai` all speak Chat Completions, Azure differing by route and
/// auth header rather than by body. `bedrock` and `vertex` are reached through
/// cloud SDKs and have no HTTP surface this server can stand in for — they are
/// out of scope for v0, and `WIRE-NOTES.md` says so.
/// A surface is spelled the way grammar 12.1 spells the provider kinds that
/// reach it — `openai`, not `open_ai` — because the reader of a transcript is a
/// harness written against the DSL, and a `snake_case` derive would have split
/// the names into a second vocabulary nobody writes. [`Surface::as_str`] is the
/// same spelling for a Rust caller, and a test pins the two together.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Surface {
    /// `POST /v1/messages`.
    #[serde(rename = "anthropic")]
    Anthropic,
    /// `POST /v1/chat/completions`.
    #[serde(rename = "openai")]
    OpenAi,
    /// `POST /openai/deployments/<deployment>/chat/completions` and the newer
    /// `POST /openai/v1/chat/completions`.
    #[serde(rename = "azure_openai")]
    AzureOpenAi,
    /// `POST /v1/responses` — OpenAI's Responses API.
    ///
    /// Its own surface rather than a route of [`OpenAi`](Self::OpenAi): the
    /// request is a list of *items* rather than messages, the tool shape is
    /// flat, structured output is `text.format` rather than `response_format`,
    /// and it is the only OpenAI wire that carries the built-in server-tool
    /// suite at all. A compiled graph reaches it when its `openai` provider
    /// declares `server_tools:` (grammar 12.1, Decision D122).
    #[serde(rename = "responses")]
    Responses,
}

impl Surface {
    /// The name this surface is spelled with in a control-plane document.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Anthropic => "anthropic",
            Self::OpenAi => "openai",
            Self::AzureOpenAi => "azure_openai",
            Self::Responses => "responses",
        }
    }
}

/// The most requests one scripted outcome may answer.
///
/// A bound rather than `u32::MAX` because `times` is arithmetic the server does
/// on every call and sums across a queue: an unbounded count is a queue depth
/// that overflows the snapshot, and a snapshot is what `/_mock/state` and
/// [`Store::reset`] both render. Well above anything a run needs — the longest
/// scripted loop in the acceptance suite is eight — and low enough that a whole
/// queue of them still fits in the sum.
pub(crate) const MAX_TIMES: u32 = 1_000_000;

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
    ///
    /// At least 1 and at most a million; a control-plane document outside that
    /// range is refused by name rather than queued. Zero is not "never answers"
    /// but a queue entry that cannot be reached, and a count near `u32::MAX` is
    /// a number no run could consume and every snapshot would have to add up.
    #[serde(default = "once", deserialize_with = "repetitions")]
    pub times: u32,
    /// Which requests it is willing to answer.
    #[serde(default, rename = "match", skip_serializing_if = "Match::is_any")]
    pub matcher: Match,
}

fn once() -> u32 {
    1
}

/// `times`, bounded — see [`Script::times`].
fn repetitions<'de, D>(deserializer: D) -> Result<u32, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let times = u32::deserialize(deserializer)?;
    if !(1..=MAX_TIMES).contains(&times) {
        return Err(serde::de::Error::custom(format!(
            "`times` must be between 1 and {MAX_TIMES}, not {times}"
        )));
    }
    Ok(times)
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
    ///
    /// # Panics
    ///
    /// If `times` is zero or above a million, which is the same range the
    /// control plane enforces. A test that asks for either has a bug in the
    /// script rather than in the graph, and the panic names it where it was
    /// written instead of failing later as a queue that answers nothing.
    #[must_use]
    pub fn times(mut self, times: u32) -> Self {
        assert!(
            (1..=MAX_TIMES).contains(&times),
            "a scripted outcome answers between 1 and {MAX_TIMES} requests, not {times}"
        );
        self.times = times;
        self
    }

    /// Answer only requests whose body contains `needle`.
    #[must_use]
    pub fn matching(mut self, needle: impl Into<String>) -> Self {
        self.matcher = Match::BodyContains(needle.into());
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
///
/// A newtype variant rather than a struct one so the control-plane spelling is
/// `"match": {"body_contains": "a draft"}` — the shape a TypeScript harness
/// writes by hand — instead of repeating the key inside itself. [`Match::Any`]
/// is the default and is omitted on the way out, so a scripted document only
/// mentions `match` when it narrows.
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Match {
    /// Any request for the model. The default, and plain FIFO.
    #[default]
    Any,
    /// Only requests whose canonically serialized body contains this substring.
    BodyContains(String),
}

impl Match {
    /// Whether this matcher accepts `body`, already canonically serialized.
    fn accepts(&self, body: &str) -> bool {
        match self {
            Self::Any => true,
            Self::BodyContains(needle) => body.contains(needle.as_str()),
        }
    }

    /// Whether this narrows nothing.
    fn is_any(&self) -> bool {
        matches!(self, Self::Any)
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
    /// provider sends. Nothing about its **body** is validated on the way out;
    /// its `headers` are the one exception, because a header HTTP cannot carry
    /// is not a response generated code can reject but one it never receives
    /// (see [`RawOutcome::headers`]).
    Raw(RawOutcome),
}

impl Outcome {
    /// Structured output: rendered through whichever mechanism the request asked
    /// for (a forced tool on the Anthropic surface, `response_format` or a
    /// forced function on the OpenAI one).
    ///
    /// A Chat Completions request may carry **both** of its mechanisms, and the
    /// pinned call wins: `response_format` shapes the content, and a tool pin
    /// decides whether the turn has content at all. A pin that names no function
    /// — `tool_choice: "required"` — leaves this reply nothing to make the call
    /// under and is refused as a `script-mismatch`; script `tools` there.
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
    ///
    /// At least one call, and only against a request that leaves tool use legal:
    /// an empty list, or any list against a `tool_choice` of `none`, is a shape
    /// neither API can send and is refused as a `script-mismatch` — see
    /// [`ReplyBody::Tools`].
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
    ///
    /// # Panics
    ///
    /// If `status` is not one HTTP can carry (it must be in 100..=999), which
    /// is the same range the control plane enforces — see [`RawOutcome::status`].
    /// A test that writes one has a bug in the script rather than in the graph,
    /// and the panic names it where it was written instead of serving something
    /// else entirely.
    #[must_use]
    pub fn raw(status: u16, body: Value) -> Self {
        assert!(
            sendable_status(status),
            "a `raw` outcome's status must be one HTTP can carry (100..=999), not {status}"
        );
        Self::Raw(RawOutcome {
            status,
            body,
            headers: BTreeMap::new(),
            delay: Delay::none(),
        })
    }

    /// The same reply, with server-tool activity woven into the turn ahead of
    /// it (Decision D122).
    ///
    /// # Panics
    ///
    /// If the outcome is not a reply. A failure and a `raw` body have no turn
    /// for a server tool to have run inside, so a builder that accepted one
    /// would have to drop it — the same rule, and the same reason, as
    /// [`Outcome::after`]'s.
    #[must_use]
    pub fn with_server_tools(self, uses: Vec<ServerToolUse>) -> Self {
        match self {
            Self::Reply(reply) => Self::Reply(Reply {
                server_tools: uses,
                ..reply
            }),
            other => panic!(
                "`{}` is not a model turn, so a server tool cannot have run inside it: script a \
                 `reply`, or `raw` for a response generated code must reject",
                other.served()
            ),
        }
    }

    /// Answer only after `delay`.
    ///
    /// The scheduling knob a fan-out test needs: with completion order forced,
    /// "appended results are reordered by source-item index" (PRD 5.6) becomes
    /// an assertion rather than a coin flip.
    ///
    /// # Panics
    ///
    /// If the outcome is a `rate_limit`, `overloaded` or `server_error`
    /// failure. Those three are the shapes with nowhere to carry a wait: they
    /// are rendered from the surface's own error envelope, and a scripted
    /// document cannot express one on them either ([`Failure`] denies unknown
    /// fields), so a builder that accepted one here would have to drop it. A
    /// dropped delay is the worst of the three outcomes — the test staging "the
    /// first route member rate-limits 300ms in" would get an instant 429 and
    /// assert about an ordering that never happened — so it is refused where it
    /// was written instead, the same way [`Outcome::raw`] refuses a status HTTP
    /// cannot carry. A late refusal is scriptable two ways: `Outcome::timeout`
    /// for no answer at all, and `Outcome::raw(429, body).after(delay)` for a
    /// late one with a status generated code classifies (PRD 5.9).
    #[must_use]
    pub fn after(self, delay: Duration) -> Self {
        let delay = Delay::from(delay);
        match self {
            Self::Reply(reply) => Self::Reply(Reply { delay, ..reply }),
            Self::Raw(raw) => Self::Raw(RawOutcome { delay, ..raw }),
            Self::Failure(Failure::Timeout { .. }) => Self::Failure(Failure::Timeout { delay }),
            Self::Failure(other) => panic!(
                "`{}` has no wait to carry, so `after` would drop it: script \
                 `Outcome::timeout(delay)` for a late non-answer, or \
                 `Outcome::raw(status, body).after(delay)` for a late refusal",
                other.served()
            ),
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
    /// Token counts, each at most [`MAX_TOKENS`]. Defaults to a deterministic
    /// estimate over the rendered request and response (see `usage`), which is
    /// what lets a test assert that accounting reaches the trace at all without
    /// pinning a real tokenizer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    /// Overrides the stop reason the body implies.
    ///
    /// Held to what the answering surface could have sent: a value in that
    /// surface's closed set, and one the body can carry — the tool reason names
    /// calls a prose answer does not have, and the plain-end reason denies the
    /// calls that ended the turn. A script that says otherwise is answered as
    /// the harness bug it is rather than rendered, the same way an impossible
    /// [`ReplyBody`] is. Anything else is [`Outcome::Raw`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop_reason: Option<String>,
    /// Server tools the provider ran **inside** this turn, in the order they
    /// ran (grammar 12.1, Decision D122).
    ///
    /// Not tool calls: a server tool executes on the provider's side and its
    /// record arrives already answered, woven into the turn ahead of whatever
    /// the model then said. So this is not a [`ReplyBody`] variant — it composes
    /// with all three of them — and a graph that receives one runs nothing.
    ///
    /// Each entry must name a `type:` **this request declared**, for the same
    /// reason a scripted tool call must name a function the request offered: a
    /// provider does not run a tool it was not given.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub server_tools: Vec<ServerToolUse>,
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
            server_tools: Vec::new(),
            delay: Delay::none(),
        }
    }
}

/// One server-tool use in a scripted reply (Decision D122).
///
/// Written once and rendered into whichever wire is answering: the Messages API
/// puts it on the turn as a `server_tool_use` block and its paired
/// `*_tool_result`, the Responses API as a `<type>_call` item. A test scripts
/// the *use*, not either spelling, so the same script drives both wires.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ServerToolUse {
    /// The tool's `type:`, exactly as the request's `server_tools:` declared it
    /// — `web_search_20250305` on the Messages wire, `web_search` on Responses.
    #[serde(rename = "type")]
    pub type_name: String,
    /// What the provider was asked, as the use block carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<Value>,
    /// What it found, as the result block carries it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// Overrides the derived id, for a test that wants to pin the correlation
    /// between the use and its result itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

impl ServerToolUse {
    /// A use of `type_name` that found `result`.
    #[must_use]
    pub fn new(type_name: impl Into<String>, input: Value, result: Value) -> Self {
        Self {
            type_name: type_name.into(),
            input: Some(input),
            result: Some(result),
            id: None,
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
    ///
    /// Checked against what the request permits, the same way a `structured`
    /// reply is: every call must name a tool the request offered, the list must
    /// not be empty (a `tool_use` / `tool_calls` stop reason names the block
    /// that ended the turn, and there would be none), and the request must not
    /// have forbidden tool use with `tool_choice: none`. A script that says
    /// otherwise is answered as the harness bug it is rather than rendered into
    /// a shape no provider sends.
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

/// The largest token count a scripted [`Usage`] may report.
///
/// Bounded for the same reason `times` is: a token count is arithmetic the
/// server does. The Chat Completions surface serves `total_tokens`, which is the
/// **sum** of the two, so an unbounded pair is an addition that overflows — and
/// an overflow in a render is a panicked connection task, which is a *dropped
/// connection*, which PRD 5.9 classifies as a provider timeout. The bound also
/// keeps both counts and their sum inside the integers JavaScript represents
/// exactly (2^53 - 1): the client reading them is generated TypeScript, and a
/// count it cannot round-trip is a count no test can assert on.
///
/// A billion is far above anything a real call reports — the widest context
/// window in service is three orders of magnitude below it — so nothing a run
/// needs is refused by it.
pub(crate) const MAX_TOKENS: u64 = 1_000_000_000;

/// Token counts, as both surfaces report them.
///
/// Each count is at most [`MAX_TOKENS`]; a control-plane document outside that
/// range is refused by name rather than queued.
#[derive(Clone, Copy, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Usage {
    #[serde(deserialize_with = "counted")]
    pub input_tokens: u64,
    #[serde(deserialize_with = "counted")]
    pub output_tokens: u64,
}

impl Usage {
    /// The counts a reply reports.
    ///
    /// # Panics
    ///
    /// If either count is above [`MAX_TOKENS`], which is the same bound the
    /// control plane enforces. A test that asks for one has a bug in the script
    /// rather than in the graph, and the panic names it where it was written
    /// instead of serving a `usage` block no provider sends.
    #[must_use]
    pub fn new(input_tokens: u64, output_tokens: u64) -> Self {
        for count in [input_tokens, output_tokens] {
            assert!(
                count <= MAX_TOKENS,
                "a scripted token count is at most {MAX_TOKENS}, not {count}"
            );
        }
        Self {
            input_tokens,
            output_tokens,
        }
    }
}

/// A token count, bounded — see [`Usage`].
fn counted<'de, D>(deserializer: D) -> Result<u64, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let count = u64::deserialize(deserializer)?;
    if count > MAX_TOKENS {
        return Err(serde::de::Error::custom(format!(
            "a token count is at most {MAX_TOKENS}, not {count}"
        )));
    }
    Ok(count)
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
    /// The status to answer with, which must be one HTTP can carry
    /// (100..=999).
    ///
    /// Checked on the way in for the same reason the headers are: a status
    /// outside the range cannot be put on the wire, and the fallback for one
    /// that cannot is **500** — a PRD 5.9 failover condition and a status both
    /// SDKs retry. A typo'd `42` would therefore stage "the provider had an
    /// error" while the script said "a response generated code must reject",
    /// and the run would report a different failure than the one it has. Refused
    /// by name here, and again on the way out (`server::render`), because this
    /// is a public field a struct literal can fill.
    #[serde(deserialize_with = "carried")]
    pub status: u16,
    pub body: Value,
    /// Extra response headers, on top of `content-type`.
    ///
    /// Checked on the way in: a name or value HTTP cannot carry is refused by
    /// name here rather than queued, because a queue entry that cannot be sent
    /// is a script that fails as a *dropped connection* — which PRD 5.9
    /// classifies as a provider timeout. See [`crate::wire::header`].
    #[serde(
        default,
        deserialize_with = "sendable",
        skip_serializing_if = "BTreeMap::is_empty"
    )]
    pub headers: BTreeMap<String, String>,
    #[serde(default, skip_serializing_if = "Delay::is_none")]
    pub delay: Delay,
}

/// Response headers, refused unless every one of them can be sent — see
/// [`RawOutcome::headers`].
fn sendable<'de, D>(deserializer: D) -> Result<BTreeMap<String, String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let headers = BTreeMap::<String, String>::deserialize(deserializer)?;
    for (name, value) in &headers {
        crate::wire::header(name, value).map_err(serde::de::Error::custom)?;
    }
    Ok(headers)
}

/// A status, refused unless HTTP can carry it — see [`RawOutcome::status`].
fn carried<'de, D>(deserializer: D) -> Result<u16, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let status = u16::deserialize(deserializer)?;
    if !sendable_status(status) {
        return Err(serde::de::Error::custom(format!(
            "`{status}` is not a status HTTP can carry (100..=999)"
        )));
    }
    Ok(status)
}

/// Whether HTTP can carry this status code at all.
///
/// The one place the range is stated, so the builder, the control plane and the
/// connection loop cannot drift apart about it.
#[must_use]
pub(crate) fn sendable_status(status: u16) -> bool {
    (100..=999).contains(&status)
}

/// The id every answer carries back, derived from the request's own arrival
/// sequence — `req_mock_00000003` is the third request's.
///
/// One function because both surfaces send it and both send it on **errors as
/// well as answers**, which is what the real APIs do: it is the identifier an
/// SDK's error object surfaces and the one a transcript reader correlates by.
pub(crate) fn request_id(sequence: u64) -> String {
    format!("req_mock_{sequence:08}")
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
    /// Whether this is a missing **credential** rather than a malformed request.
    ///
    /// Both real APIs check authentication before they look at the body, and
    /// both answer a missing key with **401**, which is the status the two
    /// client SDKs raise `AuthenticationError` from — a different class than the
    /// 400 a bad body raises. Generated code that classifies the two apart has
    /// to be taught the same difference here, so the flag travels with the
    /// complaint and each surface's `rejected()` reads it. Recorded in the
    /// transcript only when true, so an ordinary failure reads as it always did.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub authentication: bool,
}

impl ValidationFailure {
    pub(crate) fn new(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            pointer: pointer.into(),
            message: message.into(),
            authentication: false,
        }
    }

    /// A missing credential — see [`ValidationFailure::authentication`].
    pub(crate) fn credential(pointer: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            authentication: true,
            ..Self::new(pointer, message)
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
///
/// Four spellings across three wires, and [`Self::mechanism`] is the axis that
/// matters to PRD §9 resolved q53: each wire has one **native** parameter and
/// one **forced-tool** pin, and a generated graph rides whichever the endpoint
/// accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StructuredOutput {
    /// Anthropic: a tool the request forces the model to call, whose
    /// `input_schema` is the agent's output schema.
    ForcedTool { name: String, schema: Value },
    /// Anthropic: `output_config: { format: { type: json_schema, schema } }` —
    /// the Messages API's own structured output (PRD §9 resolved q53).
    ///
    /// **Nameless**, and that is the wire's shape rather than an omission: the
    /// forced tool's name is the compiler's own synthetic one and exists to be
    /// pinned by `tool_choice`, while a format constrains the assistant's text
    /// and has no call to name.
    OutputConfig { schema: Value },
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
            | Self::OutputConfig { schema }
            | Self::JsonSchema { schema, .. }
            | Self::ForcedFunction { schema, .. } => schema,
        }
    }

    /// The name the request gave it, on the three spellings that have one.
    #[must_use]
    pub fn name(&self) -> Option<&str> {
        match self {
            Self::ForcedTool { name, .. }
            | Self::JsonSchema { name, .. }
            | Self::ForcedFunction { name, .. } => Some(name),
            Self::OutputConfig { .. } => None,
        }
    }

    /// Which of the two mechanisms this spelling is (PRD §9 resolved q53).
    #[must_use]
    pub fn mechanism(&self) -> OutputMechanism {
        match self {
            Self::OutputConfig { .. } | Self::JsonSchema { .. } => OutputMechanism::Native,
            Self::ForcedTool { .. } | Self::ForcedFunction { .. } => OutputMechanism::ForcedTool,
        }
    }
}

/// Which of a wire's two ways of asking for an object a request used
/// (PRD §9 resolved q53).
///
/// The same two rungs the generated runtime ladders between, spelled the same
/// way its trace spells them (`docs/trace.md` §7) so a transcript here and a
/// trace there use one vocabulary:
///
///  * `native` — the wire's own structured-output parameter: `output_config`'s
///    `format` on the Messages wire, `response_format` on Chat Completions,
///    `text.format` on Responses;
///  * `forced_tool` — the synthetic output tool, pinned by name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OutputMechanism {
    Native,
    ForcedTool,
}

impl OutputMechanism {
    /// How this mechanism is spelled in a control-plane document.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Native => "native",
            Self::ForcedTool => "forced_tool",
        }
    }
}

/// Which structured-output mechanisms one endpoint carries (PRD §9 resolved
/// q53, ruling c).
///
/// The one thing about this server that is a property of the **endpoint** rather
/// than of a scripted answer, and the reason it is not a [`Script`]: a mechanism
/// this endpoint does not have is refused *before* anything is taken from a
/// queue, exactly as a malformed request is, because a generated graph's
/// response to that refusal is to send the **same** call again the other way. A
/// personality that ate a scripted outcome per refusal would make the ladder
/// unscriptable — the second, working call would find the queue one short.
///
/// Registered per model id, like a queue, and cleared by
/// [`Store::reset`]. The default is the endpoint every other test in the suite
/// has always been talking to: one that takes both.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Personality {
    /// Both mechanisms work — `api.anthropic.com` and `api.openai.com` today,
    /// and the default for every model nothing has registered.
    #[default]
    NativeSupported,
    /// A **lagging gateway**: an Anthropic- or OpenAI-compatible proxy a
    /// generation behind the wire it forwards, which has never heard of the
    /// native structured-output parameter and 400s it. Forced tool use works,
    /// which is why that mechanism was the least common denominator for as long
    /// as it was.
    NativeRejected,
    /// The **newest model generation**, which removed forced tool use outright
    /// and names structured outputs as the replacement: `tool_choice` of type
    /// `tool`/`any` is a 400 and the native parameter works.
    ForcedToolRemoved,
    /// Neither — the endpoint a compiled graph cannot use at all, and the one
    /// the double-refusal diagnostic exists for.
    BothRejected,
}

impl Personality {
    /// Whether an endpoint of this personality carries `mechanism`.
    #[must_use]
    pub fn carries(self, mechanism: OutputMechanism) -> bool {
        match (self, mechanism) {
            (Self::NativeSupported, _) => true,
            (Self::BothRejected, _) => false,
            (Self::NativeRejected, mechanism) => mechanism == OutputMechanism::ForcedTool,
            (Self::ForcedToolRemoved, mechanism) => mechanism == OutputMechanism::Native,
        }
    }

    /// How this personality is spelled in a control-plane document.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NativeSupported => "native_supported",
            Self::NativeRejected => "native_rejected",
            Self::ForcedToolRemoved => "forced_tool_removed",
            Self::BothRejected => "both_rejected",
        }
    }
}

/// One model's registered [`Personality`], as the control plane spells it.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Endpoint {
    /// The model id whose endpoint this describes.
    pub model: String,
    /// Which mechanisms it carries.
    pub personality: Personality,
}

impl Endpoint {
    /// `model`'s endpoint, with this personality.
    #[must_use]
    pub fn new(model: impl Into<String>, personality: Personality) -> Self {
        Self {
            model: model.into(),
            personality,
        }
    }
}

/// How an endpoint refuses a structured-output mechanism it does not carry
/// (PRD §9 resolved q53, ruling c).
///
/// **The refusal texts are the load-bearing part of the personality**, not the
/// status: the generated runtime decides whether to ladder by reading the body,
/// so a mock that refused with a wording no service uses would let a recognizer
/// that matches nothing pass. Each of these is one of the two families the
/// ruling names — an unknown key, and a key this model does not take — written
/// in the dialect of the surface that is answering.
///
/// Stated once here rather than per surface module so that the four sentences a
/// test pins and the four a run receives cannot drift apart.
pub(crate) fn unsupported_mechanism(
    surface: Surface,
    mechanism: OutputMechanism,
) -> ValidationFailure {
    match (surface, mechanism) {
        // The Messages API validates with pydantic, so an argument it has never
        // heard of is an extra input — the same sentence [`crate::strict`]'s
        // Anthropic dialect writes for every other unknown key.
        (Surface::Anthropic, OutputMechanism::Native) => ValidationFailure::new(
            "output_config",
            "output_config: Extra inputs are not permitted",
        ),
        // …and the newest generation's own words for the mechanism it removed.
        (Surface::Anthropic, OutputMechanism::ForcedTool) => ValidationFailure::new(
            "tool_choice",
            "tool_choice: type \"tool\" and \"any\" are not supported for this model.",
        ),
        (Surface::Responses, OutputMechanism::Native) => ValidationFailure::new(
            "text.format",
            "Invalid parameter: 'text.format' of type 'json_schema' is not supported with this model.",
        ),
        // A gateway that forwards Chat Completions for a model generation whose
        // structured outputs it predates: the argument is not one it knows.
        (Surface::OpenAi | Surface::AzureOpenAi, OutputMechanism::Native) => {
            ValidationFailure::new(
                "response_format",
                "Unrecognized request argument supplied: response_format",
            )
        }
        (_, OutputMechanism::ForcedTool) => ValidationFailure::new(
            "tool_choice",
            "Invalid parameter: 'tool_choice' of type 'function' is not supported with this model.",
        ),
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
    /// The `type:` of every **server** tool the request declared, in request
    /// order (grammar 12.1, Decision D122).
    ///
    /// Kept apart from `tools` because they are different things on the wire and
    /// in the graph: `tools` are the functions the runtime dispatches, and these
    /// are the tools the provider runs itself. A test asserting that a
    /// provider's suite reached the request reads this for the names and
    /// [`Self::body`] for the configs, which arrive verbatim.
    pub server_tools: Vec<String>,
    /// How the request asked for structured output, if it did.
    pub structured_output: Option<StructuredOutput>,
    /// The mechanism this **endpoint** refused, when its [`Personality`] does
    /// not carry the one the request asked for (PRD §9 resolved q53).
    ///
    /// A refusal that is not a complaint about the request: `verdict` stays
    /// `valid` because the request was well formed, and this is what says the
    /// client got a 400 anyway. A generated graph answers it by sending the same
    /// call through the other mechanism, so a transcript carrying one of these
    /// followed by an answered call **is** the ladder, and a second run of the
    /// same process carrying none is the memoization.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unsupported: Option<OutputMechanism>,
    /// What the server **answered** it with — not merely what the queue handed
    /// over.
    ///
    /// `reply.text`, `reply.structured`, `reply.tools`, `failure.*` and `raw`
    /// name a scripted outcome that was served. `rejected` and `unscripted` are
    /// refusals that took nothing from the queue. `script-mismatch` and
    /// `unsendable-response` are refusals that took an outcome and then could
    /// not send it ([`REFUSED_MISMATCH`], [`REFUSED_UNSENDABLE`]): the queue
    /// entry is gone — `/_mock/state` shows the depth it left — but no reply
    /// reached the client, and recording the taken outcome here would tell a
    /// test the opposite of what the client saw.
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

    /// Whether this endpoint refused the structured-output mechanism the
    /// request asked for ([`Self::unsupported`], PRD §9 resolved q53).
    #[must_use]
    pub fn was_unsupported(&self) -> bool {
        self.unsupported.is_some()
    }

    /// Whether the harness refused this call after taking its scripted
    /// outcome — a `script-mismatch` or an `unsendable-response`.
    ///
    /// The client got a 422 and no reply, so a test reading the transcript to
    /// find out what a run received asks this rather than reading [`Self::served`]
    /// for a reply name that is not there.
    #[must_use]
    pub fn was_refused(&self) -> bool {
        REFUSED_AFTER_TAKING.contains(&self.served.as_str())
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
    /// How many of them took an outcome and were refused anyway — a
    /// `script-mismatch` or an `unsendable-response` (see
    /// [`REFUSED_AFTER_TAKING`]).
    ///
    /// Disjoint from the two counts above: those refuse *before* the queue is
    /// touched, this one after, which is why it needs a counter of its own for
    /// [`Self::is_drained`] to mean what it says.
    pub refused: usize,
    /// How many well-formed requests this server refused for asking through a
    /// structured-output mechanism the model's [`Personality`] does not carry
    /// (PRD §9 resolved q53).
    ///
    /// Counted, and deliberately **not** counted by [`Self::is_drained`]: it is
    /// the only refusal here that is a property of the endpoint a test staged
    /// rather than of the request a graph sent, so a run that laddered exactly
    /// as it was asked to is a clean run. What keeps that from hiding a real
    /// failure is that it takes nothing from a queue — a graph that never sent
    /// the second call leaves the scripted answer behind, and the queue depth
    /// `is_drained` does read says so.
    pub unsupported: usize,
    /// Which models have a [`Personality`] registered, and which one — so a
    /// test reading `/_mock/state` can see the endpoint it staged.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub personalities: BTreeMap<String, Personality>,
}

impl Snapshot {
    /// Whether every scripted outcome was consumed and nothing was refused —
    /// the shape a finished acceptance run should have.
    #[must_use]
    pub fn is_drained(&self) -> bool {
        self.queues.is_empty() && self.invalid == 0 && self.unscripted == 0 && self.refused == 0
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
    /// Refuse: the request was **well formed** and this endpoint does not carry
    /// the structured-output mechanism it asked for ([`Personality`], PRD §9
    /// resolved q53).
    ///
    /// Rendered by the same surface method [`Self::Rejected`] is — a service
    /// refusing an argument it does not have answers its ordinary 400 — and
    /// recorded differently, because the two say opposite things about the
    /// generated code: a rejection is a codegen bug, and this is the endpoint
    /// being what it is. It takes **nothing** from the queue, so the retry that
    /// follows it finds the scripted answer waiting.
    Unsupported(Vec<ValidationFailure>),
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
    /// Which structured-output mechanisms each model's endpoint carries
    /// ([`Personality`]). Absent means the default: both.
    personalities: BTreeMap<String, Personality>,
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
    pub(crate) server_tools: Vec<String>,
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

    /// Register which structured-output mechanisms one model's endpoint carries
    /// ([`Personality`], PRD §9 resolved q53).
    ///
    /// Last registration wins, and [`Personality::NativeSupported`] is what a
    /// model nothing has registered already behaves as — so this is only ever
    /// written by a test staging an endpoint that is *not* today's.
    pub fn set_personality(&self, model: impl Into<String>, personality: Personality) {
        self.lock().personalities.insert(model.into(), personality);
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
        state.personalities.clear();
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
                    // Saturating, not `sum`: a snapshot is taken on the way out
                    // of `enqueue`, on every `/_mock/state`, and — before
                    // anything is cleared — inside `reset`. An overflow here
                    // would panic all three, and the one that can recover the
                    // store last, so the count is capped rather than trusted.
                    let depth = queue
                        .iter()
                        .fold(0u32, |depth, script| depth.saturating_add(script.times));
                    (model.clone(), depth)
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
            refused: state
                .requests
                .iter()
                .filter(|request| request.was_refused())
                .count(),
            unsupported: state
                .requests
                .iter()
                .filter(|request| request.was_unsupported())
                .count(),
            personalities: state.personalities.clone(),
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

        // Matchers read the **canonical** serialization rather than the bytes
        // that arrived, so a narrowing like `.matching("task-3")` means the same
        // thing whatever whitespace a client's serializer chose. A body that did
        // not parse has no canonical form, and matches on what it sent.
        let matched_against = incoming
            .body
            .as_ref()
            .map_or_else(|| incoming.body_text.clone(), canonical);
        // What this endpoint does not carry, before what its queue holds: a
        // mechanism refusal is decided by the *endpoint* and takes nothing, so
        // it has to be answered ahead of the take (PRD §9 resolved q53).
        let unsupported: Option<OutputMechanism> = if incoming.failures.is_empty() {
            let personality = state
                .personalities
                .get(&incoming.model)
                .copied()
                .unwrap_or_default();
            incoming.structured_output.as_ref().and_then(|asked| {
                let mechanism = asked.mechanism();
                (!personality.carries(mechanism)).then_some(mechanism)
            })
        } else {
            None
        };

        let decision = match (incoming.failures.is_empty(), unsupported) {
            (false, _) => Decision::Rejected(incoming.failures.clone()),
            (true, Some(mechanism)) => {
                Decision::Unsupported(vec![unsupported_mechanism(incoming.surface, mechanism)])
            }
            (true, None) => Self::take(&mut state.queues, &incoming.model, &matched_against),
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
            Decision::Unsupported(_) => "unsupported".to_string(),
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
            server_tools: incoming.server_tools,
            structured_output: incoming.structured_output,
            unsupported,
            served,
        });

        (decision, sequence)
    }

    /// Record that request `sequence` was refused *after* its outcome had been
    /// taken, so the transcript says what the client actually received.
    ///
    /// Two refusals are decided downstream of [`Self::serve`] — the surface
    /// finding that a taken outcome cannot answer this request
    /// ([`REFUSED_MISMATCH`]), and the connection loop finding that the answer
    /// cannot be put on the wire ([`REFUSED_UNSENDABLE`]) — because both need
    /// the rendered answer, which the store never sees. Without this the
    /// transcript would name the taken outcome as though it had been sent, and
    /// [`Snapshot::is_drained`] would report a clean run: in an e2e run the 422
    /// goes to the generated process, so the Rust side would be left with a
    /// graph that failed for no visible reason.
    ///
    /// A no-op if the record is gone — a [`Self::reset`] between the answer and
    /// this call — because a refusal that outlived its transcript is not worth
    /// a panic in a harness.
    pub(crate) fn refused(&self, sequence: u64, refusal: &str) {
        debug_assert!(
            REFUSED_AFTER_TAKING.contains(&refusal),
            "`{refusal}` is not a refusal that happens after an outcome is taken"
        );
        let Ok(index) = usize::try_from(sequence.saturating_sub(1)) else {
            return;
        };
        let mut state = self.lock();
        if let Some(request) = state.requests.get_mut(index) {
            request.served = refusal.to_string();
        }
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
        // Saturating, because `times` is a public field: the builder and the
        // control plane both refuse a zero, but a struct literal can still write
        // one, and a `u32` that wrapped here would panic the connection task —
        // which a client reads as a dropped connection and PRD 5.9 classifies as
        // a *timeout*. A harness bug must never arrive wearing a failover
        // condition.
        script.times = script.times.saturating_sub(1);
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

    /// The corollary, pinned: an unnarrowed entry accepts everything, so it
    /// shadows every narrowed entry behind it. A queue written this way answers
    /// the *wrong* call rather than refusing, which is why it is stated in the
    /// module header and asserted here.
    #[test]
    fn an_unnarrowed_entry_shadows_the_narrowed_ones_behind_it() {
        let store = store_with(vec![
            Script::new("fast", Outcome::text("for-anything")).times(2),
            Script::new("fast", Outcome::text("for-beta")).matching("beta"),
        ]);

        let served = |body: &str| match take(&store, "fast", body) {
            Decision::Serve(Outcome::Reply(Reply {
                body: ReplyBody::Text(text),
                ..
            })) => text,
            other => panic!("expected a text reply, got {other:?}"),
        };

        assert_eq!(
            served("{\"m\":\"beta\"}"),
            "for-anything",
            "the entry that narrows nothing is in front, so it answers a request written for the one behind it"
        );
        assert_eq!(served("{\"m\":\"beta\"}"), "for-anything");
        assert_eq!(
            served("{\"m\":\"beta\"}"),
            "for-beta",
            "only once the shadowing entry is spent does the narrowed one serve"
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

    /// A `times` outside the range a run could consume is refused on the way
    /// in, at both spellings — because `times` is arithmetic the server does on
    /// every call and sums across a queue, and neither is checked at the point
    /// it happens.
    #[test]
    fn a_times_outside_its_range_is_refused_by_the_control_plane() {
        let script = |times: Value| {
            serde_json::from_value::<Script>(json!({
                "model": "fast",
                "outcome": { "reply": { "body": { "text": "hi" } } },
                "times": times,
            }))
        };

        for refused in [json!(0), json!(MAX_TIMES + 1), json!(u32::MAX)] {
            let error = script(refused.clone())
                .expect_err("`times` is bounded")
                .to_string();
            assert!(error.contains("`times` must be between 1 and"), "{error}");
        }
        assert_eq!(script(json!(1)).expect("one is the smallest").times, 1);
        assert_eq!(
            script(json!(MAX_TIMES))
                .expect("a million is the largest")
                .times,
            MAX_TIMES
        );
    }

    /// The same bound, at the Rust spelling: a script written with a `times` no
    /// run could consume says so where it was written.
    #[test]
    #[should_panic(expected = "answers between 1 and 1000000 requests, not 0")]
    fn a_zero_times_is_refused_by_the_builder() {
        let _ = Script::new("fast", Outcome::text("hi")).times(0);
    }

    /// A scripted token count is bounded on the way in, for the same reason
    /// `times` is: the counts are arithmetic the server does — Chat Completions
    /// serves their **sum** as `total_tokens` — and an addition that overflows
    /// panics the connection task, which a client reads as a dropped connection
    /// and PRD 5.9 classifies as a provider timeout.
    #[test]
    fn a_token_count_outside_its_range_is_refused_by_the_control_plane() {
        let script = |usage: Value| {
            serde_json::from_value::<Script>(json!({
                "model": "fast",
                "outcome": { "reply": { "body": { "text": "hi" }, "usage": usage } },
            }))
        };

        for refused in [
            json!({ "input_tokens": MAX_TOKENS + 1, "output_tokens": 1 }),
            json!({ "input_tokens": 1, "output_tokens": u64::MAX }),
            json!({ "input_tokens": u64::MAX, "output_tokens": u64::MAX }),
        ] {
            let error = script(refused.clone())
                .expect_err("a token count is bounded")
                .to_string();
            assert!(
                error.contains("a token count is at most"),
                "{refused}: {error}"
            );
        }

        let counted = script(json!({ "input_tokens": MAX_TOKENS, "output_tokens": 0 }))
            .expect("the largest pair a run could report");
        let Outcome::Reply(reply) = counted.outcome else {
            panic!("a reply is a reply");
        };
        assert_eq!(
            reply.usage.expect("scripted counts").input_tokens,
            MAX_TOKENS
        );
    }

    /// The same bound at the Rust spelling, where a struct literal is the only
    /// way past it.
    #[test]
    #[should_panic(expected = "a scripted token count is at most 1000000000")]
    fn an_oversized_token_count_is_refused_by_the_builder() {
        let _ = Usage::new(MAX_TOKENS + 1, 1);
    }

    /// A `raw` status HTTP cannot carry is refused at both spellings, for the
    /// same reason `times` is: the fallback for a status that cannot be built is
    /// **500**, which PRD 5.9 fails over on and both SDKs retry — so a typo
    /// would stage a provider failure the script never asked for, and the run
    /// would report a failure whose shape does not match its cause.
    #[test]
    fn a_raw_status_outside_what_http_carries_is_refused_by_the_control_plane() {
        for status in [0, 42, 99, 1000, 65_535] {
            let refused = serde_json::from_value::<Script>(json!({
                "model": "fast",
                "outcome": { "raw": { "status": status, "body": {} } },
            }))
            .expect_err("a status HTTP cannot carry is not a script");
            assert!(
                refused.to_string().contains("HTTP can carry"),
                "{status}: {refused}"
            );
        }

        // The ones it can carry are queued exactly as before.
        for status in [200, 402, 429, 503] {
            serde_json::from_value::<Script>(json!({
                "model": "fast",
                "outcome": { "raw": { "status": status, "body": {} } },
            }))
            .unwrap_or_else(|error| panic!("{status} is sendable: {error}"));
        }
    }

    /// The same bound at the Rust spelling, where a struct literal is the only
    /// way past it.
    #[test]
    #[should_panic(expected = "must be one HTTP can carry (100..=999), not 42")]
    fn a_raw_status_outside_what_http_carries_is_refused_by_the_builder() {
        let _ = Outcome::raw(42, json!({}));
    }

    /// A surface is spelled on the wire the way grammar 12.1 spells the kinds
    /// that reach it, and [`Surface::as_str`] says the same thing — the two
    /// pinned together because a harness filtering `surface === "openai"` reads
    /// one of them and a Rust caller reads the other.
    #[test]
    fn a_surface_is_spelled_the_same_way_everywhere() {
        for (surface, spelling) in [
            (Surface::Anthropic, "anthropic"),
            (Surface::OpenAi, "openai"),
            (Surface::AzureOpenAi, "azure_openai"),
        ] {
            assert_eq!(surface.as_str(), spelling);
            assert_eq!(
                serde_json::to_value(surface).expect("a surface serializes"),
                json!(spelling),
                "the control plane and `as_str` must not drift apart"
            );
        }
    }

    /// Two queues of enormous entries add up to a number, not to a panic — and
    /// the snapshot is what `reset` computes *before* it can clear anything, so
    /// a panic here would leave the store unrecoverable.
    #[test]
    fn an_enormous_queue_depth_saturates_rather_than_overflowing() {
        let mut state = State::default();
        let queue = state.queues.entry("wide".to_string()).or_default();
        for _ in 0..3 {
            queue.push_back(Script {
                model: "wide".to_string(),
                outcome: Outcome::text("hi"),
                times: u32::MAX,
                matcher: Match::Any,
            });
        }
        assert_eq!(Store::snapshot_of(&state).queues["wide"], u32::MAX);
    }

    /// A `times: 0` that reached the queue anyway — the field is public — serves
    /// once and leaves, rather than wrapping a `u32` and panicking the
    /// connection task into something a client reads as a provider timeout.
    #[test]
    fn a_zero_times_entry_cannot_underflow_the_take() {
        let store = Store::new();
        store.enqueue(Script {
            model: "fast".to_string(),
            outcome: Outcome::text("once"),
            times: 0,
            matcher: Match::Any,
        });
        assert!(matches!(take(&store, "fast", "{}"), Decision::Serve(_)));
        assert!(matches!(
            take(&store, "fast", "{}"),
            Decision::Unscripted { .. }
        ));
    }

    /// The control-plane spelling of a matcher, both ways: a narrowed script
    /// reads `"match": {"body_contains": …}`, and one that narrows nothing does
    /// not mention `match` at all.
    #[test]
    fn a_matcher_round_trips_through_the_control_plane() {
        let narrowed: Script = serde_json::from_value(json!({
            "model": "fast",
            "outcome": { "reply": { "body": { "text": "hi" } } },
            "match": { "body_contains": "a draft" },
        }))
        .expect("the control-plane spelling of a matcher");
        assert!(narrowed.matcher.accepts("{\"draft\":\"a draft\"}"));
        assert!(!narrowed.matcher.accepts("{\"draft\":\"another\"}"));

        assert_eq!(
            serde_json::to_value(&narrowed).expect("a script serializes")["match"],
            json!({ "body_contains": "a draft" }),
            "what a harness reads back is what it wrote"
        );
        let plain = Script::new("fast", Outcome::text("hi"));
        assert!(
            serde_json::to_value(&plain).expect("a script serializes")["match"].is_null(),
            "an entry that narrows nothing does not mention `match`"
        );

        let error = serde_json::from_value::<Script>(json!({
            "model": "fast",
            "outcome": { "reply": { "body": { "text": "hi" } } },
            "match": { "body_contian": "a draft" },
        }))
        .expect_err("a typo in the matcher is not a matcher")
        .to_string();
        assert!(error.contains("body_contian"), "{error}");
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
    }

    /// …and a failure with nowhere to carry one says so where it was written.
    ///
    /// The three refusal shapes are rendered from the surface's error envelope
    /// and have no delay field, so accepting the call would mean dropping the
    /// wait: a test staging "the first route member rate-limits 300ms into a
    /// fan-out" would get an instant 429 and assert about an ordering that never
    /// happened. Refused the way a `raw` outcome's impossible status is.
    #[test]
    #[should_panic(expected = "`failure.rate_limit` has no wait to carry")]
    fn a_delay_on_a_failure_that_cannot_carry_one_is_refused_by_the_builder() {
        let _ = Outcome::rate_limit().after(Duration::from_millis(300));
    }

    /// The panic names what to script instead, because the two alternatives are
    /// what make the refusal a redirection rather than a dead end.
    #[test]
    fn the_refusal_names_the_two_ways_to_stage_a_late_failure() {
        let panicked = std::panic::catch_unwind(|| {
            let _ = Outcome::overloaded().after(Duration::from_millis(1));
        })
        .expect_err("`after` refuses an overload");
        let message = panicked
            .downcast_ref::<String>()
            .expect("the panic carries its message");
        assert!(message.contains("failure.overloaded"), "{message}");
        assert!(message.contains("Outcome::timeout(delay)"), "{message}");
        assert!(
            message.contains("Outcome::raw(status, body).after(delay)"),
            "{message}"
        );

        // The other half of the promise: a late refusal really is scriptable
        // that way, and the delay reaches the outcome that answers it.
        let Outcome::Raw(raw) =
            Outcome::raw(429, json!({ "error": "slow down" })).after(Duration::from_millis(300))
        else {
            panic!("a raw outcome stays raw");
        };
        assert_eq!(raw.status, 429);
        assert_eq!(raw.delay.duration(), Duration::from_millis(300));
    }
}
