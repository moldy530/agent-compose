# unbounded-fan-out

## What it protects

A `map.over` path resolves to an array with no `max_items`. Bounding is
mandatory on **both** axes of a fan-out: `max_items` on the array bounds how
many instances can exist, and `max_concurrency` on the map node bounds how many
run at once.

An agent never spawns work — it emits an array and a `map` fans out over it — so
the array's bound is the whole of what stops a model from asking for ten
thousand model calls. `max_items` on a result schema is also enforced on the
wire: structured-output validation rejects a longer array, so the model
*cannot* return more.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
state:
  tasks:
    type: array
    items: { type: string }
agent.worker:
  model: model.m
  prompt: Do the thing.
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    work:
      map:
        over: "state.tasks"
        node: agent.worker
        max_concurrency: 4
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
```

## The fix

Declare `max_items` where the array is declared — the diagnostic's second label
points at it:

```yaml
state:
  tasks:
    type: array
    max_items: 20
    items: { type: string }
```

Pick a number you would be willing to pay for. It is a bound, not a target, and
it is the number a reviewer reads to know the worst case.

Grammar: `docs/grammar.md` §3.5, §8.6, Decision D10. Topics:
`agent-compose docs maps`, `agent-compose docs schemas`.
