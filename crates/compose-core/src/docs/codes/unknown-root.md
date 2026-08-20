# unknown-root

## What it protects

Every expression surface exposes a fixed set of **root identifiers**, and
referencing anything else is refused. Edge guards see `<from>.output`, `input`,
`state` and `execution`; node `input:` bindings see `input`, `state` and
`execution` — but no node output, because node configuration never reads another
node's output; a `tool.*` implementation binding sees `input` and nothing else;
a trigger's CEL sees `payload`.

The rule that surprises people most: data that must travel between nodes goes
through a **state channel**. That is what keeps node configs order-independent
and placement-agnostic.

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
      input: { text: "stat.draft" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Read the `help:` line — a near miss like the one above is suggested outright.

Where the root you wanted is a **node's output** at a node binding, that is not
a spelling problem: declare a state channel, let the producing node write it by
name, and read `state.<channel>` here.

Grammar: `docs/grammar.md` §4.1, Decision D42. Topics:
`agent-compose docs cel`, `agent-compose docs state`.
