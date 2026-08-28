# targets

A **target** is an environment. `deploy/<target>.yml`, selected with
`--target <name>`, is the only layer that forks per environment: `agents/`,
`flows/`, `stores/` and `tools/` never do. That invariant is what makes a
composition one artifact with several deployments rather than several
compositions.

A deploy file is never imported. It is a document kind of its own, and the two
kinds are disjoint — a spec file declaring `placements:`, `storage_backends:` or
`event_sources:` is an error, and so is a deploy file declaring definitions,
`imports:`, `state:`, `triggers:` or `defaults:`.

```yaml deploy staging
# deploy/staging.yml
version: "0.1"

placements:
  agent.researcher: { runtime: isolated, network: egress }
  flow.ingest: { runtime: colocated }

storage_backends:
  defaults:
    kv: { provider: redis, url: "${REDIS_URL}" }
  aliases:
    docs_db: { provider: chroma, url: "${CHROMA_URL}" }

event_sources:
  bug_reports:
    kind: redis_streams
    url: ${REDIS_URL}
    stream: bug-reports
    consumer_group: agent-compose
```

The spec side of that project needs nothing unusual — a store names an alias and
that is all:

```yaml spec
version: "0.1"
provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.openai
  id: gpt-4o-mini

agent.researcher:
  model: model.smart
  prompt: Answer the question from the documents you are given.
  input:
    text: { type: string }
  output:
    answer: { type: string }

store.docs:
  kind: vector
  scope: global
  backend: docs_db              # an abstract slot, resolved per target
  embed:
    model: text-embedding-3-small
    provider: provider.openai

flow.ingest:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "input.text"
      value: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: end }
```

That file validates under `--target local` with no deploy file at all, and under
`--target staging` against the aliases above. The two are checked together: the
addresses `placements:` names — `agent.researcher`, `flow.ingest` — have to
resolve in the composition the deploy file is a target of.

## `local` is reserved and built in

It is the target when `--target` is omitted, it runs with an **in-memory**
checkpointer, and it substitutes SQLite and local disk for **every** store
unconditionally. Four consequences:

- `deploy/local.yml` is **optional**, and `--target local` with no file is the
  zero-config path, not an error.
- When present it may declare `placements:` and `event_sources:` — both reserved
  grammar, parsed and carried into the IR under every target.
- It **must not** declare `storage_backends:`. That section is *active* grammar
  which `local` overrides unconditionally, so the block could only be an inert
  key whose author expected a substitution. A store that wants a real backend
  locally is a `--target` of its own.
- `--target <name>` for any other name **requires** `deploy/<name>.yml` to
  exist. A missing file is `io-error` naming the expected path, never a silent
  fall-back to built-ins.

Under `local` the two target-dependent binding checks are satisfied
**vacuously**: a store's `backend:` alias needs no definition, and an `event`
trigger's `source:` needs no `event_sources:` entry. That is the
zero-infrastructure guarantee — production infrastructure in
`deploy/staging.yml` and a project that still validates and runs on a laptop.

## Checkpointing is a property of the target

v0 declares no grammar for configuring a checkpointer. The rule the grammar
depends on is fixed instead: **`local` is not durably checkpointed; every other
target is.** Exactly one static check keys off it — `detach: true` on a `map`,
which is `unsupported-detach` under any checkpointed target. So the same
composition is legal under `--target local` and rejected under
`--target staging`, and `validate --target staging` is how you find that out
before deploying.

Checkpointing is **not** durability, and the two are easy to run together.
That rule is about a LangGraph checkpointer; what makes an execution survive a
restart here is a journal the runtime writes itself (below). `local` is still
the un-checkpointed target, and it is also a durable one.

## Durability is bound by the target too, and `local` binds SQLite

Every invocation of every flow is journaled — no key turns it on, and none turns
it off. Under `--target local` the journal is one SQLite file beside the
project:

```text
<project>/.agent-compose/journal.sqlite
```

beside the stores, moved whole by `AGENT_COMPOSE_DATA_DIR`, and deleted by
deleting it. The path is the same for `run`, `serve` and `resume` of one project
and one target, which is what lets one command finish what another started.

What is in it is every **effect** a run issued as it issued it: the model answers
it got, the results its tools produced, what its stores read and wrote, and what
a person answered a `human` node. A resumed execution re-runs the same graph and
**consumes** that record instead of re-issuing it, up to the frontier — the
first effect the journal does not hold — where it goes live again.

```sh
agent-compose run main.yml flow.review --input goal=ship
# execution: exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
# … the machine reboots …

agent-compose resume main.yml exec_9f1c8a3e-1b7d-4a20-9d61-1f0e8a2c4d55
# the two model calls it had already made are not made again
```

`serve` needs no such command: on start it recovers every execution the journal
holds open, before it accepts a connection, and one parked on a `human` pause
re-parks under the same wait id — so a resume request prepared against the
process that died still finds its wait. Triggers are not re-fired. It does not
wait for the replays, so a request that arrives before one is back at its pause
is refused with `recovering: true` and told to send it again.

Which is also why `resume` is the verb for an execution **nothing is running**:
one process at a time writes a project's journal, and a `serve` that is up has
already taken every open execution it holds. Finish those through its
`POST /executions/:id/resume` route.

Because the journal holds what a trace deliberately does not — completions, tool
results, a person's answer — it is **private recovery data with the same
sensitivity as this project's stores**, never an observability artifact. Nothing
uploads it and no command prints it.

A distributed target will bind Postgres behind the same interface; every target
this release can build is process-local, so every one of them binds SQLite and
there is nothing for a deploy file to say. `docs/durability.md` is normative,
and `docs/grammar.md` Decision D121 records why there is no grammar for it.

## `storage_backends`

| Key | Shape |
|---|---|
| `defaults` | kind (`kv`/`vector`/`blob`) → backend config, a per-kind fallback |
| `aliases` | alias identifier → backend config, the slots stores name via `backend:` |

A backend config requires `provider` and accepts provider-specific keys, checked
against the storage plugin's schema. Connection strings and credentials are
env-ref values only.

| Kind | v0 `provider` values |
|---|---|
| `kv` | `memory`, `sqlite`, `redis`, `postgres` |
| `vector` | `sqlite_vec`, `chroma`, `pgvector`, `qdrant` |
| `blob` | `local_fs`, `s3`, `gcs` |

Capability checks apply at the alias definition: a `vector` store bound to a
non-vector-capable provider is a compile error.

## `placements` — reserved

Keys are component addresses (`agent.*`, `tool.*`, `flow.*`) that must resolve
in the composition. `runtime:` is `isolated` or `colocated` and is required;
`network:` is `none`/`egress`/`all` and defaults to `all`. `--target local`
implies everything colocated in one process.

## `event_sources` — reserved

Maps the logical `source:` names of `event` triggers to infrastructure. `kind:`
is `redis_streams`, `sqs`, or `nats`, plus per-plugin keys such as `url`,
`stream`, `consumer_group`, `queue_url`, `subject`.

## What "reserved" means

Reserved constructs are **fully specified, parsed, type-checked, and carried
into the IR**, and execute as no-ops in v0. Using one is never an error; relying
on its runtime effect is a documented no-op.

| Construct | Status in v0 |
|---|---|
| `placements` | parsed + validated, no-op |
| `event_sources` | parsed + validated, no-op |
| `triggers.<t>.type: schedule` | parsed + validated, no-op |
| `triggers.<t>.type: event` | parsed + validated, no-op |
| `network:` on a placement | parsed, no-op |
| `triggers.<t>.auth` | parsed + validated, no-op — the route serves unauthenticated |
| `triggers.<t>.callback_auth` | parsed + validated, no-op — deliveries carry no credential |
| `triggers.<t>.callback_allow` | parsed + validated, no-op — no callback URL is refused |

`human` nodes were on this list and have left it: the runtime landed, so a
compiled project really pauses and resumes — and their durability has left it
too: a wait that a restart interrupted comes back with its id intact, because
the wait id is the node's instance path and the journal is what a resumed
execution reads its answers out of.

Normative source: `docs/durability.md`, `docs/grammar.md` §14, §14.1, §14.2, §14.3, §15
