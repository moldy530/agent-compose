# triggers

Triggers define what causes an execution to exist. The entrypoints of a project
— the surface a deployment exposes — are exactly the flows its **declared**
triggers point at. There is no `entrypoint:` key.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.reviewer:
  model: model.m
  prompt: Review what you are given.
  input:
    goal: { type: string }
  output:
    verdict: { enum: [approve, revise] }

flow.review:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    review: { agent: agent.reviewer, input: { goal: "input.goal" } }
  edges:
    - { from: start, to: review }
    - { from: review, to: end }

triggers:
  cli:
    type: manual
    flow: flow.review

  on_request:
    type: http
    flow: flow.review
    path: /review
    method: POST
    input:
      goal: "payload.body.goal"
    session_key: "payload.headers['x-session-id']"
    respond: async
    callback: "payload.body.callback_url"
```

## One entry exists without being declared

**Every flow is runnable from the CLI** —
`agent-compose run main.yml flow.<name> [--input k=v]… [--session <key>]` —
whether or not a `manual` trigger names it. Writing the trigger out documents
the intended entry and lets it carry a `description:` or a `session_key:` remap;
it neither enables nor restricts anything the CLI would otherwise do.

Implicit invocation is **not** a trigger, and the difference is normative: it
contributes no entry to the trigger table, has no name, and every rule that
quantifies over triggers — input-binding compatibility, session coherence,
sync interrupt-freedom, event source binding — quantifies over **declared**
triggers only. A flow no declared trigger names is still a complete, legal
definition.

## Common keys

| Key | Required | Notes |
|---|---|---|
| `type` | yes | `manual` \| `http` \| `schedule` \| `event` |
| `flow` | yes | a `flow.*` reference; the execution's entry module |
| `input` | `http`/`schedule`/`event` only | flow-input field → CEL over `payload`; **illegal on `manual`** |
| `session_key` | no | CEL over `payload` → string |
| `description` | no | |

Where `input:` is legal it is compile-checked against the target flow's
`inputs`: every field the flow declares as required — one with no `default:` —
must be bound, and a binding for a field the flow does not declare, or whose
result type the field cannot accept, is a compile error.

## `manual`

```yaml
cli:
  type: manual
  flow: flow.review
```

A manual trigger must **not** declare `input:`. CLI arguments are validated
directly against the flow's input schema at run start — an unknown argument
name, a missing required field, or a value that does not fit fails the run
naming the field. That is the one place this check is deferred, for the same
reason env-ref presence is: the values do not exist until the command runs.

`session_key:` defaults to `"payload.session"`, and the manual payload has
exactly one member — the CLI's `--session <key>`. So a manual trigger always has
a session identity available, which is what makes the session-coherence check
satisfiable with no binding. Supplying the value is a run-time requirement:
`--session` is mandatory for a run whose flow reaches a session-scoped store.

## `http`

| Key | Required | Default | Notes |
|---|---|---|---|
| `path` | no | `/triggers/<name>` | starts with `/`, no whitespace; router parameter syntax (`/reviews/:id`) passes through |
| `method` | no | `POST` | `POST` \| `PUT` \| `GET` |
| `input` | no | — | field → CEL over `payload` |
| `respond` | no | `async` | `sync` \| `async` |
| `timeout` | `sync` only | `60s` | the response budget; **illegal** with `respond: async` |
| `callback` | no | — | a completion webhook; `async` only |

`payload` is `payload.body` (a decoded JSON object), `payload.query`,
`payload.headers` (lowercase names), `payload.path`, `payload.method`.

**`payload.body` on a bodyless request.** A `GET` decodes no body, so
`payload.body` is `{}` — present and readable, so `has(payload.body.goal)`
answers `false` instead of erroring. Consequently a `method: GET` trigger must
**not** read *through* `payload.body` in any of its CEL: the object is `{}` on
every request it can receive, so it is a guaranteed runtime failure and is
refused at compile time. Bind from `payload.query` instead.

- **`respond: async`** returns an execution id immediately; the optional
  `callback:` webhook fires on completion.
- **`respond: sync`** blocks and returns the flow's outputs, and the flow must
  be statically **interrupt-free**: it must not reach a `human` node through its
  own nodes, its maps' dispatch targets, the flows it instantiates, or the
  `flow.*` tools of any agent it reaches (`sync-trigger-interrupt`). On timeout
  expiry the response **upgrades to async** — 202 plus an execution id and a
  status URL — and the execution continues.
- `callback:` with `respond: sync` is an error: the response already carries the
  outputs. `timeout:` with `respond: async` is an error for the mirror reason:
  there is no response for a budget to bound.

Two `http` triggers declaring one route — the same effective `path:` at the same
`method:` — is `duplicate-route`. The generated app mounts one route per trigger
and cannot dispatch one pair two ways.

Generated apps expose `start`, `resume` and `status` routes; resume payloads are
validated against the interrupting `human` node's output schema.

## `schedule` and `event` — reserved

Both are **fully parsed, type-checked, and carried into the IR**, and execute as
no-ops in v0. Using one is never an error; relying on its runtime effect is a
documented no-op.

```yaml
nightly:
  type: schedule
  flow: flow.triage
  cron: "0 3 * * *"      # 5-field POSIX cron, shape-checked at compile time
  timezone: UTC          # an IANA name
  input: { scope: "'full'" }

ingest:
  type: event
  flow: flow.triage
  source: bug_reports    # a logical name bound in deploy/<target>.yml
  input: { report: "payload.body" }
  dedupe_key: "payload.id"
```

A `schedule` payload is `payload.scheduled_at` and `payload.trigger`; an `event`
payload is `payload.id`, `payload.body`, `payload.attributes`,
`payload.source`. `dedupe_key` defaults to `payload.id` and is the inbound
at-least-once dedupe. A `source:` not defined in the active target's
`event_sources:` is a compile error naming the target — satisfied vacuously
under `local`, which runs no consumer.

## Idempotency keys

Two constructs deliver an effect without observing its outcome — a **detached**
`map` dispatch and a **store write** — so each carries an idempotency key the
receiver dedupes on. The key is `execution.id` followed by one frame per node
crossed from the root instance down to the effect site, outermost first:

```
<flow-local node id> "/" <traversal ordinal> [ "/" <source-item index> ]
```

A flow-as-tool call contributes `<tool name>/<call ordinal>` beneath the
agent node's own frame. Examples:

```
exec_01/a/0/save/0                   store node `save`, in a flow instantiated by node `a`
exec_01/outer/0/3/inner/0/0/save/0   `save` under item 0 of `inner`, itself item 3 of `outer`
exec_01/dispatch/0/7                 the detached dispatch of item 7 by map node `dispatch`
```

Two properties follow: **distinct effects get distinct keys**, and **a repeated
attempt at one effect reuses its key** — a node retry, an item retry, and a
`flow:` node's re-execution all re-run the same attempt.

How the key reaches a sink is fixed per binding kind: an `http:` target gets the
`Idempotency-Key` header, an `exec:` target the `IDEMPOTENCY_KEY` environment
variable, a `function:` target an `idempotency_key` field on its invocation
context. It is delivery metadata, never part of the target's input schema, and
never authored — there is nothing here for `validate` to reject.

Normative source: `docs/grammar.md` §9.4, §13, §13.1–13.5
