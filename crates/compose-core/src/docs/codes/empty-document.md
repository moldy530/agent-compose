# empty-document

## What it protects

A spec file is one YAML document with a mapping at its root. A file holding
nothing but comments, or nothing at all, declares no document — and since a
composition is exactly the entrypoint plus what it imports, a file contributing
nothing is more likely a mistake (a wrong path, a half-finished split) than an
intention.

## A spec that triggers it

```yaml triggers
# a file with nothing in it but this line
```

## The fix

Write the file's content, or drop its entry from `imports:`. An imported file
that is genuinely empty for now is better left out of the list until it has
something in it: the import list is the authoritative statement of what is in
the graph.

Grammar: `docs/grammar.md` §1.1, §1.4. Topic:
`agent-compose docs getting-started`.
