# stores

Durable, attachable storage as a first-class component. Three kinds — `kv`,
`vector`, `blob` — and two consumption surfaces: a `store:` node
(deterministic, graph-invoked) and an agent's `stores:` list, which synthesizes
LLM-facing tools. Relational storage is deliberately out of scope.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
provider.embeddings:
  kind: openai
  api_key: ${OPENAI_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

store.user_prefs:
  kind: kv
  scope: session
  value_schema:
    theme:     { type: string }
    verbosity: { enum: [low, high] }

store.docs:
  kind: vector
  scope: global
  description: Project documentation, chunked.
  embed:
    model: text-embedding-3-small
    provider: provider.embeddings
    dimensions: 1536
  metadata_schema:
    source: { type: string }
  backend: docs_db
  agent_access: read

agent.answerer:
  model: model.m
  prompt: Answer the question using the documentation.
  stores: [store.docs]
  output:
    answer: { type: string }

state:
  prefs:
    type: object
    properties:
      theme:     { type: string }
      verbosity: { enum: [low, high] }

flow.answer:
  inputs:
    question: { type: string }
  outputs: {}
  nodes:
    load_prefs:
      store: store.user_prefs
      op: get
      key: "execution.session_key"
      writes: { value: prefs }
    reply:
      agent: agent.answerer
      input: "input.question"
  edges:
    - { from: start, to: load_prefs }
    - { from: load_prefs, to: reply }
    - { from: reply, to: end }

triggers:
  cli: { type: manual, flow: flow.answer }
```

## Definition keys

| Key | Required | Notes |
|---|---|---|
| `kind` | yes | `kv` \| `vector` \| `blob` |
| `scope` | yes | `execution` \| `session` \| `global` — lifetimes are explicit |
| `value_schema` | `kv` **required**; illegal on `vector`/`blob` | the stored value's shape |
| `metadata_schema` | `vector` optional; illegal on `kv`/`blob` | filterable metadata |
| `embed` | `vector` **required**; illegal on others | see below |
| `backend` | no | a bare alias naming an abstract slot; never provider config |
| `description` | no | LLM-facing for agent-attached stores |
| `agent_access` | no | `read` \| `read_write` (default) |

`metadata_schema` on a `blob` store is `conflicting-keys`: nothing in this
grammar reads it — no `blob` op takes a `metadata` or `filter` parameter and no
synthesized `blob` tool returns one — so declared there it would be a key every
write and every read ignores. A blob's filterable attributes live in a `kv`
store keyed by the same key.

## `embed`, on `vector` stores

| Key | Required | Notes |
|---|---|---|
| `model` | yes | a provider-native embedding model id — a bare string, not a `model.*` |
| `provider` | **yes** | which connection computes the vectors |
| `dimensions` | no | asserted against the backend's index |

**Embeddings are served by a `provider.*`, and the storage backend never
computes them.** `backend:` says *where the vectors live* and forks per target;
`embed.provider` says *what turns text into a vector* and does not. Naming the
connection is what keeps one store's embeddings identical under `local` and
under `staging`, with only the vectors' home changing. The provider must publish
an embeddings capability, or it is `missing-capability`.

## Backends and scope

`backend:` names an **abstract alias** defined per target in
`deploy/<target>.yml` under `storage_backends.aliases`. Resolution is explicit
alias, then a per-kind `defaults:`, then a target built-in. An alias undefined
in the active target is a compile error naming the target.

**`--target local` substitutes local storage for every store unconditionally**,
so under `local` no alias and no per-kind default is consulted at all — which is
the zero-infrastructure guarantee: a project with production storage in
`deploy/staging.yml` still validates and runs locally. See
`agent-compose docs targets`.

**A backend the reaching process opens for itself cannot be reached from a
mesh.** `memory`, `sqlite`, `sqlite_vec` and `local_fs` have no server in the
middle, so two processes reaching one such store hold two stores — and a target
that declares `placements:` runs a placed component in more than one process by
design. Binding one from anything a placement's process can execute is
`process-local-store`, at every `scope:` and under `local` too. The repair is a
networked backend, whose variables the deployment already routes to every
placement that reaches the store — or keeping the component that binds the store
off `placements:` so only the hub opens it.

**Which backends this release opens.** The four above, and — for the `kv` kind —
`postgres` and `mysql`. A store bound to one of those two is a connection rather
than a file, and nothing else about it changes: the same ops below, the same
scope partitions, the same recorded reads and deduplicated writes, so a graph
moved between backends addresses the same rows and cannot tell which answered. It
is deliberately **multi-writer** — no writer guard, because the placement rule
above is what governs who may reach a store — and per key the last write wins. A
store bound to any other provider (`redis`, `chroma`, `pgvector`, `qdrant`, `s3`,
`gcs`) compiles and then refuses at its first op, naming the backend and where
the binding came from.

`scope: session` requires the execution to have a session identity, and that
identity comes from the trigger. A declared `http`, `schedule`, or `event`
trigger whose flow **reaches** a session-scoped store must declare
`session_key:`, or it is `missing-session-key`. `manual` triggers carry
`session_key: "payload.session"` by default and satisfy the check statically;
supplying `--session` is then a run-time requirement.

## Ops

`V` is the store's `value_schema`; `M` is its `metadata_schema`.

| kind | `op` | Parameters | Output |
|---|---|---|---|
| `kv` | `get` | `key` | `{ value: V (optional), found: boolean }` |
| `kv` | `set` | `key`, `value` (field map of CEL matching `V`) | `{ key: string }` |
| `kv` | `delete` | `key` | `{ deleted: boolean }` |
| `kv` | `list` | `prefix` (optional), `limit` (1..1000, required) | `{ keys: array<string> }` |
| `vector` | `search` | `query`, `top_k` (1..100, required), `filter` (optional) | `{ matches: array<{ id, score, text, metadata: M }> }` |
| `vector` | `upsert` | `key`, `value` (the text), `metadata` (optional) | `{ id: string }` |
| `vector` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `put` | `key`, `value`, `content_type` (optional literal) | `{ key: string }` |
| `blob` | `get` | `key` | `{ value: string (optional), found: boolean }` |
| `blob` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `list` | `prefix` (optional), `limit` (1..1000, required) | `{ keys: array<string> }` |

**A row's parameter list is exact**: a parameter the op does not take is an
error, not an ignored key. `op: get` with `top_k:` is rejected.

Six parameters are CEL over `input`/`state`/`execution` — `key`, `query`,
`prefix`, `value`, `filter`, `metadata`. Three are **literals**: `top_k` and
`limit` are integers, and `content_type` is a media type string. A CEL string
where an integer is declared is a type error, not a computed bound.

A `store:` node takes **no `input:` key**: its parameters are exactly the op's
row, and none of them resolves by name from a channel.

**A `get` that misses** returns `found: false` and **no `value`**. So a
`writes: { value: prefs }` performs no write on that pass and the channel keeps
what it held, and reading `<node>.output.value` from a guard **fails the
execution**. Route on the companion instead — `when: "load_prefs.output.found"`
— or ask with `has()`.

A `vector` store with **no** `metadata_schema` derives matches with no
`metadata` field at all, so reading one is an unknown field, and `filter:`/
`metadata:` naming any key is an error. Declaring `metadata_schema: {}` is a
*declaration*: the field is present and can only hold `{}`.

## Writes inside a `map`

A store **write** performed inside a `map`-dispatched instance must be one of
two forms:

1. an **item-derived key** — the `key:` expression *reads*
   `execution.item_index`, or reads an `input.<field>` whose binding *at that
   dispatch site* references the item binding or the index. Legal for every
   kind.
2. a **keyed `kv` write** — `set` or `delete` on a `kv` store with any legal
   key, including one constant across instances.

Anything else — a `vector` or `blob` write whose key is not item-derived — is
`unkeyed-map-write`. A field bound from `state.*`, from the enclosing flow's
`input.*`, or from a literal is **not** item-derived, however item-derived its
*name* looks: N documents would land on one vector key.

*Reads* the index, not *is* it: every op's `key` is a CEL string and
`execution.item_index` is an integer, so `key: "execution.item_index"` satisfies
this rule and then fails type-checking with `type-mismatch`. The spelling that
satisfies both is a string-valued expression that reads the index —
`key: "state.tasks[execution.item_index]"`.

The `kv` exemption is not a loophole. A `kv` write replaces the whole value at a
slot the author named, so concurrent instances writing one key are a declared
overwrite — the store-side counterpart of `reduce: last_wins`. A `blob` or
`vector` write has no such story: it is bulk content at a document id.

Store writes are at-least-once and carry an idempotency key derived from the
execution id and the store node's flattened instance path.

## Agent-attached stores

| kind | `agent_access: read` | `read_write` (default) |
|---|---|---|
| `kv` | `<name>_get` | `<name>_get`, `<name>_set` |
| `vector` | `<name>_search` | `<name>_search`, `<name>_upsert` |
| `blob` | `<name>_get`, `<name>_list` | `<name>_get`, `<name>_list`, `<name>_put` |

`<name>` is the store's local name, so `store.user_prefs` gives `user_prefs_get`.
A synthesized name colliding with an attached tool name is
`tool-name-collision`.

Each tool's arguments are its op row with the expressions replaced by what the
model supplies, held to that schema **before the store sees them**. A call the
schema refuses is returned to the model as an error tool result and reaches no
backend — a refused call leaves no store record. A **backend** that could not
answer fails the agent node instead.

Normative source: `docs/grammar.md` §8.8, §11, §11.1–11.5, §14.3
