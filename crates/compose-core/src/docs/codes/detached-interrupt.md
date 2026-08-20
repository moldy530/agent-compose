# detached-interrupt

## What it protects

A **detached** `map` dispatch whose target reaches a `human` node. The join
counts the dispatch resolved the moment it is issued, so the execution that
issued it can finish while the instance is still in flight — and a pause inside
one is a question whose answer nothing is left to receive. The wait belongs to
an execution that is not waiting for it, and is dropped when that execution
ends.

This one is **target-independent**, unlike the checkpointing rule that also
governs `detach:`. And "reaches" is the composition-wide relation, so the
`human` node a reader has to remove may be nowhere near the `detach:` key that
makes it unanswerable: inside a flow the target instantiates, or inside a flow
attached to an agent it reaches.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  requests:
    type: array
    max_items: 9
    items: { type: string }
flow.sign_off:
  inputs:
    text: { type: string }
  outputs: {}
  nodes:
    sign:
      human:
        input:
          text: { type: string }
        output:
          decision: { enum: [approve, reject] }
      input: { text: "input.text" }
  edges:
    - { from: start, to: sign }
    - { from: sign, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.requests"
        node: flow.sign_off
        detach: true
        max_concurrency: 2
        input: { text: "item" }
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
```

## The fix

**Drop `detach:`**, so the dispatch is joined and the pause has an execution
waiting on it. That is the right answer whenever the answer matters — and if a
human is being asked, it does.

Otherwise take the `human` node out of what the dispatch reaches: split the
approval into a flow with its own trigger, and let the detached dispatch do only
the fire-and-forget part.

Grammar: `docs/grammar.md` §7.7, §8.6, §8.7, Decisions D94, D118. Topics:
`agent-compose docs maps`, `agent-compose docs human`.
