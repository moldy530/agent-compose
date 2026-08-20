# unreduced-write

## What it protects

A channel without a `reduce:` policy is single-writer and sequential. Writing
one from **concurrent contexts** — two branches of a fork, or the instances of a
`map` — is refused, because two writes with no order between them have no
defined result and a replay could reproduce either.

Declaring a policy is how you say what should happen: `append` collects,
`merge` combines key-wise, `last_wins` is an explicit opt-in to overwrite.

The concurrency analysis is deliberately conservative: a guard pair it cannot
prove exclusive is treated as co-takeable, so it may ask for a policy on a
channel two branches could not really both write. Declaring the policy is the
cost, and a declared overwrite beats a silent race.

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
    max_items: 5
    items: { type: string }
  result: { type: string, default: "" }
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

Declare the policy the writers actually want. For a fan-out collecting one
result per item, that is `append`:

```yaml
state:
  result:
    type: array
    max_items: 5
    items: { type: string }
    reduce: append
```

Note what the write then supplies: an `append` channel takes **one element** per
write, which is exactly what one instance contributes.

Grammar: `docs/grammar.md` §7.6.1, §8.6, §10.2, Decisions D32, D58. Topics:
`agent-compose docs state`, `agent-compose docs maps`.
