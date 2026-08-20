# non-exhaustive

## What it protects

A node routing on an enum field leaves a variant with nowhere to go. At run time
that is a "no viable route" failure on exactly the pass where the model emits
the uncovered value — the worst kind of bug, because it is invisible until
production data reaches it.

Coverage is decided over a **closed set of guard shapes**, and the last row of
that table is what most reports come down to: a term the table does not
recognize — a `size()` call, any function, a comparison against a different
field — contributes **no** guarantee and excludes **no** variant. So
`n.output.f == 'a' && size(state.xs) > 0` guarantees nothing at all.

The same code covers a `map` whose `routes:` leave a variant of the item union
unrouted with no `default:`.

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
    review:  { agent: agent.a, input: "'x'" }
    publish: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: review }
    - { from: review, to: publish, when: "review.output.verdict == 'approve'" }
    - { from: review, to: publish, when: "review.output.verdict == 'revise'" }
    - { from: publish, to: end }
```

## The fix

Two ways, and both are one line.

**Add a catch-all**: an outgoing edge that is unconditional or carries
`else: true` fires for every value of every field.

```yaml
- { from: review, to: publish, else: true }
```

**Or cover the field exactly**: rewrite the guards so the union of what they
*guarantee* is the whole variant set — `!=` and `in [...]` are the two shapes
that cover several variants at once.

For a `map`, add the missing `routes:` entry or a `default:`.

Grammar: `docs/grammar.md` §7.3.1, §8.6, Decisions D30, D82. Topics:
`agent-compose docs routing`, `agent-compose docs maps`.
