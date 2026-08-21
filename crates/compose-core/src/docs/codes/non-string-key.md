# non-string-key

## What it protects

Mapping keys are strings. YAML admits integers, sequences and complex `? …`
keys; this grammar admits none of them, because every key position here is a
name — a section, a typed address, a node id, a field name, a channel name —
and every one of those is drawn from one identifier class.

An integer key is also a trap: `1:` and `"1":` are different keys to a YAML
loader and the same name to a reader.

## A spec that triggers it

```yaml triggers
version: "0.1"
state:
  1: { type: string }
```

## The fix

Quote the key, or rename it. Identifiers are 1–64 characters of lowercase
letters, digits and `_`, starting with a **letter** — so a channel cannot be
called `1` under any spelling, and the real fix here is a name.

Grammar: `docs/grammar.md` §1.1, §2.1. Topic:
`agent-compose docs getting-started`.
