# conflicting-writes

## What it protects

A node's **effective write map** — every output field paired with the channel it
actually writes, name-based destinations included — must be injective. Two
fields of one node landing on one channel would be two writes from one writer
with no order between them, which the canonical write order has nothing to say
about: it orders *writers*, and within one writer there is meant to be at most
one write per channel.

Both spellings are caught. Two remaps onto one channel is the obvious one. The
other is a remap landing on a **sibling's name-based destination** — output
`{a, b}` with `writes: { a: b }` and a declared channel `b`, whose effective map
is `a → b` and `b → b`. That second form passes any check that reads `writes:`
alone, which is why the rule is stated over the effective map.

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
  summary: { type: string, default: "" }
agent.a:
  model: model.m
  prompt: Do the thing.
  input:
    text: { type: string }
  output:
    result:  { type: string }
    summary: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "'x'" }
      writes: { result: summary }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Send one of them somewhere else. Either remap the colliding sibling to a channel
of its own — `writes: { result: verdict }`, with a `verdict` channel declared —
or drop the remap, which leaves `result` node-scoped and `summary` writing the
one channel there is. Remember a remapped field is **not** also written to its
same-named channel: the collision above is `result → summary` meeting
`summary → summary`, and either write moving settles it.

Grammar: `docs/grammar.md` §7.6.4, §8.0, Decision D93. Topic:
`agent-compose docs state`.
