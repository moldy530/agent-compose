# unsupported-version

## What it protects

`version:` is the spec's own semver, and the compiler declares the range it
supports. Old syntax is **never silently reinterpreted**: a spec written for
another version is refused, and told the range this build takes, rather than
parsed under today's rules and quietly given a new meaning.

## A spec that triggers it

```yaml triggers
version: "9.9"
```

## The fix

Set `version:` to a version this build supports — the message names the set.
Editing the field is not by itself a migration, though: a spec genuinely
written against an older version has syntax to bring forward too, and refusing
it here is what stops today's compiler reading yesterday's meaning.
`agent-compose --version` says which build you are holding, and the release
notes for the versions in between say what moved.

Remember the field must be **quoted**. Unquoted `0.1` is a YAML float and never
reaches this check.

Grammar: `docs/grammar.md` §1.3. Topic:
`agent-compose docs getting-started`.
