# invalid-import-path

## What it protects

`imports:` entries have one portable spelling, so a path means the same thing in
the artifact, on a command line and in a diagnostic on every host: `/`-separated
segments, each `.`, `..` or a name matching `[A-Za-z0-9_][A-Za-z0-9_.-]*`, the
last ending in `.yml` or `.yaml`. No backslashes, no whitespace, no leading `/`,
no URLs, no globs.

Two further rules share this code. Entries must be **unique** after
normalization — a repeated import adds nothing, because import order does not
affect semantics — and must not name the entrypoint itself.

A path may contain `..`, but every prefix must stay **inside the project root**.
A path that climbs out and returns is refused even where it lands back inside:
whether it re-enters the same project depends on the checkout's parent
directory, which the spec cannot see.

## A spec that triggers it

```yaml triggers
version: "0.1"

imports:
  - agents/reviewer.yml
  - agents/reviewer.yml
```

## The fix

Drop the duplicate, or rewrite the path in its in-root spelling
(`models.yml`, not `../project/models.yml`). For a file genuinely outside the
project, copy it in or make it its own project: the root is a fence.

Grammar: `docs/grammar.md` §1.4, Decision D80. Topic:
`agent-compose docs getting-started`.
