# wrong-type

## What it protects

A value has the wrong **YAML kind** for its position: a string where a mapping
is required, a sequence where a scalar is, a number where a boolean is. This is
the shape check that runs before any spec-level meaning is read, so the message
names what was expected and what was found and nothing more.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  draft: string
```

## The fix

Write the construct out. A state channel is a **type node** — a mapping carrying
exactly one discriminating key — not a bare type name:

```yaml
state:
  draft: { type: string, default: "" }
```

The shorthand that does exist is the inline — YAML calls it *flow-style* —
form `{ type: string }`, which is the same mapping written on one line. It has
nothing to do with a `flow.*` definition.

Grammar: `docs/grammar.md` §3.2, §10.1. Topic:
`agent-compose docs schemas`.
