# models

Two namespaces, split because the layers change for different reasons.
`provider.*` holds **connection**; `model.*` holds **behavior**. Agents
reference only `model.*` — which is what makes swapping the model behind a graph
one edit in one file.

```yaml spec
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

provider.local:
  kind: openai_compatible
  base_url: ${LOCAL_LLM_URL}
  api_key: ${LOCAL_LLM_KEY}

provider.gateway:
  kind: anthropic
  base_url: ${LLM_GATEWAY}
  headers:
    authorization: "Bearer ${PROXY_TOKEN}"

model.gateway:
  provider: provider.gateway
  id: claude-sonnet-4-5

model.smart:
  provider: provider.anthropic
  id: claude-sonnet-4-5
  settings:
    max_tokens: 8000
    thinking: { budget_tokens: 4000 }

model.fast:
  provider: provider.anthropic
  id: claude-haiku-4-5
  settings: { temperature: 0.2 }

model.default:
  route: [model.smart, model.fast]
  route_on: [rate_limit, overloaded, timeout]

agent.triage:
  model: model.default
  prompt: Classify the report.
  output:
    severity: { enum: [low, high] }

flow.triage:
  outputs: {}
  nodes:
    classify: { agent: agent.triage, input: "'a report'" }
  edges:
    - { from: start, to: classify }
    - { from: classify, to: end }
```

## Providers

Two keys are legal on every provider — `kind:` and `description:` — and every
other key belongs to the rows its `kind` names.

| `kind` | Required | Optional |
|---|---|---|
| `anthropic` | `api_key` — **or** a `base_url` naming the gateway that holds one | `api_key` (beside a `base_url`), `base_url`, `headers`, `server_tools` |
| `openai` | `api_key` — **or** a `base_url` naming the gateway that holds one | `api_key` (beside a `base_url`), `base_url`, `headers`, `organization`, `server_tools` |
| `openai_compatible` | `base_url` | `api_key`, `headers`, `server_tools` |
| `azure_openai` | `base_url`, `api_key`, `api_version` | `headers` |
| `bedrock` | `region` | `access_key_id`, `secret_access_key`, `session_token`, `profile` |
| `vertex` | `project`, `location` | `credentials_json` |

**A kind's row is closed.** A key belonging to another kind's row is a compile
error naming the key and the kind — not an ignored setting. `region:` on an
`anthropic` provider and `api_key:` on a `vertex` one are both refused, because
the plugin never reads them and accepting one would leave you believing a
connection setting is in effect that is not.

`bedrock` and `vertex` take neither `base_url` nor `headers`: a connection made
through a cloud SDK has no bare endpoint and no request the spec composes
headers onto. A deployment that genuinely needs either is reaching a compatible
HTTP endpoint, which is what `openai_compatible` is for.

## Keyless providers behind a gateway

`anthropic` and `openai` are the two kinds with a **default endpoint**: omit
`base_url:` and the connection reaches `https://api.anthropic.com` or
`https://api.openai.com`, where nothing but a key authenticates. So on those two
kinds `api_key:` is required when `base_url:` is absent and **optional when it is
present** — which is the shape a corporate deployment writes, where a gateway
injects the vendor credential server-side and nobody running the graph holds a
key:

```yaml
provider.gateway:
  kind: anthropic
  base_url: ${LLM_GATEWAY}
```

A provider declaring **neither** is `missing-credential`, and the message names
both repairs: add the key, or name the gateway.
`agent-compose explain missing-credential` prints the worked pair.

When the key is absent the compiled graph sends **no authentication header at
all** — no `x-api-key`, no `authorization` — rather than an empty one, so the
gateway sees a request that never claimed to authenticate. A gateway that wants
a token of its *own* takes it through `headers:`, whose values interpolate, which
is what the opening spec's `provider.gateway` shows:

```yaml
  headers:
    authorization: "Bearer ${PROXY_TOKEN}"
```

The other four kinds keep the rows they had. `azure_openai` reaches a
per-resource deployment with no default endpoint to fall back to, so all three
of its keys stay required; `openai_compatible` already paired an optional
`api_key` with a required `base_url`; and `bedrock` and `vertex` authenticate
through their cloud's own credential chain.

The **header** rule above still reaches one of them, because it is stated over
the connection rather than over the kind: an `openai_compatible` provider that
declares no `api_key:` — a local llama.cpp or ollama endpoint — now sends no
`Authorization` header at all, where before it sent an empty `Bearer `. Same fix
for the same reason, on a kind whose row did not move.

**Credentials are never literals.** `api_key`, `api_secret`, `token`,
`password`, `access_key_id`, `secret_access_key`, `session_token`,
`credentials_json`, `url`, `base_url`, `endpoint`, `dsn` take the env-ref value
form — the whole string is one `${NAME}` — and a literal there is
`invalid-env-ref`. `region`, `location`, `project`, `organization`, `profile`
and `api_version` are plain strings and may be interpolated.

Env refs survive **unresolved** into the artifact: `validate` and `build` check
syntax only, so a build succeeds in CI holding no keys. Presence is checked at
process start, and `run`/`serve` fail fast naming the missing variable.

## Server tools

A **server tool** runs on the provider's side, *inside* the model call: the
compiled graph dispatches nothing, and what the tool found arrives woven into
the assistant's turn. Web search is the one everybody meets first.

`server_tools:` is an array of config objects written in **that provider's own
wire vocabulary**, and the runtime appends it to the `tools` of every request the
connection serves, after the agent's own:

```yaml
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search
      max_uses: 5
      allowed_domains: ["docs.example.com"]
```

Every entry needs a `type:`, which is the vendor's own key for the tool and is
never interpolated. Everything else is the vendor's and travels verbatim, and —
like the rest of a provider's non-secret config — its **string** values may embed
`${ENV}` references. Numbers and booleans may not, and the two tiers below are
why: a field the compiler's table types is read at compile time, where there is
nothing to read, and interpolation produces a string, which is not what
`max_uses:` carries to the wire. `max_uses: ${SEARCH_BUDGET}` is therefore a
`type-mismatch` whose help says so.

One kind of string does not interpolate either, for the same reason read the
other way round: a field the table pins to a **single** value. The Messages
wire's `name:` is the one you will meet, and a nested object's `type:` — the
`approximate` of a `user_location:`, the `ephemeral` of a `cache_control:` — is
the same shape. That value is decided by the entry's own `type:` and the service
refuses a request that spells it otherwise, so `name: ${WEB_SEARCH_NAME}` is an
`unexpected-env-ref` naming the one value the field takes rather than a knob read
at process start. A closed set of *several* values is an ordinary interpolable
string: `search_context_size: ${SEARCH_DEPTH}` is a staging deployment searching
shallowly, and validates.

**The suite belongs to the connection.** Every agent whose model resolves to
that provider holds it; to give one agent a search and not another, define a
second provider. Providers are cheap.

**One name, one tool.** The suite lands in the same `tools` array as the agent's
own, and the provider surfaces refuse a request offering two tools under one
name — so a name spent twice is `tool-name-collision`, the same code and the same
reason as two colliding client tools (grammar 11.5). Two ways to spend one:

```yaml triggers tool-name-collision
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: code_execution_20250522
      name: code_execution
    - type: code_execution_20250825
      name: code_execution
```

Both dated revisions are `code_execution` — that is what the Messages wire pairs
with either `type:` — so one of the two goes. That half holds on every kind: two
entries of one array under one name are one tool twice by your own reckoning,
whatever the wire keys on.

The other way is an attachment, and that half is the **Messages wire's alone**:
an `agent.*` holding `tool.web_search` whose model reaches an `anthropic`
provider declaring `web_search_20250305` offers `web_search` twice, and the fix
is to rename the attachment or to move the suite onto a provider that agent does
not use. It is the one wire where a server tool and a client tool sit under the
same key. On the Responses wire a built-in is addressed by its `type:` while a
function tool carries a `name:`; on Chat Completions a function tool's name is
nested inside its own object; and no table could say what a gateway keys its
vocabulary on. So `tool.search_docs` beside an `openai_compatible` connection
whose suite declares `name: search_docs` compiles — the entry's
`unknown-server-tool` warning is the whole of what this release has to say about
it.

### Two tiers of checking, and why

The compiler keeps a curated table of the server tools each kind is known to
serve. A `type:` **in** it is checked strictly — the fields the table models,
their types, their ranges, and the constraints the vendor states, like web
search's allow-list and deny-list being mutually exclusive. A config the provider
will refuse is otherwise a run that dies on its first model call with a 400 and
no span.

On the Messages wire that includes the `name:` beside the `type:`, which is not
free text: Anthropic pairs each dated type with one fixed name and refuses a
request whose two disagree. `web_search_20250305` is `web_search`,
`web_fetch_20250910` is `web_fetch`, and both `code_execution_*` revisions are
`code_execution`.

A `type:` **outside** it is a warning and is carried to the wire as written:

```yaml triggers unknown-server-tool
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search
      name: web_search
```

That composition **builds and runs**. The warning names exactly what could not
be verified — here, that `web_search` is the OpenAI spelling and the Messages
wire takes the dated `web_search_20250305` — and the point of the second tier is
the case where the spelling is right and this release is simply older than the
tool: a server tool the vendor ships tomorrow is usable the day it ships.

A **key** the table does not name, inside a `type:` it does, is the same
warning one level down:

```yaml triggers unknown-server-tool-field
version: "0.1"

provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
  server_tools:
    - type: web_search_20250305
      name: web_search
      max_uses: 5
      result_freshness: week
```

The table's row for a tool is a snapshot of it taken when this compiler was
released, and vendors add parameters to tools they already ship. Refusing
`result_freshness:` would be the treadmill at field granularity — and there is no
way out of it from the spec, since renaming the `type:` to reach the unchecked
tier would change which tool runs. So the key is carried, and the warning is the
record that it was. Where the spelling is close to a field the table does name,
the diagnostic says which (`max_usages` → `max_uses`), because nothing in the
compiler can tell a typo from a parameter it predates.

What stays an **error** is what the table genuinely knows: a field it models
given the wrong kind of value, a value outside a range or closed set the vendor
states, a required field left out (`file_search` with no `vector_store_ids:`),
and two fields the vendor refuses together.

`azure_openai`, `bedrock` and `vertex` refuse the key outright
(`unsupported-server-tools`): their wires have not been taught the shape, so a
suite declared there would be dropped on the floor rather than merely unchecked.

### The OpenAI seam

OpenAI's built-in tool suite lives on the **Responses API**, which Chat
Completions does not carry. So an `openai` provider that declares
`server_tools:` speaks `POST /v1/responses` for **all** of its calls, and one
that declares none keeps Chat Completions exactly as before. One provider, one
wire — a connection that switched per request would make "what did this model
see" depend on which agent asked.

Two `settings:` keys change spelling on that wire and the runtime translates
them: `max_tokens` becomes `max_output_tokens`, and `reasoning_effort` becomes
`reasoning: { effort }`. Two others have **no** Responses equivalent — `stop:`
and `seed:` — and are a **compile error** on a model bound to a provider that
declares a suite:

```yaml triggers unknown-key
version: "0.1"

provider.searching:
  kind: openai
  api_key: ${OPENAI_API_KEY}
  server_tools:
    - type: web_search

model.smart:
  provider: provider.searching
  id: gpt-5
  settings:
    stop: ["\n\n"]
```

Which wire the connection speaks is the compiler's own decision — one key beside
another decides it — so a knob that wire will not read is decidable here rather
than on the first model call. If you need either, keep the suite off that
provider and declare a second one for the agents that want a search.

**One thing the compiler does not decide: retention.** The Responses API's
service-side default for `store` is `true`, where Chat Completions' is `false`,
so a connection that moves onto this wire has its prompts and completions
retained by the provider where before they were not. The emitted request does
not pin the key — `store: false` makes the service refuse a replayed `reasoning`
item, and a tool loop replays every turn it takes — so a deployment with a
retention policy declares the suite on a provider whose data it may retain, and
leaves the rest of the graph on a connection that never moved.

`openai_compatible` is unaffected on every count: a gateway keeps Chat
Completions, its suite rides that request's own `tools` array verbatim, and
every entry there is second-tier, because no table could be authoritative about
what a gateway honours.

### Failover

A route's members each name their own provider, so which tools were on offer
depends on which member answered. A route whose members declare **different**
suites is a warning (`mismatched-server-tools`), not a refusal: a fallback vendor
with no web search is still a fallback, and the compiler's job is to make the
difference visible rather than to choose for you.

Different means **field for field**, not tool for tool: two members that both
declare web search and give it `max_uses: 1` and `max_uses: 99` offered the
model materially different tools, and a copy-then-edit of one provider is
exactly how that arrives.

## Models

A model definition is **either** a direct binding **or** a route — never both.

**Direct form:**

| Key | Required | Notes |
|---|---|---|
| `provider` | yes | a `provider.*` reference |
| `id` | yes | the provider-native model id; no env refs |
| `settings` | no | validated against the provider plugin's settings schema |
| `description` | no | |

**Route form:**

| Key | Required | Notes |
|---|---|---|
| `route` | yes | ≥ 2 **distinct** `model.*` refs, ordered fallback; members must be direct models — no nested routes |
| `route_on` | no | non-empty, distinct; default `[rate_limit, overloaded, timeout]`; the fourth value is `server_error` |
| `description` | no | |

A repeated `route:` member is a fallback to the model that just failed — an
inert entry, and an error. `route_on: []` declares a route that never fails
over; write a direct model instead.

## Settings

`settings:` is the one **open** object in the logical layer. Known keys are
typed for editors — `temperature`, `top_p`, `top_k`, `max_tokens`, `stop`,
`seed`, `thinking`, `reasoning_effort`, `parallel_tool_calls` — and
plugin-specific keys are accepted and checked against the provider plugin's
schema. `thinking:` on an OpenAI provider is a compile error **at the model
definition**, not at run time.

Values inside `settings:` carry no env refs: every LLM configuration in a
project stays greppable in one file, and a value that only exists at process
start is one neither `validate` nor a diff can see.

**There are no inline settings overrides on agents.** Different settings means
another named model. That is the whole reason the two namespaces exist.

## How structured output rides each wire

**There is nothing to configure here.** This section is for reading a trace, not
for writing YAML: every agent declares an `output:` schema, and how that schema
is asked for on the wire is the provider integration's problem and never yours.
No key, no capability flag, no model table you have to keep up to date.

What there *is* to know is that each wire has **two** ways of asking a model for
an object under a schema, and they move with model generations:

| wire | native | forced tool |
|---|---|---|
| Messages (`anthropic`) | `output_config: { format: { type: json_schema, schema } }` | a synthetic tool carrying the schema, pinned with `tool_choice: { type: "tool" }` |
| Chat Completions (`openai`, `openai_compatible`, `azure_openai`) | `response_format: { type: json_schema, … }` | the same tool as a function, pinned with `tool_choice: { type: "function" }` |
| Responses (`openai` with `server_tools:`) | `text.format` of type `json_schema` | the same, pinned by name |

The runtime prefers the **native** parameter and keeps the forced tool as the
other rung. A refusal that names the mechanism — a 400 for an argument the
endpoint has never heard of, or the newest Anthropic generation's
`tool_choice: type "tool" and "any" are not supported for this model.` — makes it
send the *same* call once more the other way. The rung an endpoint **refused** is
remembered per provider-and-model for the life of the process, so only the first
call of a pairing pays that extra round trip. Both halves of that key are load
bearing: what one model behind a gateway refuses says nothing about the model
beside it, and what a gateway refuses says nothing about the vendor endpoint
serving the same model name.

A proxy that **relays** the refusal it got upstream is read the same way. Behind
a gateway you rarely see the service's own envelope: what comes back is the
gateway's exception class or vendor label with the upstream sentence inside it —
`litellm.BadRequestError: AnthropicException - {"…":"output_config: Extra inputs
are not permitted"}`. What decides the ladder is the complaint that arrived, not
the name in front of it, so a proxied deployment ladders exactly as a direct one
does.

One thing decides the order before any of that, and it is a property of the agent
rather than of the endpoint — but only on the **Messages** wire. `output_config`
constrains a decoder over a **closed** schema and refuses one that is not, so an
agent whose `output:` nests an object with `optional:` properties starts on the
forced tool there. The OpenAI wires send such a schema with `strict: false`,
which constrains nothing and refuses nothing, so they stay on the native
parameter. Two agents on one Anthropic model can therefore ask in two different
ways, and a trace showing exactly that is not a bug.

An agent with `tools:` still **offers** them on the call that asks for its
output, under either mechanism — the exchange being replayed names them, so the
request has to declare them, and the provider's own `server_tools:` are on that
call too. The forced tool pins the answer; the native parameter shapes it without
forbidding anything, and nothing else on that call forbids tool use either: a
`tool_choice` that did would forbid the provider's server tools along with the
agent's, and would make the native rung depend on a second parameter whose
refusal the ladder cannot read — an endpoint refusing *that* would leave nowhere
to ladder to. So on the native rung a model can answer that last turn with
one more tool call instead of the object, and the node then fails with "asked
for … and the answer carried no structured output", naming
`stop_reason: tool_use` as what the surface said about why.
That is a model ignoring a fixed instruction to produce its result, not an
endpoint that lacks a mechanism — nothing was refused, and nothing laddered.

Nothing else ladders. A 401, a 429, a 5xx, a content refusal, a schema the
decoder will not compile, a complaint about the **conversation** rather than
about a parameter — a strict gateway refusing a history that ends on an assistant
turn — and a parameter refused because of **another parameter in the same
request**, such as `output_config` beside a `settings:` key this model will not
take it with, are refusals about something other than the mechanism, and each
behaves exactly as it always has. The last of those is the one worth knowing
about, because it is the only refusal here that *does* name something in your
composition to change: you get the endpoint's own sentence, and it names the key. Including for `route_on:`, which is untouched: the
mechanism ladder is *inside* one call to one route member, so a condition the
working rung answers with is the route's to act on as it always was, and a route
still fails over only on grammar 12.2's infrastructure conditions — which the
double refusal below is not.

**Where to see it.** Each model call's trace record carries `outputMechanism` —
`"native"` or `"forced_tool"` — on the one call per agent node that asks for the
output object (`agent-compose docs trace`). Two deployments of one composition
can answer through different mechanisms: a gateway a generation behind the wire
it proxies takes only the forced tool, the newest model generation takes only the
native parameter, and the same YAML runs on both. That comparison is usually made
on a collector rather than in two files, so a `trace_sink:` sending OTLP carries
the same fact as `agentcompose.model.output_mechanism` on the model call's span.

The ladder itself is **inside** one model call, so it does not add a record: a
first call that discovered its rung is one `models[]` entry naming the mechanism
that answered, and the rung that was refused leaves none. Where the extra round
trip shows up is your provider's own request log — two requests against one
trace record — and in that call's timing. So `outputMechanism: "forced_tool"` is
what says the discovery happened at all, with the one exception the paragraph
above names: on the Messages wire an agent whose schema is not closed starts
there, and nothing was discovered.

**When both are refused**, the run fails with a diagnostic quoting *both*
refusals verbatim. That is deliberate and it is not a missing knob: the runtime
has already tried everything the wire offers, so what is left to say is what the
endpoint said. Read the two bodies — they name the endpoint, not your
composition.

## Capability checks

Every model an agent references must come from a provider declaring
structured-output and tool-use capability — agents require structured output.
All members of a route must be **capability-equivalent**, so failover cannot
silently break it.

Failover conditions are **infrastructure conditions only**. Content-based
routing is out of scope; that is what graph edges are for.

Normative source: `docs/grammar.md` §12, §12.1, §12.2
