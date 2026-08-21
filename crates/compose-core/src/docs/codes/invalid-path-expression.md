# invalid-path-expression

## What it protects

`map.over` takes a **path expression**, a strict subset of CEL: a root
identifier followed by field selections and integer literal indexes. No calls,
no operators.

The reason is that the validator must statically resolve the array's schema — to
prove the `max_items` bound and to narrow the variants of a discriminated union.
A call makes that undecidable, and an undecidable `over` means an unbounded
fan-out the compiler cannot refuse.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.planner:
  model: model.m
  prompt: List the tasks this request needs.
  output:
    tasks:
      type: array
      max_items: 8
      items: { type: string }
agent.worker:
  model: model.m
  prompt: Work.
  output:
    result: { type: string }

flow.demo:
  inputs:
    request: { type: string, min_length: 1 }
  outputs: {}
  nodes:
    plan:
      agent: agent.planner
      input: "input.request"
    work:
      map:
        over: "plan.output.tasks.filter(t, t != '')"
        node: agent.worker
        max_concurrency: 4
  edges:
    - { from: start, to: plan }
    - { from: plan, to: work }
    - { from: work, to: end }
```

## The fix

Move the filtering **into the producer**. Have the upstream node emit exactly
the array you want to fan out over — a second output field, or a narrower
schema — and point `over:` at that.

That also makes the selection visible in a trace and in a plan, which a filter
buried in a map expression is not.

It is **two** edits, not one. `over: "plan.output.ready_tasks"` on its own names
a field the producer does not declare, so repointing the map without widening
the agent's `output:` trades this diagnostic for the next one — the array has to
exist before a path can select it.

## The fix, applied

The spec above with `agent.planner` emitting a second array and `work`'s `over:`
reading it. Nothing else moves.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.planner:
  model: model.m
  prompt: List the tasks this request needs.
  output:
    tasks:
      type: array
      max_items: 8
      items: { type: string }
    ready_tasks:
      description: The subset of `tasks` that can start now.
      type: array
      max_items: 8
      items: { type: string }
agent.worker:
  model: model.m
  prompt: Work.
  output:
    result: { type: string }

flow.demo:
  inputs:
    request: { type: string, min_length: 1 }
  outputs: {}
  nodes:
    plan:
      agent: agent.planner
      input: "input.request"
    work:
      map:
        over: "plan.output.ready_tasks"
        node: agent.worker
        max_concurrency: 4
  edges:
    - { from: start, to: plan }
    - { from: plan, to: work }
    - { from: work, to: end }
```

Grammar: `docs/grammar.md` §4.2, §8.6, Decision D43. Topic:
`agent-compose docs maps`.
