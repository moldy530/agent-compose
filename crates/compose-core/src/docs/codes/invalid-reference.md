# invalid-reference

## What it protects

A reference is a `namespace.name` string used as a **value**, and every
reference position accepts a fixed set of namespaces. An agent's `model:` takes
`model.*`; a node's `agent:` takes `agent.*`; a `function:` node takes `tool.*`;
a trigger's `flow:` takes `flow.*`; a `map`'s dispatch target takes `agent.*`,
`tool.*` or `flow.*`.

This code is raised when the string is not a legal address at all, or names the
wrong namespace. Where the address is well formed and nothing defines it, the
code is `undefined-reference` instead — different question, different message.

## A spec that triggers it

```yaml triggers
version: "0.1"

placements:
  store.docs:
    runtime: isolated
```

## The fix

Name something the position accepts. Above, `placements:` keys are `agent.*`,
`tool.*` or `flow.*` — a store is not a placeable component — and the section
belongs in a deploy file besides.

Two positions take a **flow-local node id** rather than an address, and a typed
address there is this error too: `on_error: { fallback: … }` and a `human`
node's `on_timeout:`. Control transfer stays inside one flow.

Grammar: `docs/grammar.md` §2.2, §2.3, §14.1. Topic:
`agent-compose docs getting-started`.
