# dead-end

## What it protects

A node has a pass on which its branch can take no outgoing edge. A branch
retires only at `end`, so a branch that gets stuck never retires and the flow
instance never reaches quiescence — it just stops having anything to do, with no
outputs materialized.

Three shapes reach this code:

- **a node with no outgoing edge at all**, which swallows its branch;
- **`on_error: skip` with every edge guarded** — a skipped node produces no
  output, so every guard reading that output is `false`, and the pass where the
  state-only guards are false too takes nothing;
- **a `max_iterations` budget with no escape** — an exhausted edge is not taken
  whatever its guard says, so the pass that spends the budget needs an edge no
  budget can withdraw.

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
    n:    { agent: agent.a, input: "'x'" }
    sink: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: sink }
```

## The fix

Give the node an exit that is guaranteed to fire — one that is unconditional, or
carries `else: true`:

```yaml
- { from: sink, to: end }
```

`- { from: <node>, to: end }` is the one-line way to say "this branch is done
here", and it is the fix for all three shapes: an escape for a budgeted cycle, a
catch-all for a skipping node, an exit for a leaf.

Grammar: `docs/grammar.md` §7.3, §7.4, §7.6.3, Decisions D19, D71. Topics:
`agent-compose docs routing`, `agent-compose docs cycles`.
