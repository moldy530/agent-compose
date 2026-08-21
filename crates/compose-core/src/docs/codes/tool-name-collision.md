# tool-name-collision

## What it protects

Attaching a store to an agent synthesizes LLM-facing tools from the store's kind
and `agent_access:` — `<name>_get`, `<name>_set`, `<name>_search`,
`<name>_upsert`, `<name>_list`, `<name>_put` — where `<name>` is the store's
local name. A synthesized name colliding with an attached `tool.*` or `flow.*`
means the model is offered two different things under one name, and the wire has
no way to say which one a call meant.

Neither half of the rule is decidable in one place: the synthesized names come
from the store's definition, and the name they collide with is in the agent's
own `tools:` list.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
store.prefs:
  kind: kv
  scope: global
  agent_access: read
  value_schema:
    theme: { type: string }
tool.prefs_get:
  description: Read a preference the long way round.
  input:
    key: { type: string }
  output:
    value: { type: string }
  exec:
    command: prefs-get
agent.a:
  model: model.m
  prompt: Decide.
  tools: [tool.prefs_get]
  stores: [store.prefs]
  output:
    answer: { type: string }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: end }
```

## The fix

Rename the store, rename the tool, or drop one of the two attachments. The third
is worth considering first: a hand-written tool that duplicates a synthesized
one is usually the older of the two, and `agent_access: read` already narrows
the surface without a wrapper.

Grammar: `docs/grammar.md` §11.5. Topics:
`agent-compose docs stores`, `agent-compose docs agents`.
