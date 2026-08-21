# yaml-syntax

## What it protects

The file's YAML does not parse. Nothing spec-level has happened yet — no
sections were read, no names were resolved — so this is reported alone and
every other check is skipped for that file.

The message carries the parser's own account of where the structure went wrong,
which is usually a line or two *after* the mistake: an unclosed `{` is noticed
at the next thing that cannot follow it.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  draft: { type: string
```

## The fix

Read from the reported line **upwards** for the construct that was left open.
The usual causes are an unclosed `{` or `[`, inconsistent indentation under a
mapping key, and a tab character where YAML requires spaces.

One spec-specific gotcha lands here rather than on a compiler check: a `${NAME}`
reference written unquoted inside `{ … }` or `[ … ]` is a syntax error, because
YAML forbids those indicators in a plain scalar in flow context. Quote it.

The other thing worth quoting — an unquoted `version: 0.1`, which YAML reads as
a float — is *not* one of these. It parses, and the compiler reports it as
`wrong-type`.

Grammar: `docs/grammar.md` §1.1, §1.3, §4.3. Topic:
`agent-compose docs getting-started`.
