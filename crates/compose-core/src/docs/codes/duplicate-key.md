# duplicate-key

## What it protects

A key declared twice in one mapping is an error, **never last-wins**. YAML
loaders usually take the last one silently, which turns a copy-paste mistake
into a composition that runs with configuration nobody wrote on purpose — the
second `api_key:`, the second `prompt:`, the second definition of one address.

Refusing it makes the mistake visible at the place it was made, with both
declarations named.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.anthropic:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}

provider.anthropic:
  kind: anthropic
  api_key: ${SECOND_API_KEY}
```

## The fix

Delete one, or rename one. Two providers that differ only in their key are two
providers and want two names.

Note the neighbouring rule: the same address declared in **two files** is
`duplicate-definition`, not this. This code is for two keys in one mapping.

Grammar: `docs/grammar.md` §1.1, §2.2. Topic:
`agent-compose docs getting-started`.
