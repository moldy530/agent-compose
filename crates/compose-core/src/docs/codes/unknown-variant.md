# unknown-variant

## What it protects

A value is not one of a closed set. Trigger types, provider kinds, store kinds
and scopes, reduce policies, `on_error` strategies, `context:`, `agent_access:`,
`route_on` conditions, `format:` names — each is a fixed vocabulary, and the
compiler knows all of them.

Closed vocabularies are what make the rest of the checking possible: routing
exhaustiveness needs a finite variant set, capability checking needs to know
which plugin a `kind:` selects.

## A spec that triggers it

```yaml triggers
version: "0.1"
triggers:
  on_request:
    type: webhook
    flow: flow.f

flow.f:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
```

## The fix

Use a member of the set — the `help:` line lists them, and suggests one when the
spelling is close. Above, the four trigger types are `manual`, `http`,
`schedule` and `event`; an inbound webhook is an `http` trigger.

Grammar: `docs/grammar.md` §11.1, §12.1, §13.1. Topic:
`agent-compose docs triggers`.
