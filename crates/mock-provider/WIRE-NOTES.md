# WIRE-NOTES

What this server believes about the two provider wire surfaces, split into what
is **certain** (the shapes it would be a bug to get wrong) and what is
**assumed** — written down here because it was fixed from documentation and
client-library knowledge rather than from a live call, and the first real
end-to-end run is what confirms or corrects it.

The point of the split is that a wrong assumption should fail *loudly and once*:
each numbered assumption below says what would confirm it and what breaks if it
is wrong, so the first live e2e can walk this list.

> Scope note: this file is about the **wire**. What the compiler accepts is
> `docs/grammar.md`; what a compiled graph should send is codegen's problem. An
> assumption here that turns out to be wrong is a change to this crate, not to
> the DSL.

---

## Out of scope in v0

**`bedrock` and `vertex` are not served.** Grammar 12.1 lists six provider kinds;
four of them speak an HTTP API this server can stand in for, and two do not:

| kind | reached by | in the mock |
|---|---|---|
| `anthropic` | `POST /v1/messages` | yes |
| `openai` | `POST /v1/chat/completions` | yes |
| `openai_compatible` | the same route at another `base_url` | yes |
| `azure_openai` | `POST /openai/deployments/<deployment>/chat/completions`, or `POST /openai/v1/chat/completions` | yes |
| `bedrock` | AWS SDK (SigV4, `bedrock-runtime` endpoints) | **no** |
| `vertex` | Google Cloud SDK (OAuth2 service-account credentials) | **no** |

The two SDK-reached kinds take neither `base_url` nor `headers` in the grammar,
"because a connection made through a cloud SDK has no bare endpoint to point at"
(grammar 12.1) — which is exactly why the harness cannot redirect one at itself.
Standing in for them means either signing SigV4/OAuth2 requests or injecting a
fake SDK transport into generated TypeScript, and both are a different project
from this one. A fixture project that needs a mocked model therefore uses
`anthropic` or `openai_compatible`.

**Streaming is not served.** Both surfaces refuse `stream: true` with a
validation failure naming this file. A v0 compiled graph invokes a model and
waits for structured output (PRD 5.2); nothing in M1 streams. A server that
answered a streaming request with a non-streaming body would be teaching the
generated client something false about the provider.

**Embeddings are not served.** `store.docs`-style vector stores name an
`embed.provider` (grammar 11.2), and M1 backs stores with SQLite/local disk. When
the embedding call becomes real, `POST /v1/embeddings` is an additive route here.

---

## Certain

These are load-bearing and pinned by tests in `src/` and `tests/`:

* **Anthropic requires `max_tokens`**, `model`, and `messages`; roles alternate
  and the first message is `user`.
* **`tool_result` blocks lead the user turn that answers `tool_use` blocks**, and
  every `tool_use` id must be answered in the *next* message.
* **Anthropic's overload status is 529**, and its rate-limit status is 429 with a
  `retry-after` header; the error envelope is
  `{"type":"error","error":{"type":…,"message":…}}`.
* **OpenAI tool-call arguments travel as a JSON-encoded string**, not an object,
  under `choices[].message.tool_calls[].function.arguments` and in the assistant
  messages a client echoes back.
* **A `tool` role message carries `tool_call_id`** and must answer a call from a
  preceding assistant message.
* **OpenAI's error envelope is `{"error":{"message","type","param","code"}}`**,
  and unknown request arguments are refused rather than ignored.
* **Azure requires an `api-version` query parameter** and authenticates with an
  `api-key` header (a bearer token is also accepted, for AAD).
* **A pinned tool choice guarantees a call.** Anthropic's `tool_choice: {type:
  "tool", …}` and `{type: "any"}`, OpenAI's forced function and `tool_choice:
  "required"`, and OpenAI's `response_format: {type: "json_schema"}` each make
  free prose an impossible answer. A `text` reply scripted against one of them
  is refused as a `script-mismatch` for the same reason a `structured` reply to
  a request that pinned nothing is: the harness must not teach generated code
  that a provider answers in a way it cannot.
* **`tool_choice` requires `tools`**, on both surfaces, and **OpenAI refuses an
  empty `tools` array** (`Invalid 'tools': empty array. Expected an array with
  minimum length 1.`) — which is what a compiled graph sends for an agent with
  neither `tools:` nor `stores:` if it always writes the key.

---

## Assumed

### 1. LangChain JS asks Anthropic for structured output with a forced tool

`withStructuredOutput(schema)` on `ChatAnthropic` is understood to bind the
schema as a single tool and pin `tool_choice: {type: "tool", name: <name>}`,
then read the structured object out of the returned `tool_use` block's `input`.

*This server*: accepts exactly that, and renders `Outcome::structured(v)` as one
`tool_use` block. It also accepts `tool_choice: {type: "any"}` when exactly one
tool is on offer, which is the same request in effect.
*Confirmed by*: the first live e2e's recorded request carrying
`tool_choice.type == "tool"`.
*If wrong*: a structured reply is refused as a `script-mismatch` (see (11) for
the status, header `x-mock-provider-error`), which is loud and points here.

### 2. The default tool name for a structured output is not fixed

LangChain's default extraction tool has been `extract` in some versions and the
schema's own name in others. This server does not care: it reads the name from
`tool_choice` and answers under it.
*If wrong*: nothing — the name is echoed, never assumed. Named here so a reader
does not add a check for a specific one.

### 3. LangChain JS has two ways to ask OpenAI for structured output

Function calling (a forced function) and `response_format: {type: "json_schema",
json_schema: {name, schema, strict}}`, selected by a `method` option.
*This server*: accepts both, and renders `Outcome::structured(v)` the way the
request asked — serialized into `message.content` for `json_schema`, or as a
tool call whose `arguments` is the serialized object for a forced function.
*If wrong*: as (1), a `script-mismatch`.

### 4. `anthropic-version` is required and `2023-06-01` is what clients send

The Messages API refuses a request with no `anthropic-version` header. The value
itself is not checked here — only its presence — because a client library pins
its own and a mock that pinned a different one would fail every real request.
*Confirmed by*: a live request's headers in the transcript.

### 5. The exact `type`/`code` strings on an OpenAI 429

This server sends `{"message": …, "type": "requests", "param": null, "code":
"rate_limit_exceeded"}`. Observed OpenAI 429s have used `type: "requests"` with
`code: "rate_limit_exceeded"` and, for quota exhaustion, `type:
"insufficient_quota"`.
*Why it may not matter*: generated code classifies failover conditions by
**status** (429 → `rate_limit`), not by these strings, and the client libraries
raise their error classes from the status too.
*If wrong*: only a test that asserts on the string breaks — `tests/openai_wire.rs`
is the one place that does.

### 6. OpenAI's overload status is 503

Anthropic's is unambiguously 529; OpenAI's overload/capacity answer is taken here
to be 503 with `type: "server_error"`. Some deployments answer 429 with an
"engine overloaded" message instead.
*If wrong*: a graph classifying overload from 503 alone would also need the 429
path, which it already has. The harness can script either.

### 7. The Azure route forms

Classic: `POST /openai/deployments/<deployment>/chat/completions?api-version=…`,
where the deployment names the model and the body's `model` is optional. Newer:
`POST /openai/v1/chat/completions?api-version=preview`, where the body names the
model. Both are served; when a body names a model it outranks the path, so one
deployment can host several scripted model ids.
*Confirmed by*: a live Azure-backed run's recorded `path` and `model`.

### 8. `max_tokens` is always present on an Anthropic request

The API requires it, and the JS `ChatAnthropic` integration defaults it (2048 at
time of writing) rather than omitting it. This server refuses a request without
one.
*If wrong* (a generated client omits it): the transcript shows
`max_tokens: Field required`, which is the correct answer either way — a real
call would be refused too.

### 9. Response fields no client reads are still worth sending

`stop_sequence: null`, `system_fingerprint: "fp_mock"`, `logprobs: null`,
`created`, `request-id` / `x-request-id` headers. They are cheap and their
absence is the kind of thing a strict client library trips over.
*Note*: `created` is the **frozen** constant `1700000000` (see
`control::CREATED`), because a golden transcript must not carry the wall clock.
A client that rejects a stale timestamp would be a real difference from a live
provider — no client is known to.

### 10. Token counts may be estimated

Usage defaults to a four-characters-to-the-token estimate over the serialized
request and response. It is not a tokenizer. A test that needs exact numbers
scripts them (`usage` on a reply).

### 11. The harness's own refusals use a status no provider sends and no SDK retries

An unscripted call, a script that cannot be rendered, and a control-plane
document that will not parse all answer **422** with an
`x-mock-provider-error` header, wrapped in the surface's own error envelope so a
client library can still parse them. Two mechanisms had to be dodged, not one:

* **failover.** PRD 5.9 routes on 429/5xx/no-answer, so a harness refusal
  wearing one of those would be silently failed over and the run would report a
  different failure than the one it had.
* **SDK-level retry.** `@anthropic-ai/sdk` and `openai` (node) both retry
  **408, 409, 429 and 5xx** by default, twice — so 409, the first choice here,
  would have produced three transcript entries and three `unscripted` counts
  for one call. 422 is in neither set.

The envelope's `type` is the surface's `invalid_request_error` rather than an
invented one, because an unknown error type is what a client library least
reliably parses.
*Confirmed by*: the first live e2e — one refused call producing exactly one
transcript entry.
*If wrong* (some client retries 422s): the transcript still shows the refusal,
and the run fails with the queue empty rather than passing.

### 12. Request-header checks are presence checks

`x-api-key`, `authorization: Bearer …`, `api-key`, `content-type:
application/json`. Values are never compared: the harness needs **no API keys**
(PRD §7 M1), so any non-empty placeholder passes. What is being checked is that
generated code sends the header at all, which a live call would otherwise be the
first to discover.

### 13. OpenAI's strict-mode schema rules, and the sentences it refuses with

`strict: true` — on `response_format.json_schema` and on a function tool — is
not a hint: the service compiles the schema into a constrained decoder and
refuses one it cannot close. Every object must carry `additionalProperties:
false` and list every one of its `properties` in `required`, all the way down.
This server enforces it, and words the refusal the way the service does:

```
Invalid schema for response_format 'reviewer_output': In context=(), 'additionalProperties' is required to be supplied and to be false.
Invalid schema for function 'lookup': In context=('properties', 'author'), 'required' is required to be supplied and to be an array including every key in properties. Missing 'name'.
```

*What is certain*: the rules. They are the documented condition for structured
outputs, and this is the most common 400 on the surface — a Zod-to-JSON-Schema
path that drops either key passes a mock that does not check and fails on the
first live call.
*What is assumed*: the exact sentences, and the `context=` path spelling (a
Python tuple of the schema keys walked through). The recursion follows
`properties`, `items`, `$defs` and `anyOf`, and reports one missing-`required`
complaint per object, naming the first property left out — the service reports
one at a time.
*If wrong*: only a test asserting on the string breaks;
`src/openai.rs`'s `a_strict_schema_must_close_every_object_and_require_every_property`
is where the strings live.

### 14. `tool_choice` without `tools` is refused on both surfaces

The mirror of the rule tool *blocks* obey ("Requests which include `tool_use` or
`tool_result` blocks must define tools"): a request that says how the model must
use its tools has to have some. Refused once — at `tools` on the Messages
surface, at `tool_choice` on Chat Completions — rather than by complaining that
the forced name is not in a list that does not exist.
*Assumed*: the wording, on both. The rule itself is not in doubt; the sentences
are this server's own phrasing in each API's idiom.
*If wrong*: nothing a compiled graph does — codegen has no reason to emit a bare
`tool_choice` — so the cost of being wrong is a message, not a verdict.

---

## Accepted-key lists are curated, not exhaustive

Both surfaces refuse unknown top-level keys, checked against a list in
`src/anthropic.rs` (`REQUEST_KEYS`) and `src/openai.rs` (`REQUEST_KEYS`). The
lists cover every key a compiled graph could send today, including grammar
12.2's `settings:` vocabulary. A provider adding a parameter, or codegen starting
to send one, means adding a line — and until that line exists the request is
refused with the provider's own "extra inputs are not permitted" / "unrecognized
request argument" message. That is the intended failure mode: a silently accepted
unknown key is how a setting stops taking effect without anyone noticing.

**The OpenAI list is the union of three provider kinds, not `api.openai.com`.**
One route serves `openai`, `openai_compatible` and `azure_openai`, and the body
carries nothing that distinguishes them, so the list has to admit any key *any*
of the three accepts. `top_k` is the case that forces the choice: grammar 12.2
lists it in the typed `settings:` vocabulary, vLLM and ollama accept it, and the
`agent-openai` fixture's provider is `kind: openai_compatible` — so it is
accepted here even though `api.openai.com` answers 400. The cost is one key this
server will not catch for a graph pointed at OpenAI proper; the alternative is
refusing a request that is correct for the kind the fixture declares, which the
harness would have no way to tell apart. The Anthropic list has no such
ambiguity and carries `top_k` because the Messages API takes it.
