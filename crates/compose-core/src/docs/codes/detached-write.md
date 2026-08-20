# detached-write

## What it protects

A detached dispatch writes no state, however the write is spelled.

`detach: true` is a statement about the **join**: the map counts the dispatch
resolved the moment it is issued and never waits for its outcome, so the map
node can complete — and its outgoing edges fire — while the instance is still in
flight. Anything it wrote would land, or not, *after* everything downstream had
already read the channel.

Two spellings reach this. A `writes:` remap beside `detach: true` is refused by
the parser. The other only a whole composition shows: the target's output field
and a declared channel share a name, so the write is made by name-based wiring
with no `writes:` key to refuse.

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
    max_items: 9
    items: { type: string }
  note:
    type: array
    max_items: 9
    items: { type: string }
    reduce: append
agent.sink:
  model: model.m
  prompt: Record.
  input:
    text: { type: string }
  output:
    note: { type: string }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        node: agent.sink
        detach: true
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
```

## The fix

Decide which of the two you want. **Drop `detach:`** if the result matters — the
map then joins on every instance, and the write is an ordinary reduced write.
**Keep `detach:`** if it does not, and send the result out through the target
itself: a detached sink's whole job is the effect it performs, not what it
returns.

Renaming the channel is not a fix; the rule is about the write, not the name.

Grammar: `docs/grammar.md` §8.6, Decisions D31, D94. Topic:
`agent-compose docs maps`.
