# coalesced-in-key

## What it protects

A trigger whose `concurrency:` declares `debounce:` coalesces the arrivals for
one group into one execution: each arrival opens or extends a pending intake
row holding the newest payload, and when the debounce closes one execution
starts with that payload and `payload.coalesced` — the number of arrivals it
closed over (PRD resolved q77 ruling c).

That count exists only once the debounce has closed. Two of a trigger's
expressions are evaluated **before** then, as each arrival lands:

* `dedupe_key:`, which decides whether the arrival is a repeat of one already
  admitted — a repeat is answered and starts nothing, so it never joins a
  debounce at all;
* `concurrency.key:`, which decides **which** group's debounce the arrival
  joins.

An expression in either that read `payload.coalesced` would be defined in terms
of the coalescing it decides, and there is no value for it to read. `input:`
and `session_key:` are bound when the execution starts — after the debounce has
closed — so they read it freely.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.rebuild:
  inputs:
    branch: { type: string }
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  pushes:
    type: http
    flow: flow.rebuild
    input: { branch: "payload.body.branch" }
    concurrency:
      key: "payload.coalesced > 1 ? 'busy' : payload.body.branch"
      debounce: 30s
```

## The fix

Key the arrival off what it carries, and read the count where the execution's
inputs are bound:

```yaml spec
version: "0.1"
flow.rebuild:
  inputs:
    branch: { type: string }
    pushes: { type: integer }
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
triggers:
  pushes:
    type: http
    flow: flow.rebuild
    input:
      branch: "payload.body.branch"
      pushes: "payload.coalesced"
    concurrency:
      key: "payload.body.branch"
      debounce: 30s
```

Grammar: `docs/grammar.md` §13.1, Decision D163. Topic:
`agent-compose docs triggers`.
