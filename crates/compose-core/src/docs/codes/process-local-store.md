# process-local-store

## What it protects

A placement moves where a component **executes**. It moves nothing else — and a
store on a process-local backend is the one thing that makes that difference
visible as silent data loss.

`memory`, `sqlite`, `sqlite_vec` and `local_fs` are a heap, a file, and a
directory *inside the process that opened them*. Two processes that open "the
same" one open two. So a placed agent that reaches such a store reads and writes
a copy on the worker's disk, under the artifact tree that worker unpacked, while
the hub reads and writes its own — and nothing fails. The write succeeds. The
hub's read answers empty. The flow carries on with data that is not there.

The networked backends of grammar §14.3 — `redis`, `postgres`, `chroma`,
`pgvector`, `qdrant`, `s3`, `gcs` — are addressed by a URL, so a hub and a worker
pointed at one are pointed at the same data. That is the premise the whole
distributed model rests on: `docs/distributed.md` §1 says executions "share
nothing by construction — global-scope stores are already external backends",
and this check is that sentence turned into a rule.

**Every scope is refused, not only `global`.** `scope: execution` is the
sharpest case, because one execution is exactly what a hub and a worker are
collaborating on: a `store:` node on the hub reading what a placed agent wrote is
the ordinary shape, and it would read nothing. `scope: session` is the same
failure over a longer lifetime, and two workers claiming one placement diverge
from each other as well as from the hub. What decides the refusal is the
**backend**, never the scope.

What the check walks is each placed `agent.*`: its own `stores:`, and every store
reached by a `flow.*` in its `tools:`. A flow attached as a tool starts its
instance inside the agent's own loop, so a `store:` node in that flow runs on the
worker too. A placed `tool.*` is left alone, because a tool reaches no store — it
is a process or a request.

Note that this is decided **per target**. `--target local` substitutes local
storage for every store unconditionally, so a composition that is fine under
`staging` — where the store's alias resolves to `chroma` — is refused under
`local` if the same component is placed there. That is the same target-dependent
shape the binding checks of grammar §11.3 already have, and it is honest: under
`local` the store really is a file.

## A spec that triggers it

`agent.archivist` keeps notes in `store.notes`, and the deploy file puts the
agent on a worker while leaving the store on the target's built-in SQLite:

```yaml triggers
version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

store.notes:
  kind: kv
  scope: execution
  description: What has been filed so far in this run.
  value_schema:
    filed: { type: string }

agent.archivist:
  model: model.smart
  prompt: File what you are given, and say what you filed.
  stores: [store.notes]
  input:
    document: { type: string }
  output:
    verdict: { type: string }

flow.archive:
  inputs:
    document: { type: string }
  outputs: {}
  nodes:
    file:
      agent: agent.archivist
      input:
        document: "input.document"
  edges:
    - { from: start, to: file }
    - { from: file, to: end }
```

```yaml deploy mesh
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  vault:
    members: [agent.archivist]
```

## The fix

**Bind the store to a backend both processes share.** The store keeps its
declaration — `kind:`, `scope:`, its schema — and the deploy layer decides where
the data lives, which is what the split is for. A per-kind default covers every
`kv` store in the target; an alias named by the store's own `backend:` covers one.

```yaml deploy shared
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  vault:
    members: [agent.archivist]

storage_backends:
  defaults:
    kv:
      provider: redis
      url: ${REDIS_URL}
```

**Or take the component out of the placement.** A component in no placement
executes on the hub, beside the store — which is the right answer when the
placement was about capability the component does not actually need.

Grammar: `docs/grammar.md` §11.3, §14.1, §14.3. Topic:
`agent-compose docs targets`. Protocol: `docs/distributed.md` §1, §9.1.
