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

**Embeddings are served, and are the one route that is not scripted.**
`store.docs`-style vector stores name an `embed.provider` (grammar 11.2), so a
`vector` op is a store op with a real HTTP round trip inside it — and unlike a
model call, what comes back is not a decision the graph makes. A test asserts
about the *search*, never about the vector, so a scripted queue here would make
every store test enqueue answers it never reads.

`POST /v1/embeddings` (and its Azure spelling) therefore answers without a
script, **deterministically in the text**: a small vector — eight dimensions,
because nothing here is a language model — derived from the input's lowercased
words, so identical texts embed identically, different texts embed differently,
and a text scores higher against its own document than against an unrelated one.
That is exactly the property a `search` assertion needs and it holds with no
model in the loop. The *request* is still held to the surface's shape — a
`model`, and an `input` list of strings — because a compiled graph that sent
something else would be a codegen bug this harness exists to catch. A fixture
that declares `embed.dimensions:` declares **8**, since grammar 11.2 asserts the
declared width against what the provider answered.

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
* **Azure's classic deployment route requires an `api-version` query
  parameter**, and both Azure routes authenticate with an `api-key` header (a
  bearer token is also accepted, for AAD). The newer `/openai/v1/...` route makes
  `api-version` optional — see (7), where the route forms live.
* **A pinned tool choice guarantees a call.** Anthropic's `tool_choice: {type:
  "tool", …}` and `{type: "any"}`, OpenAI's forced function and `tool_choice:
  "required"`, and OpenAI's `response_format: {type: "json_schema"}` each make
  free prose an impossible answer. A `text` reply scripted against one of them
  is refused as a `script-mismatch` for the same reason a `structured` reply to
  a request that pinned nothing is: the harness must not teach generated code
  that a provider answers in a way it cannot.
* **A pinned tool choice outranks a `response_format`.** The consequence of the
  rule above on a Chat Completions request that carries *both* mechanisms:
  `response_format` shapes the content, and a pinned `tool_choice` decides
  whether the turn has content at all, so the answer is the call and
  `finish_reason: "tool_calls"`. A `structured` reply is therefore rendered as
  the pinned call's `arguments`, never into the content — answering one of these
  with `stop` and no `tool_calls` would hand a codegen path that emitted both
  mechanisms (3) a prose branch to pass on and a tool call on its first live
  request. `tool_choice: "required"` pins a call without naming the function to
  make it under, so a `structured` reply to one is refused as a
  `script-mismatch`: the script has to name the function, with `tools` or by
  forcing one.
* **A tool choice pinned by *name* guarantees a call to *that* tool.** The
  stronger half of the same rule, and the one an agent with tools makes
  reachable: `tool_choice: {type: "tool", name: X}` and a forced function both
  offer the model no other move, so a `tools` reply naming a **sibling** tool the
  request also offered — legal-looking, since the tool is on the surface — is
  refused as a `script-mismatch` too. (`{type: "any"}` and `"required"` pin that
  *some* tool is called and not which, so any offered tool is fair game there.)
* **Both APIs check a known key's type and range, not just its name.**
  `temperature: "hot"`, `top_p: 5`, `n: "2"` and `stream: "true"` are 400s, not
  ignored settings, and so is an empty Anthropic text block. This is the runtime
  half of grammar 12.2: the compiler range-checks a `settings:` block against the
  literal in the spec, and nothing else looks at what a template or a codegen bug
  actually serializes. The exact sentences are (15).
* **A string is one text block, and the empty one is refused as such.** The
  Messages API takes a string wherever it takes a list of text blocks —
  `messages[].content`, `system` — as shorthand for a single block, and it
  addresses its complaint at the **normalised** block: `content: ""` is refused
  at `messages.0.content.0.text`, an address the request never spelled. That
  matters because the string is the spelling a compiled graph sends: a prompt or
  a bound input that rendered to nothing arrives as `""`, not as an explicit
  empty block, so a rule that only reached the block form would miss every real
  instance of the bug it was written for. The same rule reaches the text parts
  of a `tool_result`. (An **absent** `tool_result` content, and the empty string
  in that one position, stay legal: a tool that returned nothing says so that
  way, and the API makes the key optional. `system: ""` is not that case —
  grammar 5.4 requires a non-empty `prompt:`, so no correct composition
  produces one.)
* **`tool_choice: none` forbids one**, which is the same rule read backwards, so
  a `tools` reply scripted against it is refused the same way. So is a `tools`
  reply carrying **no calls**: `stop_reason: "tool_use"` / `finish_reason:
  "tool_calls"` names the block that ended the turn, and neither API sends the
  name with nothing behind it. Prose is `text`; a shape generated code must
  reject is `raw`.
* **A scripted stop reason is held to the same rule as the body it rides on.**
  `stop_reason` / `finish_reason` is the one field a compiled agent's tool loop
  branches on, so a `reply` may override it only with a value the surface really
  sends *and* one the body can carry — see (16). Everything else, `raw`.
* **A missing credential is a 401.** `x-api-key` on the Messages API,
  `Authorization: Bearer …` / `api-key` on Chat Completions — absent, the answer
  is 401 (`authentication_error` there, `code: "invalid_api_key"` here), not the
  400 a malformed body draws, because 401 is the status both SDKs raise
  `AuthenticationError` from. Authentication is settled before the body is, so
  the refusal names the credential and nothing else — while the transcript still
  records every failure the request had. See (12) for what "credential" means
  here, and (11) for why 401 is safe: neither the failover set nor the SDK retry
  set claims it.
* **`tool_choice` requires `tools`**, on both surfaces, and **OpenAI refuses an
  empty `tools` array** (`Invalid 'tools': empty array. Expected an array with
  minimum length 1.`) — which is what a compiled graph sends for an agent with
  neither `tools:` nor `stores:` if it always writes the key. An assistant
  message's **`tool_calls: []`** is the same mistake one message lower, sent by
  a client that always writes the key when it echoes history, and it is refused
  with the same sentence at `messages.N.tool_calls`.
* **An assistant turn with an empty content list is refused.**
  `messages.N.content: List should have at least 1 item` — the Messages API's
  own sentence, and the shape a tool loop sends when it *rebuilds* its assistant
  turn from the text and tool calls it read rather than replaying what the model
  sent. An answer cut short at `max_tokens`, or one that was all `thinking`,
  reduces to nothing under that reading; a compiled graph must stop on the empty
  answer instead, and this is the rule that says which of the two happened.

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
tool call whose `arguments` is the serialized object for a forced function. A
request that carries **both** is answered by the tool pin (the Certain list,
above), because a codegen path confusing the two is a live risk and the harness
must not be the thing that hides it. A forced function that declares no
`parameters` takes no arguments, so a `structured` reply to one is refused as a
`script-mismatch` naming the missing `parameters` — the function has to carry
the agent's output schema for there to be an object to answer with.
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
where the deployment names the model, the body's `model` is optional, and
`api-version` is **required**. Newer: `POST /openai/v1/chat/completions`, where
the body names the model and `api-version` is **optional** — the v1 API needs it
only to opt into preview features. Both are served; when a body names a model it
outranks the path, so one deployment can host several scripted model ids.

*This server*: enforces `api-version` on the classic route only. Refusing it on
the v1 route would fail a fixture that is correct, which is the same class of bug
as accepting a request the service refuses — so the two routes are told apart
(`openai::Route::Azure` and `Route::AzureV1`) rather than merged.

The classic route is therefore the **only** place a body without `model` is
accepted: on `/openai/v1/...` there is no deployment to stand in for it, so an
omitted `model` is refused exactly as it is on the direct route. Accepting it
would key every such call under the empty string — one queue for a whole run, and
a codegen bug that dropped `model` reported as a confusing "no scripted outcome
for model ``".
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
`created`, `request-id` / `x-request-id` headers, and the Messages surface's
`usage.cache_creation_input_tokens` / `usage.cache_read_input_tokens` (both `0`,
because nothing here is cached — they are on every live response whether or not
the request asked for caching). They are cheap and their absence is the kind of
thing a strict client library trips over.

The argument applies to **errors as well as answers**, which is where it is
easiest to forget: both APIs return a request id on every response, and it is
what an SDK's error object surfaces and what support tooling asks for. So every
answer this server generates carries one derived from the call's arrival
sequence — 200s, scripted failures, refused requests and harness refusals alike —
and the Messages surface's error envelope carries the `request_id` member it has
in addition to the header. The one exception is a `raw` outcome, which is served
verbatim: a script that wants headers there writes them.
*Note*: `created` is the **frozen** constant `1700000000` (see
`control::CREATED`), because a golden transcript must not carry the wall clock.
A client that rejects a stale timestamp would be a real difference from a live
provider — no client is known to.

### 10. Token counts may be estimated

Usage defaults to a four-characters-to-the-token estimate over the serialized
request and response. It is not a tokenizer. A test that needs exact numbers
scripts them (`usage` on a reply), and a scripted count is bounded at a billion
— `control::MAX_TOKENS` — because the counts are arithmetic this server does
(Chat Completions serves their **sum** as `total_tokens`), because an overflow
in a render is a dropped connection and PRD 5.9 reads that as a provider
timeout, and because the client reading them is generated TypeScript, which
represents integers above 2^53 - 1 only approximately. The widest context window
in service is three orders of magnitude below the bound.

### 11. The harness's own refusals use a status no provider sends and no SDK retries

An unscripted call, a script that cannot be rendered, a script whose `raw` status
or headers cannot be put on the wire, and a control-plane document that will not
parse all answer **422** with an `x-mock-provider-error` header, wrapped in the
surface's own error envelope so a client library can still parse them. Two
mechanisms had to be dodged, not one:

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

Every one of them is **recorded as a refusal**, which matters most for the two
that happen after a scripted outcome has already been taken from its queue: a
`script-mismatch`, and a `raw` outcome that cannot be put on the wire. The
transcript's `served` names the refusal (`script-mismatch`,
`unsendable-response`) rather than the outcome that was taken, and
`/_mock/state` counts it under `refused`, so `is_drained()` is false for a run
that was refused. In an e2e run the 422 goes to the generated process, so
without that the Rust side would see a transcript claiming a reply was served,
a clean snapshot, and a graph that failed for no visible reason.
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

The three **credential** headers answer differently from the fourth. A missing
credential is 401 and a missing or wrong `content-type` is 400 — the split the
Certain list states, and the one both SDKs classify on (`AuthenticationError` is
raised from the status alone). *Assumed*: that a **present but placeholder** key
would also pass a live call, which is the whole basis of a keyless harness, and
the exact error `code` on the Chat Completions 401 (`invalid_api_key`; a live
missing-key 401 may carry `code: null`). *If wrong*: only a test asserting the
string breaks — `tests/openai_wire.rs`'s
`a_request_without_credentials_is_refused` is where it lives.

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

The **root** of a schema is held to two further rules, which apply whether or
not `strict` was asked for — see (16).

*What is certain*: the rules. They are the documented condition for structured
outputs, and this is the most common 400 on the surface — a Zod-to-JSON-Schema
path that drops either key passes a mock that does not check and fails on the
first live call.

The recursion follows `properties`, `items`, `anyOf`, and **both** spellings of
the definitions bucket — `$defs`, which is JSON Schema 2020-12's and what
OpenAI's examples show, and `definitions`, which is draft-07's and
`zod-to-json-schema`'s **default** `definitionPath`. The second is the one a Zod
model with a reused or recursive sub-schema actually lands on, so a walk that
knew only `$defs` would accept the unclosed object under the likelier spelling
and 400 on the first live call — the outcome this check exists to prevent. Every
entry in the bucket is walked rather than only the ones a `$ref` reaches: an
object refused when referenced is refused when merely declared, and a `$ref` walk
would have to resolve pointers and guard cycles to arrive at the same objects.

*What is assumed*: the exact sentences, and the `context=` path spelling (a
Python tuple of the schema keys walked through). One missing-`required` complaint
is reported per object, naming the first property left out — the service reports
one at a time.
*If wrong*: only a test asserting on the string breaks;
`src/openai.rs`'s `a_strict_schema_must_close_every_object_and_require_every_property`
and `the_strict_walk_reaches_both_spellings_of_the_definitions_bucket` are where
the strings live.

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

### 15. The sentences a bad `settings:` value is refused with

*What is certain*: the rules. Both services check a known key's **type** and its
**range**, and neither ignores a key it knows but cannot read — `temperature:
"hot"`, `temperature: -3`, `top_p: 5`, `n: "2"`, `stream: "true"` and an empty
Anthropic text block are all 400s. This server enforces them, in each dialect:

```
temperature: Input should be a valid number
temperature: Input should be less than or equal to 1
messages.0.content.0.text: text content blocks must be non-empty
Invalid type for 'temperature': expected number, but got string instead.
5 is greater than the maximum of 2 - 'temperature'
```

*What is assumed*: the exact sentences, and the ranges where the three kinds
behind the Chat Completions route disagree — `temperature` is bounded at 2 there
because that is the widest of them, and the list is a union for the same reason
`top_k` is on it (see *Accepted-key lists*, below). The empty-text sentence is
also the one used for the **string** spelling of a `content` or a `system` (the
Certain list, above): the refusal is not in doubt, but the Messages API is also
known to word an empty turn as `messages: all messages must have non-empty
content except for the optional final assistant message`, so the first live e2e
that sends one is what says which sentence and which address it uses.
*If wrong*: only a test asserting on the string breaks —
`src/anthropic.rs`'s `the_sampling_knobs_are_checked_for_type_and_range` and
`src/openai.rs`'s namesake are where they live. A range that is too *narrow*
would be worse than a wrong sentence, because it refuses a request that is
correct; that is why the widest of the three kinds is the one enforced.

### 16. OpenAI's root-schema rules

Structured Outputs generates against a JSON **object**, so the root of a declared
schema must be `type: "object"` and must not be an `anyOf`. Both places a schema
is declared are held to it — `response_format.json_schema.schema` and a function
tool's `parameters` — and at any `strict`, because the rules are about what the
model is asked to produce rather than about how tightly the decoder is
constrained. Below the root, `anyOf` is ordinary.

```
Invalid schema for response_format 'reviewer_output': schema must be a JSON Schema of 'type: "object"'.
Invalid schema for function 'lookup': 'anyOf' is not permitted at the root level of the schema.
```

*What is certain*: the rules, and that codegen can reach the second one.
`zod-to-json-schema` renders a `z.discriminatedUnion` as a bare root `anyOf`, and
PRD §7 M1 promises "state models (incl. tagged unions via Zod)" — so a path that
puts a tagged union at an agent's output root, or unwraps a single-field output
to its bare field schema, produces exactly the request the service refuses. The
union has to be one level down, under a property.
*What is assumed*: the exact sentences. The first is the one this server already
used for a function tool's `parameters`; the second is this server's own phrasing
of the documented rule.
*If wrong*: only a test asserting on the string breaks — `src/openai.rs`'s
`a_declared_schemas_root_must_be_an_object_and_not_an_any_of` is where the
strings live.

### 17. The stop reasons each surface sends, and what they say about the body

A `reply` may override the stop reason its body implies, and the override is held
to the surface's closed set and to the body it accompanies — the same doctrine as
every other part of a `reply`: it is what the API could have sent, and anything
else is `raw`.

| surface | closed set |
|---|---|
| Messages | `end_turn`, `max_tokens`, `stop_sequence`, `tool_use`, `pause_turn`, `refusal` |
| Chat Completions | `stop`, `length`, `tool_calls`, `content_filter` |

The coherence rules are three: `tool_use` / `tool_calls` requires content that
carries a call (the sentence an empty `tools` reply is already refused with);
`end_turn` / `stop` is refused *over* such content, because the calls are what
ended that turn; and `stop_sequence` requires a request that declared
`stop_sequences`, since the API only matches ones it was given — the first is
then named in the response's `stop_sequence` member, which is `null` in every
other case. `max_tokens` / `length`, `content_filter`, `pause_turn` and `refusal`
cut either body and are legal over both.

*What is certain*: that these are the fields a compiled agent's tool loop
branches on, and that a mock which served an impossible pairing would let a
codegen PR watch its loop take a branch and pass a criterion no live call could
have produced.
*What is assumed*: the closed sets themselves — a surface that adds a stop reason
means adding a line, and until it exists the reason is refused as a
`script-mismatch` (the same intended failure mode as the accepted-key lists
below). Chat Completions' deprecated `function_call` is deliberately left out:
this server does not serve the `functions` request surface, so a choice naming it
would name a member no answer here has.
*If wrong*: a test scripting the missing reason is refused loudly, with the set
in the message; nothing is served that a provider would not.

### 18. How a tool result says it is an **error**, on each surface

A compiled graph answers a tool call the tool's contract refused by handing the
model the refusal rather than ending the node (grammar D119), so it sends a tool
result that is not a result. The two surfaces spell that differently, and one of
them does not spell it at all:

| surface | how the refusal travels |
|---|---|
| Messages | the ordinary `tool_result` block with `is_error: true` beside its `content` — the key this server already accepts as an optional boolean |
| Chat Completions | the ordinary `tool` role message, whose text *is* the refusal. There is nothing to set: the message is closed to `role`, `content` and `tool_call_id` |

*What is certain*: that the block or message has to be **sent at all**. Both
surfaces refuse a request that leaves a `tool_use` id or a `tool_call_id`
unanswered, and this server checks both directions (`check_messages` in
`src/anthropic.rs`, `an_orphaned_tool_message_is_refused` in `src/openai.rs`) —
so a runtime that answered only the calls that worked would be caught here on its
next request rather than in production.

*What is assumed*, and it is one thing: that a **`tool_use` naming a tool the
current request's `tools` does not declare stays legal in the history.** It is
reachable exactly once — a model answered with a name the agent never offered,
the loop refused it, and the assistant turn carrying that `tool_use` is replayed
because the Messages API requires a turn back unaltered. Both APIs are documented
to require `tools` to be *present* when the messages carry tool blocks, which is
the rule this server enforces; neither is documented to re-validate the names in
history against it. The alternative would be for a client to drop or rewrite the
model's own turn, which the Messages API forbids for a different reason
(`thinking` blocks must come back unaltered beside the `tool_use` they preceded).

*If wrong*: the first live run in which a model invents a tool name answers 400
on the request *after* the refusal, and the fix is this server growing the same
check plus a codegen decision about which turn to send. Nothing about a correct
composition changes: every other request carries only names the agent offered.

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
