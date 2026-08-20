# unexpected-env-ref

## What it protects

An environment reference appears on a surface that never interpolates. Every
string surface in this grammar is in exactly one of three classes, and class 3 —
**no refs** — is the default: prompts, every `description:`, every part of a
schema, every CEL expression, a model `id`, everything inside `settings:`,
`version:`, `imports:` entries, a trigger's `path:` and `cron:`, every
identifier and reference position.

An unescaped token there is an error rather than text that silently survives,
because whoever wrote it expected a substitution. Discovering at run time that a
prompt says six literal characters is a worse outcome than a diagnostic.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  home:
    type: string
    default: "${HOME}"
```

## The fix

If you wanted a **literal**, escape it: `$${HOME}`. The escape is recognized
wherever a `${…}` token is.

If you wanted a **value from the environment**, this is not the surface for it.
Interpolation happens on `http` urls and headers, on the whole `exec:` block,
and on connection configuration. Dynamic data reaches a graph through its inputs
and its state, not through the process environment.

Grammar: `docs/grammar.md` §4.3, Decisions D41, D92. Topic:
`agent-compose docs cel`.
