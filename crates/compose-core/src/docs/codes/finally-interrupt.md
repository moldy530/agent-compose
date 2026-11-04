# finally-interrupt

## What it protects

A flow's `finally:` node runs once when its instance ends, and its whole point is
that it **finishes**: a release that always happens, a record that is always
written. A `human` node is a pause — a run that reaches one with nobody to
answer parks, and stays parked until somebody does — so a finally node that is
one, or that reaches one, is a step that must always finish waiting on a person
who may never come.

"Reaches" is the composition-wide relation: the `human` node may be inside a
flow the finally node instantiates, inside a flow it dispatches as a `map`, or
inside a `flow.*` tool of an agent it calls.

## A spec that triggers it

```yaml triggers
version: "0.1"
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
    work:
      exec: { command: "true" }
    wrap_up:
      flow: flow.sign_off
      input: { text: "'the work is done'" }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: wrap_up
```

The pause can sit further off than a flow the finally node instantiates — behind
the `flow.*` tools of an agent it calls, where nothing in the finally node's own
flow names a `human` node at all:

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.recorder:
  model: model.m
  prompt: Record how the work ended, and have it signed off.
  tools: [flow.sign_off]
  input:
    outcome: { type: string }
  output:
    note: { type: string }
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
    work:
      exec: { command: "true" }
    wrap_up:
      agent: agent.recorder
      input: { outcome: "execution.outcome" }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: wrap_up
```

## The fix

**Take the pause out of what the finally node runs.** Ask the question in the
body, where a pause is an ordinary wait, and keep the finally node to the work
that has to happen whatever was answered:

```yaml spec
version: "0.1"
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
    work:
      exec: { command: "true" }
    ask:
      flow: flow.sign_off
      input: { text: "'the work is done'" }
    wrap_up:
      exec: { command: "true" }
  edges:
    - { from: start, to: work }
    - { from: work, to: ask }
    - { from: ask, to: end }
  finally: wrap_up
```

Grammar: `docs/grammar.md` §7.7, §7.9, §8.7, Decision D159. Topics:
`agent-compose docs flows`, `agent-compose docs human`.
