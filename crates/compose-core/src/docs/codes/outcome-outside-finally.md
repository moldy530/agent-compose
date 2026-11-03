# outcome-outside-finally

## What it protects

`execution.outcome` and `execution.error` say **how a flow instance ended** —
`completed`, `failed` or `cancelled`, and the error that ended it — so they
exist in exactly one place: the node a flow's `finally:` names, which runs once
when the instance has ended. Every other expression in the flow — an edge guard,
another node's `input:`, a store key — runs *before* the end, while there is no
outcome yet to read.

So reading either one anywhere but inside the finally node is refused by name,
rather than accepted and answered with a value that could only be a guess. The
field is spelled right and is a member of `execution` — at the other site — so
the repair is moving the read, never correcting the spelling.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
    record:
      exec: { command: "logger", args: ["-t", "flow.f"] }
      input: { outcome: "execution.outcome" }
  edges:
    - { from: start, to: work }
    - { from: work, to: record }
    - { from: record, to: end }
```

## The fix

**Name the node that reads it as the flow's `finally:`.** It then runs once when
the instance ends, whatever the outcome, and `execution.outcome` is in its
scope. A finally node is outside routing, so it takes no edges:

```yaml spec
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    work:
      exec: { command: "true" }
    record:
      exec: { command: "logger", args: ["-t", "flow.f"] }
      input: { outcome: "execution.outcome" }
  edges:
    - { from: start, to: work }
    - { from: work, to: end }
  finally: record
```

Where what you wanted was to route on whether a node succeeded, route on what
that node answered — its output in an edge guard, or a channel it wrote — and
give the node an `on_error:` if a failure should take a branch of its own.

Grammar: `docs/grammar.md` §4.1, §7.9, Decision D159. Topic:
`agent-compose docs flows`.
