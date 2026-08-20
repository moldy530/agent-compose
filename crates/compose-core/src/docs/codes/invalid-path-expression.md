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
agent.worker:
  model: model.m
  prompt: Work.
  output:
    result: { type: string }

flow.demo:
  outputs: {}
  nodes:
    work:
      map:
        over: "plan.output.tasks.filter(t, t.ready)"
        node: agent.worker
        max_concurrency: 4
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
```

## The fix

Move the filtering **into the producer**. Have the upstream node emit exactly
the array you want to fan out over — a second output field, or a narrower
schema — and point `over:` at that:

```yaml
over: "plan.output.ready_tasks"
```

That also makes the selection visible in a trace and in a plan, which a filter
buried in a map expression is not.

Grammar: `docs/grammar.md` §4.2, §8.6, Decision D43. Topic:
`agent-compose docs maps`.
