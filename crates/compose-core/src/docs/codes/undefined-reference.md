# undefined-reference

## What it protects

A reference names nothing the composition defines. Five kinds of name reach this
code: a typed address, a flow-local node id in an edge or a control-transfer
position, a storage backend alias, an event source name, and a `map` route's
target.

This is the check that makes multi-file layout safe. The composition is exactly
the entrypoint plus its imports, so a name that resolves nowhere is either a
typo or a definition in a file nobody imported.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    run: { exec: { command: "true" } }
  edges:
    - { from: start, to: run }
    - { from: run, to: rnu }
```

## The fix

Read the `help:` line first — where a defined name is close, the diagnostic
suggests it, and a transposition like the one above is usually the whole story.

Otherwise the definition is somewhere the composition cannot see it. Check that
the file declaring it is in the entrypoint's `imports:`; imports are **not
transitive**, so a file imported by an imported file is not in the graph.

Two names resolve **per target** rather than per composition: a store's
`backend:` alias and an `event` trigger's `source:`. Both are satisfied
vacuously under `local`, so a report about one means the named target's deploy
file is missing the entry.

Grammar: `docs/grammar.md` §1.4, §2.3, §2.4, §11.3, §13.5. Topics:
`agent-compose docs getting-started`, `agent-compose docs targets`.
