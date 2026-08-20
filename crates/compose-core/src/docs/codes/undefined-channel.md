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

Read the `help:` line first. Above, `drafts` is one letter from the declared
`draft`, and where the report names a near miss the repair is the name: `text:
"state.draft"`, and `state:` is left alone.

Where the channel is genuinely missing, declare it — and declare it with a type
the reads can use. That is the half with something to get wrong: this check only
asks whether the name exists, so a `drafts` declared as an array of strings
clears it and hands the next check the same binding, which wants the `string`
`text:` is declared as. The repair below is the one that clears both.

For a flow output whose producing node writes a differently-named field, the
other fix is a `writes:` remap on that node — `writes: { summary: draft }` feeds
the `draft` channel the `outputs:` field reads.

## The fix, applied

The spec above with `state:` gaining the channel the node reads, and nothing
else changed. It is the declare-it fix rather than the near-miss one because it
is the one with a type to get wrong, and it is written out rather than described
because a repair a reader cannot run is a repair they have to trust.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
state:
  draft: { type: string, default: "" }
  drafts: { type: string, default: "" }
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

Grammar: `docs/grammar.md` §7.5, §10.1, §10.3, Decision D53. Topic:
`agent-compose docs state`.
