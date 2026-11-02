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
| `agent_access` | no | `read` \| `read_write` (default) \| `full` |

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
above is what governs who may reach a store — and a plain unconditional `set` is
last-write-wins per key, while a conditional or partial write is serialized
against every other write to its key (see *Versions, conditions and partial
updates* below).
Each dialled store creates its tables at the first op of whichever process
reaches it first, and several processes of one placement doing that at the same
moment is the ordinary start-up rather than a race to lose. The one place a
server shows through is how long a key may be: a `kv` key and a session key are
whatever the composition and the trigger supplied, and a `mysql` store holds
2048 characters of a `kv` key. A session key is bounded more tightly than that
number suggests, because it is not stored as it arrived: the partition a
`scope: session` store writes is `session/` followed by the session key
**percent-encoded**, one `%XX` escape per byte outside `[A-Za-z0-9_-]` — so the
same 2048 characters are 2040 of an ASCII session key but only 226 of a CJK one,
each of whose characters is three bytes and therefore nine encoded characters. A
`postgres` store holds about 2704 bytes across the store name, the partition —
encoded the same way — and the key together. A `sqlite` store bounds neither.
A `mysql` store also has a version floor — **MySQL 8.0.19 or newer**, the release
whose upsert spelling and key collation it is written for — and a server below it
is refused when the store is opened, naming the version that server reported
rather than failing later on a write. The `postgres` arm asks for nothing newer
than `ON CONFLICT`, which every Postgres still in support has, so it states no
floor of its own.
A store bound to any other provider (`redis`, `chroma`, `pgvector`, `qdrant`,
`s3`, `gcs`) compiles and then refuses at its first op, naming the backend and
where the binding came from.

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
| `kv` | `get` | `key` | `{ value: V (optional), found: boolean, version: integer (optional) }` |
| `kv` | `set` | `key`, `value` (field map of CEL matching `V`), one optional condition | `{ key, applied: boolean, version: integer (optional), deduped: boolean }` |
| `kv` | `delete` | `key`, `if_version` (optional) | `{ deleted: boolean, applied: boolean, version: integer (optional), deduped: boolean }` |
| `kv` | `list` | `prefix` (optional), `limit` (1..1000, required) | `{ keys: array<string> }` |
| `kv` | `merge` | `key`, `fields` (field map of CEL, each a field of `V`), one optional condition | `{ key, applied, version (optional), value: V (optional), deduped }` |
| `kv` | `append` | `key`, `field` (an array field of `V`), `value` (one item), `unique` (optional literal, default `false`), one optional condition | as `merge` |
| `kv` | `incr` | `key`, `field` (an `integer`/`number` field of `V`), `by` (optional, default `1`), one optional condition | as `merge` |
| `vector` | `search` | `query`, `top_k` (1..100, required), `filter` (optional) | `{ matches: array<{ id, score, text, metadata: M }> }` |
| `vector` | `upsert` | `key`, `value` (the text), `metadata` (optional) | `{ id: string }` |
| `vector` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `put` | `key`, `value`, `content_type` (optional literal) | `{ key: string }` |
| `blob` | `get` | `key` | `{ value: string (optional), found: boolean }` |
| `blob` | `delete` | `key` | `{ deleted: boolean }` |
| `blob` | `list` | `prefix` (optional), `limit` (1..1000, required) | `{ keys: array<string> }` |

**A row's parameter list is exact**: a parameter the op does not take is an
error, not an ignored key. `op: get` with `top_k:` is rejected. So is an op the
store's kind does not take — a `merge` on a `vector` store — which is
`invalid-value` naming the ops the kind serves.

Nine parameters are CEL over `input`/`state`/`execution` — `key`, `query`,
`prefix`, `value`, `filter`, `metadata`, `fields`, `by`, `if_version`. Six are
**literals**: `top_k` and `limit` are integers, `content_type` is a media type
string, `field` is a field name, `unique` is a boolean, and `if` is `absent` or
`present`. A CEL string where an integer is declared is a type error, not a
computed bound.

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

## Versions, conditions and partial updates

What a store several runs share needs: a claim that cannot be won twice, and
writers that change one field of a record without dropping each other's.

```yaml spec
version: "0.1"
store.jobs:
  kind: kv
  scope: global
  value_schema:
    owner:    { type: string }
    attempts: { type: integer }
    log:
      type: array
      max_items: 100
      items: { type: string }

flow.work:
  inputs:
    job:    { type: string }
    worker: { type: string }
  outputs: {}
  nodes:
    claim:
      store: store.jobs
      op: set
      key: "input.job"
      if: absent
      value:
        owner: "input.worker"
        attempts: "0"
        log: "[]"
    count:
      store: store.jobs
      op: incr
      key: "input.job"
      field: attempts
    note:
      store: store.jobs
      op: append
      key: "input.job"
      field: log
      value: "'claimed by ' + input.worker"
  edges:
    - { from: start, to: claim }
    - { from: claim, to: count, when: "claim.output.applied" }
    - { from: claim, to: end, else: true }
    - { from: count, to: note }
    - { from: note, to: end }

triggers:
  cli: { type: manual, flow: flow.work }
```

**Every `kv` key carries a `version`**, drawn from one monotonic sequence per
partition: every write that lands takes the next value, and `get` answers it
beside the value (`version` is present exactly when `found` is). A sequence
rather than a per-key counter, so a key deleted and set again never shows a
version it showed before.

**A write may be conditional.** `set`, `merge`, `append` and `incr` take one of
`if: absent`, `if: present`, or `if_version: <CEL int>`; `delete` takes
`if_version:`. Both at once is `conflicting-keys`. A condition that does not hold
**writes nothing** and answers `applied: false` — with the key's current
`version` where it holds one, a declined `delete` included — which is a result
an edge routes on, as `claim` above does, never a node error. A `delete` that
applied answers no `version`: the key holds nothing. `if_version:` on a `vector`
or `blob` `delete` is `invalid-value`: a version is a `kv` key's.

**`merge`, `append` and `incr` write part of a value.** Each reads what the key
holds, applies itself, validates the **whole** result against `value_schema`, and
commits, in one transaction serialized against every other write to that key — so
two runs appending at once keep both items, and two runs merging different fields
keep both fields. On an absent key `merge` creates the value and has to supply
everything the schema requires, `append` starts from `[]` and `incr` from `0`.
`unique: true` on an `append` adds only an item the array does not already hold.
A result the schema refuses **fails the node**, names the field, and changes
nothing. `validate` refuses an `append` to a field that is not an array and an
`incr` of one that is not an `integer` or `number` (`type-mismatch`), and a field
the schema does not declare (`unknown-field`).

**Every `kv` write answers `applied`, `version` and `deduped`**; a partial update
also answers the whole `value` it left. A retried write — the same idempotency
key, the same attempt — is answered with its first attempt's whole answer and
`deduped: true`, so a retried claim that won answers `applied: true` again rather
than losing to itself.

**Schema evolution.** `value_schema` is checked on write: by CEL typing at
`validate` for a node, by the emitted Zod at run time for a tool's arguments. A
`get` returns the stored JSON as it is, so a value written under an older schema
is not refused there — it fails only when a CEL read reaches a field it lacks. A
`merge`, `append` or `incr` validates the value it is building on too, so under a
changed schema it **fails naming the field** the stored value lacks; a `set` of
the whole value is the repair.

## Writes inside a `map`

A store **write** performed inside a `map`-dispatched instance must be one of
two forms:

1. an **item-derived key** — the `key:` expression *reads*
   `execution.item_index`, or reads an `input.<field>` whose binding *at that
   dispatch site* references the item binding or the index. Legal for every
   kind.
2. a **keyed `kv` write** — any write on a `kv` store (`set`, `delete`,
   `merge`, `append`, `incr`) with any legal key, including one constant across
   instances.

Anything else — a `vector` or `blob` write whose key is not item-derived — is
`unkeyed-map-write`. A field bound from `state.*`, from the enclosing flow's
`input.*`, or from a literal is **not** item-derived, however item-derived its
*name* looks: N documents would land on one vector key.

*Reads* the index, not *is* it: every op's `key` is a CEL string and
`execution.item_index` is an integer, so `key: "execution.item_index"` satisfies
this rule and then fails type-checking with `type-mismatch`. The spelling that
satisfies both is a string-valued expression that reads the index —
`key: "state.tasks[execution.item_index]"`.

The `kv` exemption is not a loophole. A `kv` write addresses a slot the author
named: a `set` replaces the whole value there, so concurrent instances setting
one key are a declared overwrite — the store-side counterpart of
`reduce: last_wins` — and a `merge`, `append` or `incr` is serialized against
every other write to the key, so a `map` whose instances all append to one key
keeps every instance's item. A `blob` or `vector` write has no such story: it is
bulk content at a document id.

Store writes are at-least-once and carry an idempotency key derived from the
execution id and the store node's flattened instance path.

## Agent-attached stores

| kind | `agent_access: read` | `read_write` (default) | `full` |
|---|---|---|---|
| `kv` | `<name>_get` | `<name>_get`, `<name>_set` | those, and `<name>_merge`, `<name>_append`, `<name>_incr`, `<name>_delete` |
| `vector` | `<name>_search` | `<name>_search`, `<name>_upsert` | as `read_write` |
| `blob` | `<name>_get`, `<name>_list` | `<name>_get`, `<name>_list`, `<name>_put` | as `read_write` |

`full` is opt-in because the writes past a whole-value `set` are a different
authority: a model that can delete a key can remove what a run depends on. It
grants all four on every `kv` store, whatever `value_schema` declares: where it
declares no array field, `<name>_append` has no field to name and refuses every
call, and so does `<name>_incr` where it declares no `integer` or `number` one —
each says so in its own description. `<name>_set` takes the conditions
at `read_write` — a condition narrows a write — as `if` (`absent`, `present`, or
empty for none) and `if_version` (`-1` for none); an append's field and item
arrive together as `item`, naming one array field: `{"item": {"tags": "x"}}`.

`<name>` is the store's local name, so `store.user_prefs` gives `user_prefs_get`.
A synthesized name colliding with an attached tool name is
`tool-name-collision`.

Each tool's arguments are its op row with the expressions replaced by what the
model supplies, held to that schema **before the store sees them**. A call the
schema refuses is returned to the model as an error tool result and reaches no
backend — a refused call leaves no store record. So is a call naming both
conditions, and a partial update whose result `value_schema` refuses: the
arguments were the model's, and nothing was written. A **backend** that could
not answer fails the agent node instead.

Normative source: `docs/grammar.md` §8.8, §11, §11.1–11.5, §14.3
