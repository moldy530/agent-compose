# unsupported-version

## What it protects

`version:` is the spec's own semver, and the compiler declares the range it
supports. Old syntax is **never silently reinterpreted**: a spec written for
another version is refused with a pointer at the migration, rather than parsed
under today's rules and quietly given a new meaning.

## A spec that triggers it

```yaml triggers
version: "9.9"
```

## The fix

Set `version:` to a version this build supports — the message names the set. For
a spec genuinely written against an older version, run `agent-compose migrate`
rather than editing the field by hand: a version bump that broke something comes
with a codemod.

Remember the field must be **quoted**. Unquoted `0.1` is a YAML float and never
reaches this check.

Grammar: `docs/grammar.md` §1.3. Topic:
`agent-compose docs getting-started`.
