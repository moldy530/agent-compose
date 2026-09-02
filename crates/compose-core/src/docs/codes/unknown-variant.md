# unknown-variant

## What it protects

A value is not one of a closed set. Trigger types, provider kinds, store kinds
and scopes, reduce policies, `on_error` strategies, `context:`, `agent_access:`,
`route_on` conditions, `format:` names, the four runtime built-ins — each is a
fixed vocabulary, and the compiler knows all of them.

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

The **built-in** set is the one closed vocabulary that is closed on purpose
rather than by what a plugin serves: `bash` and `files` are what a `builtin:`
binding may name — `builtin.bash` and `builtin.files` as shorthands in an
agent's `tools:` list — and the set grows by a resolved question rather than by
a release adding a name (`agent-compose docs tools`). A tool of your own goes in
a `tool.*` definition with an `exec:`, `http:`, `function:` or `module:` binding
and is attached by its address.

Grammar: `docs/grammar.md` §5.5, §6.1, §11.1, §12.1, §13.1, Decision D135.
Topics: `agent-compose docs triggers`, `agent-compose docs tools`.
