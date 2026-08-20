# unreachable-node

## What it protects

A node no execution can reach is code with no way to run. Usually it is a
leftover from a refactor, or an edge that was meant to point at it and points
somewhere else.

Reachability is computed per flow, from that flow's `start`, over three
relations: edges, `on_error: { fallback: … }`, and a `human` node's
`on_timeout:`. The last two are load-bearing — a dedicated `cleanup` node
reached solely by a fallback is an ordinary pattern, and an edges-only reading
would reject it.

Guards are ignored. Whether an edge fires is a runtime question; this check asks
whether a node is *ever* addressable.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise, escalate] }
flow.f:
  outputs: {}
  nodes:
    n:      { agent: agent.a, input: "'x'" }
    orphan: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
    - { from: orphan, to: end }
```

`orphan` has an edge *out*. Nothing addresses it.

## The fix

Give it an inbound edge, name it as an `on_error` fallback or a `human` node's
`on_timeout:` target, or delete it. Deleting is the right answer more often than
it looks: a node nothing reaches has never run, so nothing depends on it.

Note this is about a node inside one flow. A **flow** no trigger names is not an
error — every flow is runnable from the CLI, and flows are also reached through
`flow:` nodes and tool attachment.

Grammar: `docs/grammar.md` §7.8, Decision D95. Topic:
`agent-compose docs cycles`.
