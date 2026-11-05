# concurrency-on-manual

## What it protects

A `concurrency:` block bounds the executions a trigger's **arrivals** start: a
request on an `http` route, a `schedule` tick, an event its source produced.
Each arrival is admitted through the journal's intake ledger, and one that finds
its group full is queued, dropped, or supersedes what is running — a decision
the serving process makes and keeps, because an arrival is something that
happened to the deployment and has to be answered whether or not a slot is free.

A `manual` trigger has no arrivals. It describes the CLI entry for its flow —
`run` — which is a command somebody typed at a terminal, and a
terminal is not a queue: nothing should sit waiting in the journal for a slot
on behalf of a prompt that has already returned. So the block means nothing on
it, and a `manual` trigger that declared one would read as a bound the CLI
enforces when it enforces none of its own.

What the CLI does do is respect the groups the **other** triggers declare: a
`run` of a flow covered by a trigger's key-less group is refused at start while
that group has no free slot, naming the execution that holds it (PRD resolved
q77 ruling e).

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.nightly:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  cli:
    type: manual
    flow: flow.nightly
    concurrency:
      limit: 1
```

## The fix

Declare the block on the trigger whose arrivals it bounds. A `run` of the same
flow is then refused while that group is full:

```yaml spec
version: "0.1"
flow.nightly:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  cli:
    type: manual
    flow: flow.nightly
  nightly:
    type: schedule
    flow: flow.nightly
    cron: "0 3 * * *"
    concurrency:
      limit: 1
      on_conflict: drop
```

Grammar: `docs/grammar.md` §13.1, §13.2, Decision D163. Topics:
`agent-compose docs triggers`, `agent-compose docs cli`.
