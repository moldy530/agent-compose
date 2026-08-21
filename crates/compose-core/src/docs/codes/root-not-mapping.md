# root-not-mapping

## What it protects

The document root must be a mapping of sections and definition keys. A sequence
or a scalar at the root is not a spec file — most often it is a fragment saved
under the wrong name, or an `imports:` list written without its key.

## A spec that triggers it

```yaml triggers
- providers.yml
- models.yml
```

## The fix

Give the content its key. The example above is an `imports:` list that lost its
heading:

```yaml
version: "0.1"
imports:
  - providers.yml
  - models.yml
```

Every top-level key of a spec file is either a section (`version`, `imports`,
`defaults`, `state`, `triggers`) or a typed-address definition key
(`agent.reviewer:`, `flow.review_loop:`).

Grammar: `docs/grammar.md` §1.1, §1.5. Topic:
`agent-compose docs getting-started`.
