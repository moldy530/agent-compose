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

That `http` trigger is unauthenticated, which in v0 is a posture rather than an
omission: "Authenticating an `http` trigger", below, is what a spec writes when
it wants otherwise, and what the compiler does with it today.

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
| `auth` | no | — | how an inbound call is authenticated; exactly one of `bearer:`/`hmac:` |
| `callback_auth` | no | — | how a delivery identifies itself; `bearer:`, `hmac:`, or both |
| `callback_allow` | with `callback_auth` | — | where a callback may point |

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

## Authenticating an `http` trigger

**All three keys below are reserved in v0.** `auth:`, `callback_auth:` and
`callback_allow:` are **fully specified, parsed, type-checked, and carried into
the IR**, and nothing generated reads them yet: a served trigger declaring
`auth:` is exactly as open as one declaring none, and a callback still POSTs
wherever the payload pointed. Everything below is the contract the runtime is
written against, not what `serve` enforces today — until it lands, a deployment
that needs these guarantees puts a gateway in front, and the block is what that
gateway is configured to match. The closing section, "What `reserved` means", is
the same statement once for every construct in this topic.

A generated app is the thing a webhook vendor calls directly, so verifying
callers is its job rather than a gateway's. Auth is **per trigger, never
server-wide**: who may invoke this flow is readable off the trigger that exposes
it.

```yaml
auth:                            # exactly ONE of bearer | hmac
  hmac:
    secret: ${WEBHOOK_SECRET}    # required; ${ENV} only, never a literal
    header: X-Hub-Signature-256  # default X-Signature
    algorithm: sha256            # sha1 | sha256 | sha512; default sha256
    encoding: hex                # hex | base64; default hex
    prefix: "sha256="            # default "" (empty)
```

- **`bearer`** compares a static secret against a named header — `Authorization`
  with a `Bearer ` prefix by default — in **constant time**.
- **`hmac`** verifies a signature over the **raw request body bytes**, which is
  the GitHub-shaped family most vendors speak.
- Both secrets are `${ENV}` references and nothing else: a literal is
  `invalid-env-ref`, because a secret never lives in the spec text.
- A block declaring **neither** scheme and one declaring **both** are each a
  compile error. One request carries one credential, and a route verifying either
  would be as open as its weaker half.
- `header:` is recorded as written and **matched case-insensitively**: HTTP/2
  lowercases header names on the wire, which is why `payload.headers` shows them
  lowercased too. `X-Hub-Signature-256` finds `x-hub-signature-256`.

**`auth:` guards three routes.** `resume` *injects data into a parked run* —
strictly more sensitive than starting one — so `resume` and `status` enforce the
auth of the trigger that **started that execution**. An execution an
authenticated trigger began never answers an unauthenticated poll; one a no-auth
trigger began keeps open routes.

## Identifying a callback delivery

Outbound auth mirrors the inbound pair, so one verification recipe serves both
directions — but here **both schemes together are legal**, because a receiver may
want a token and a signature.

```yaml
callback: "payload.body.callback_url"
callback_auth:
  bearer:
    token: ${CALLBACK_TOKEN}
  hmac:
    secret: ${CALLBACK_SECRET}   # signing is fixed HMAC-SHA256/hex: no other keys
callback_allow:
  - "https://hooks.example.com/*"
```

**Declaring `callback_auth:` makes `callback_allow:` mandatory** — a compile
error, `missing-callback-allowlist`, not a warning. The callback URL comes from
the payload and is attacker-controlled by construction, so a deployment careful
enough to authenticate its deliveries must not hand them, credential and all, to
whatever host a payload named. That compile error is live today; the matching it
demands is not — a URL outside the list will be refused when it is read, at
parking or settle, and recorded as a refused delivery, once the runtime reads the
list at all.

An entry is an absolute `http`/`https` URL with `*` standing for any run of
characters, matched against the whole callback URL. `http` stays legal — a
localhost receiver is the common first case. An empty list is an error: an
allowlist admitting nothing refuses every delivery. Either key on a trigger with
no `callback:` is an error too — it describes a delivery that never happens.

**Write the host out.** `*` crosses `/` and `?` like any other character, so
`https://hooks.example.com/*` constrains a host and `https://*.hooks.example.com/*`
constrains none: that leading `*` swallows `attacker.test/collect?x=` on its way
to the dot, and the payload naming it is admitted. `https://hooks.example.com*`
constrains none either — it admits `hooks.example.com.evil.test`. Both compile,
because a wildcard bounded to one label of the authority is a second wildcard
kind and the PRD's question to settle, not the compiler's; what an entry must
carry is a scheme and a host, so `https:///deliveries` is the error. One entry
per subdomain is what makes the list a guarantee.

**A `callback:` with no `callback_auth:` is a documented test posture**: it signs
nothing, claims nothing, needs no allowlist, and may POST anywhere. Choose it
deliberately.

**The delivery wire.** A callback fires on lifecycle events — every quiescence
that opened new pauses (`parked`) and settle (`settled`) — carrying the status
route's report plus `X-AgentCompose-Event`, `X-AgentCompose-Delivery`
(`<execution_id>:<ordinal>`), `X-AgentCompose-Ordinal` and
`X-AgentCompose-Timestamp`; with `callback_auth.hmac`, also
`X-AgentCompose-Signature: sha256=<hex>`. Deliveries are journaled and
at-least-once, so they can arrive out of order: **order by ordinal, never by
arrival**.

Those names are the receiver's contract, so the `X-AgentCompose-` namespace is
reserved: a `callback_auth.bearer.header:` inside it is a compile error, since
two values under one header name is not something a receiver can read. Inbound
`auth:` may name them freely — that is how a trigger verifies deliveries from
*another* agent-compose deployment.

## `schedule` and `event` — reserved

Both are reserved in the sense the closing section gives: **fully parsed,
type-checked, and carried into the IR**, and executed as no-ops in v0.

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

## What `reserved` means

Reserved constructs are **fully specified, parsed, type-checked, and carried
into the IR**, and execute as no-ops in v0. Using one is never an error; relying
on its runtime effect is a documented no-op.

| Construct | Status in v0 |
|---|---|
| `triggers.<t>.type: schedule` | parsed + validated, no-op |
| `triggers.<t>.type: event` | parsed + validated, no-op |
| `triggers.<t>.auth` | parsed + validated, no-op — the route serves unauthenticated |
| `triggers.<t>.callback_auth` | parsed + validated, no-op — deliveries carry no credential |
| `triggers.<t>.callback_allow` | parsed + validated, no-op — no URL is refused |

The three authentication keys are the ones worth reading twice: a no-op
`schedule` runs nothing and is visibly inert, while a no-op `auth:` serves every
caller and looks exactly like a guarded route. Read them as a declaration of
what the deployment will enforce, and put a gateway in front of anything that
needs the guarantee today.

The whole surface written out is a spec `validate` calls valid and `build` owes
a project to — which is the point of declaring it now:

```yaml spec
version: "0.1"

flow.support:
  outputs: {}
  nodes:
    approve:
      human:
        input: {}
        output:
          decision: { enum: [approve, reject] }
  edges:
    - { from: start, to: approve }
    - { from: approve, to: end }

triggers:
  intake:
    type: http
    flow: flow.support
    callback: "payload.body.callback_url"
    auth:
      hmac:
        secret: ${WEBHOOK_SECRET}
    callback_auth:
      hmac:
        secret: ${CALLBACK_SECRET}
    callback_allow:
      - "https://hooks.example.com/*"
```

Keep the block: it is what the gateway in front is configured to match, and it
is what the release that lands the runtime reads.

Normative source: `docs/grammar.md` §9.4, §13, §13.1–13.5, §15
