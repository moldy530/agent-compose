# undefined-channel

## What it protects

A `state.*` read, a `writes:` destination, or a flow `outputs:` field names a
channel `state:` does not declare. The channel set is the composition's shared
vocabulary; a name outside it is a typo or a channel somebody forgot to declare.

The `outputs:` half is the one that surprises people. Each field of a flow's
`outputs:` is read from the **state channel of the same name** at quiescence,
and that channel must exist. There is no `returns:` binding.

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
  draft: { type: string, default: "" }
agent.a:
  model: model.m
  prompt: Do the thing.
  input:
    text: { type: string }
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { text: "state.drafts" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Read the `help:` line for a near miss, or declare the channel:

```yaml
state:
  drafts:
    type: array
    max_items: 10
    items: { type: string }
    reduce: append
```

For a flow output whose producing node writes a differently-named field, the
other fix is a `writes:` remap on that node — `writes: { summary: draft }` feeds
the `draft` channel the `outputs:` field reads.

Grammar: `docs/grammar.md` §7.5, §10.1, §10.3, Decision D53. Topic:
`agent-compose docs state`.
