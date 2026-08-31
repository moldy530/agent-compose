# unsupported-placement

## What it protects

A `placements:` entry whose `members:` list names a `flow.*`. v1 places the
leaves that hold a machine's capability — `agent.*` and `tool.*` — and placing a
flow is deferred, named, in the PRD rather than left to be discovered.

The reason is what a flow *is*. An agent is one model call and a tool is one
implementation: each is a unit of work a worker can be handed, run, and answer
for. A flow is a subgraph — its own nodes, edges, routing, joins and possibly a
`human` pause — and the thing that schedules those is the hub, which owns the
scheduler, the journal and the wait board. Placing a flow would mean either
shipping the scheduler to the worker (a second scheduler, and a second journal
behind it) or placing every node the flow reaches and calling the result one
claim. Both are real designs and neither is v1's.

So the refusal is a deferral rather than a correction: the composition is
well-formed, and what the author asked for is a feature this release does not
have. Placing the nodes the flow reaches gets the same machine running the same
work, one address at a time.

## A spec that triggers it

The composition is ordinary — the placement is what refuses:

```yaml triggers
version: "0.1"

provider.vendor:
  kind: openai
  api_key: ${OPENAI_API_KEY}

model.smart:
  provider: provider.vendor
  id: gpt-4o-mini

agent.signer:
  model: model.smart
  prompt: Describe what to sign, given the build log.
  input:
    log: { type: string }
  output:
    summary: { type: string }

flow.release:
  outputs: {}
  nodes:
    sign:
      agent: agent.signer
      input:
        log: "input.log"
  inputs:
    log: { type: string }
  edges:
    - { from: start, to: sign }
    - { from: sign, to: end }
```

```yaml deploy mesh
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  mac:
    members: [flow.release]
```

## The fix

**Name the components the flow reaches.** A flow is a module, and the work it
does is the agents and tools inside it. Placing those puts the same work on the
same machine, and it puts it there one address at a time — which is also what
makes the placement readable: `members: [agent.signer]` says which capability
the machine is being asked for.

**Or leave it unplaced.** A component in no placement executes on the hub, which
is the default and needs no entry. A flow whose nodes all run on the hub is the
ordinary case.

```yaml deploy fixed
version: "0.1"

hub:
  join_token: ${MESH_JOIN_TOKEN}

placements:
  mac:
    members: [agent.signer]
    description: the machine with the signing keys
```

Grammar: `docs/grammar.md` §14.1, §14.2, Decisions D128, D129. Topic:
`agent-compose docs targets`. Protocol: `docs/distributed.md`.
