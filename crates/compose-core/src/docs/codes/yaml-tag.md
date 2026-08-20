# yaml-tag

## What it protects

YAML tags (`!!str`, `!custom`) carry language-level type directives. This spec
has none: a value's type comes from the schema it is declared against, and the
posture that the spec contains no executable code extends to it carrying no
type machinery either.

A tag is also usually a workaround for something with a proper spelling —
`!!str 0.1` for a version, most often, where quoting is the answer.

## A spec that triggers it

```yaml triggers
version: !!str "0.1"
```

## The fix

Drop the tag. To keep a scalar a string, **quote** it: `version: "0.1"`,
`id: "4"`. To change what a value means, change the schema it is declared
against.

Grammar: `docs/grammar.md` §1.1, §1.3, §3. Topics:
`agent-compose docs getting-started`, `agent-compose docs schemas`.
