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

One **class 2** surface reports it too, for the same reason read the other way
round: a `server_tools:` field the compiler's curated table pins to a single
value (§12.1). The Messages wire's `name:` is the one you will meet — the API
pairs each dated `type:` with one fixed name and answers a request whose two
disagree with a 400 — and a nested object's `type:` is the same shape:
`user_location:` is an `approximate` one, `cache_control:` is `ephemeral`,
`container:` is `auto`. The value is decided by the entry's own `type:`, so
there is nothing for the environment to say about it, and a reference there is
redundant where the process happens to hold that constant and a refused request
everywhere else.

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

On a pinned `server_tools:` field the fix is to write the one value the
diagnostic names — `name: web_search`, `type: approximate`. A closed set of
*several* values is an ordinary interpolable string, so
`search_context_size: ${SEARCH_DEPTH}` is not this error and needs no change.

Grammar: `docs/grammar.md` §4.3, §12.1, Decisions D41, D92, D122. Topic:
`agent-compose docs cel`.
