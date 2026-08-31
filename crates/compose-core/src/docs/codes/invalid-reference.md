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

agent.reviewer:
  model: tool.web_search
  prompt: Review the draft and say whether it ships.
  output:
    verdict: { type: string }
```

## The fix

Name something the position accepts. Above, an agent's `model:` takes a
`model.*`, and `tool.web_search` is a tool: the agent needs the model binding it
calls, and the tool belongs in its `tools:` list if it wants it at all.

A deploy file's `placements:` reads the same way one level along: a placement's
`members:` are `agent.*` and `tool.*` addresses, so `store.docs` there is this
code — a store is not a placeable component (§14.1). A `flow.*` member is
refused too, but with a message naming the deferral rather than this one, since
placing a flow is a feature this release does not have rather than a spelling
mistake (`unsupported-placement`).

Two positions take a **flow-local node id** rather than an address, and a typed
address there is this error too: `on_error: { fallback: … }` and a `human`
node's `on_timeout:`. Control transfer stays inside one flow.

One position takes a reference **or** something that is not one: an agent's
`tools:` list, whose other entry shape is a runtime built-in
(`agent-compose docs tools`). `builtin.bash` written there as a bare address is
this error, and the message says why — a built-in carries the bounds it runs
under, so the entry is a mapping rather than a name:

```yaml
tools:
  - tool.repo_grep
  - builtin.read_file: { root: "${WORKSPACE}" }
  - builtin.bash:      { root: "${WORKSPACE}", timeout: 30s }
```

Grammar: `docs/grammar.md` §2.2, §2.3, §5.5, §14.1, Decision D123. Topics:
`agent-compose docs getting-started`, `agent-compose docs tools`.
