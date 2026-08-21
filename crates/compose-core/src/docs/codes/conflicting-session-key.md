# conflicting-session-key

## What it protects

Two `manual` triggers naming one flow with different `session_key:` expressions.
A manual trigger is the CLI entry for its flow, and a flow has one CLI entry —
so two answers to "where does this run's session identity come from" is two
answers for one command line, with nothing to choose between them.

A `session_key:` on a manual trigger defaults to `"payload.session"`, which is
the CLI's `--session <key>`. Two triggers that both take the default agree, and
never reach this check.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  cli:
    type: manual
    flow: flow.f
    session_key: "payload.session"
  batch:
    type: manual
    flow: flow.f
    session_key: "'fixed'"
```

## The fix

Keep one manual trigger per flow. Declaring one at all is optional — the same
CLI entry, with the same defaulted `session_key:`, exists implicitly for every
flow — so the usual fix is to delete the one that was added later.

Where the two really are different entry points, they want different **flows**,
or an `http` trigger each: `http` triggers are per-route and may each carry their
own `session_key:` without colliding.

Grammar: `docs/grammar.md` §13, §13.2, Decision D64. Topic:
`agent-compose docs triggers`.
