# multiple-documents

## What it protects

A spec file holds **exactly one** YAML document. A `---` separator introducing a
second one is refused rather than silently ignored or silently merged.

Both alternatives are worse than the error. Reading only the first document
would drop definitions the author wrote and expected to be in the graph; merging
would invent a rule about which document wins per key. One file, one document,
one answer.

## A spec that triggers it

```yaml triggers
version: "0.1"
---
version: "0.1"
```

## The fix

Split the second document into a file of its own and add it to the entrypoint's
`imports:`. That is what the import list is for, and it costs one line.

If the `---` was a leading document-start marker rather than a separator, it is
legal at the top of the file; the error is about a *second* document.

Grammar: `docs/grammar.md` §1.1, §1.4. Topic:
`agent-compose docs getting-started`.
