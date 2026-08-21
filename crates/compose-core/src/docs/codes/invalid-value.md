# invalid-value

## What it protects

The value is well-typed and still not legal here. Shape rules the type system
cannot state live behind this code: an `http` trigger's `path:` must start with
`/`, a cron expression must have five fields, `else:` takes the literal `true`,
a `pattern:` must be a valid RE2 expression, an `expect_exit` list must be
non-empty and distinct.

Most of these refuse an **inert** value — one that changes nothing about what
runs. `else: false` says nothing, because an unguarded edge is already
unconditional; `expect_exit: []` accepts no outcome at all. A key that inverts
or negates its own purpose is a mistake, not a configuration.

## A spec that triggers it

```yaml triggers
version: "0.1"
triggers:
  on_request:
    type: http
    flow: flow.f
    path: review

flow.f:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: end }
```

## The fix

Read the message: it names the constraint and what was found, and often the
default. Above, write `path: /review`; dropping the key entirely would have
given the default, `/triggers/on_request`.

Grammar: `docs/grammar.md` §3.3, §7.2, §13.3, Decisions D61, D100. Topic:
`agent-compose docs triggers`.
