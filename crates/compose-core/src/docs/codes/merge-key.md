# merge-key

## What it protects

`<<:` is a YAML **1.1** extension and sits outside YAML 1.2 core. Supporting it
would mean the spec's meaning depended on which loader read it, which is exactly
the portability this grammar's YAML profile is written to remove.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.anthropic: &connection
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

provider.secondary:
  <<: *connection
  api_key: ${SECONDARY_API_KEY}
```

## The fix

Repeat the keys, or alias the **whole** value with `*anchor`. Anchors and
aliases are supported and are expanded by the parser before anything
spec-level happens:

```yaml
provider.anthropic: &connection
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

provider.mirror: *connection      # the whole mapping, not a merge
```

Where the two definitions genuinely differ, write both out. A provider
definition is four lines, and two of them being identical is not duplication
worth a language feature.

Grammar: `docs/grammar.md` §1.1. Topic:
`agent-compose docs getting-started`.
