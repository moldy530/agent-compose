# duplicate-route

## What it protects

Two `http` triggers declaring one route — the same effective `path:` at the same
`method:`. The generated app mounts one route per declared trigger and cannot
dispatch one method-and-path pair two ways, so this is an app that refuses to
start: a guaranteed runtime failure both trigger objects show.

Remember `path:` defaults to `/triggers/<trigger name>`, so two triggers only
collide when at least one of them names a path explicitly.

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
    goal: { type: string }
  output:
    result: { type: string }
flow.f:
  inputs:
    goal: { type: string }
  outputs: {}
  nodes:
    n:
      agent: agent.a
      input: { goal: "input.goal" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
triggers:
  on_ask:
    type: http
    flow: flow.f
    path: /ask
    input:
      goal: "payload.body.goal"
  on_retry:
    type: http
    flow: flow.f
    path: /ask
    input:
      goal: "payload.body.goal"
```

## The fix

Give one of them its own `path:` or `method:`. The diagnostic names both
triggers and which one claimed the route first.

Where the two really are one entry point with different behaviour, that is a
routing decision and belongs inside the flow: one trigger, one route, and an
edge guard reading `input` to tell the cases apart.

Grammar: `docs/grammar.md` §13.3. Topic: `agent-compose docs triggers`.
