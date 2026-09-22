# process-local-store

## What it protects

One store, not several wearing one address.

A `memory`, `sqlite`, `sqlite_vec` or `local_fs` store is **opened by the
process that reaches it**. There is no server in the middle: the bytes are a heap
map, a SQLite file, or a directory, and two processes reaching that store hold
two stores. A `redis`, `postgres`, `mysql`, `chroma`, `pgvector`, `qdrant`, `s3`
or `gcs` store is the opposite — the process dials something outside itself, and
two processes reaching it are two readers of one store.

A mesh runs a placed component in **more than one process by design**. Several
workers may claim one placement — that is what a pool is — and the hub dispatches
whatever else in the composition reaches the same store. So a placement that can
open a process-local store forks it: the worker writes into its own copy under
its data directory, the hub reads from the one beside the built project, the read
comes back empty, and the flow carries on with data that is not there. Nothing
fails, which is what makes it worth a compile error.

The rule reads on **execution, not membership**, exactly as the environment
manifest does. A component can execute in a placement's process when it is a
`members:` entry, or when something placed reaches it through an attachment — an
attached `tool.*`, and every agent and tool an attached `flow.*` reaches, all of
which run inside the attaching agent's own tool loop. So the store an unplaced
agent binds is refused too, when a placed agent attaches the flow that reaches
that agent.

It refuses at **every scope**. `scope: execution` is not an exemption: an
execution spans processes the moment one of its nodes is dispatched to a worker,
so the fork is the same fork one node later. And it refuses under `--target
local`, where a hub and its workers are separate processes on one machine —
which is the deployment shape `placements:` under `local` exists to develop.

What is untouched is everything one process reaches. A composition with no
`placements:` is one process and may keep every local store it likes; so may a
store no placed component and no attachment of one ever reaches.

## A spec that triggers it

`agent.archivist` files notes in `store.notes`, and the deploy file puts the
agent on the `vault` machine:

```yaml triggers
version: "0.1"

provider.openai:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.openai
  id: gpt-4o-mini

store.notes:
  kind: kv
  scope: session
  description: What this session has already filed.
  value_schema:
    last_note: { type: string }

agent.archivist:
  model: model.smart
  prompt: File what you are given, and say what you filed.
  stores: [store.notes]
  input:
    text: { type: string }
  output:
    filed: { type: string }

flow.archive:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    file:
      agent: agent.archivist
      input:
        text: "input.text"
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

storage_backends:
  defaults:
    kv: { provider: sqlite }
```

## The fix

**Bind a dialled backend**, which for a `kv` store is a repair a build of this
release runs. `postgres` and `mysql` are implemented behind the store interface
(PRD resolved question 63): the ops, the scopes, the recorded reads and the
deduplicated writes are the same on them as on a file, and the one thing that
changes is that the hub and every worker reach the same rows. The environment
partition routes a backend's variables to the hub *and* to every placement whose
components reach a store bound to it, so a worker that opens the store is refused
at join if it has no credential for it rather than failing at its first op. One
line of the deploy file, and the two processes are two readers of one store.

```yaml deploy fixed
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  vault:
    members: [agent.archivist]

storage_backends:
  defaults:
    kv: { provider: postgres, url: "${NOTES_DB_URL}" }
```

Such a store is deliberately **multi-writer** — it takes no writer guard, because
this rule is what governs who may reach it — and per key the last write wins.

**For a `vector` or a `blob` store that repair is still a deployment rather than
an edit**: `validate` and `build` take the block, and the first store op throws —
the rest of production `storage_backends` (Redis, Chroma, pgvector, Qdrant, S3
and GCS) land behind the store plugin interface in M3 (PRD §7). For those, the
repair a build of this release runs is the other one.

**Take the component out of the mesh.** A store only the hub ever opens is a
store with one process, whatever its backend. Dropping `agent.archivist` from
`members:` says the filing happens on the hub, which is where the store is.

The **diagnostic** names the two in the other order — the networked backend
first, because that is the shape the deployment is heading for, then which of the
names it offered this release opens — so both texts leave a reader at an edit
that works, whichever they met first.

**Under `--target local` the backend repair is not available at all**, and the
diagnostic says so: `local` substitutes local storage for every store
unconditionally and refuses a `storage_backends:` block outright, so there is no
line to edit. A `local` mesh that has to share a store is a target of its own — a
`deploy/<name>.yml` naming a dialled backend — and until there is one, the
component that binds the store belongs outside `placements:`.

Grammar: `docs/grammar.md` §14.1 rule 5, §11.3, §14.3, Decision D131. PRD:
resolved question 45. Protocol: `docs/distributed.md` §1, §9.1. Topic:
`agent-compose docs targets`.
