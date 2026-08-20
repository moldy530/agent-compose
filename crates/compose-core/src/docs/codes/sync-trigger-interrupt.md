# sync-trigger-interrupt

## What it protects

A `respond: sync` http trigger blocks and returns the flow's outputs. A `human`
node in that flow would park the execution waiting for a person, with an HTTP
request holding the line — there is no answer the request can wait for.

The flow must therefore be statically **interrupt-free**, and "reaches" is the
composition-wide relation: its own nodes, its maps' dispatch targets, the flows
it instantiates, and the `flow.*` entries in the `tools:` list of any agent it
reaches. An interrupt inside a flow-as-tool is still an interrupt in the middle
of a synchronous request.

## A spec that triggers it

```yaml triggers
version: "0.1"
triggers:
  on_request:
    type: http
    flow: flow.f
    respond: sync
flow.f:
  outputs: {}
  nodes:
    approve:
      human:
        input: {}
        output:
          decision: { enum: [approve, reject] }
  edges:
    - { from: start, to: approve }
    - { from: approve, to: end }
```

## The fix

Declare `respond: async` and take the result through `callback:`. That is what
the async shape is for: the request returns an execution id immediately, the
pause is answered through the resume route, and the webhook fires on completion.

```yaml
triggers:
  on_request:
    type: http
    flow: flow.f
    respond: async
    callback: "payload.body.callback_url"
```

The other fix is to take the `human` node out of what the flow reaches — split
the approval into a second flow with its own trigger.

Grammar: `docs/grammar.md` §7.7, §8.7, §13.3. Topics:
`agent-compose docs human`, `agent-compose docs triggers`.
