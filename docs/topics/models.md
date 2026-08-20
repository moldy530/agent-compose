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
| `anthropic` | `api_key` | `base_url`, `headers` |
| `openai` | `api_key` | `base_url`, `headers`, `organization` |
| `openai_compatible` | `base_url` | `api_key`, `headers` |
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

**Credentials are never literals.** `api_key`, `api_secret`, `token`,
`password`, `access_key_id`, `secret_access_key`, `session_token`,
`credentials_json`, `url`, `base_url`, `endpoint`, `dsn` take the env-ref value
form — the whole string is one `${NAME}` — and a literal there is
`invalid-env-ref`. `region`, `location`, `project`, `organization`, `profile`
and `api_version` are plain strings and may be interpolated.

Env refs survive **unresolved** into the artifact: `validate` and `build` check
syntax only, so a build succeeds in CI holding no keys. Presence is checked at
process start, and `run`/`serve` fail fast naming the missing variable.

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

## Capability checks

Every model an agent references must come from a provider declaring
structured-output and tool-use capability — agents require structured output.
All members of a route must be **capability-equivalent**, so failover cannot
silently break it.

Failover conditions are **infrastructure conditions only**. Content-based
routing is out of scope; that is what graph edges are for.

Normative source: `docs/grammar.md` §12, §12.1, §12.2
