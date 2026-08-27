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

The same rule reaches two more pairs, because the request's `tools` array is one
namespace whoever fills it. A provider's `server_tools:` suite is appended to
that array on every request the connection serves (grammar 12.1), so:

* two entries of one suite that reach the wire as one tool collide — the Messages
  wire pairs each dated `type:` with one fixed `name:`, which is what makes
  `code_execution_20250522` beside `code_execution_20250825` two types with one
  name;
* a server tool collides with an attached `tool.*`/`flow.*` or a synthesized
  store tool of any agent whose model reaches that provider — including through
  a failover route, whose members each declare their own suite.

Both are reported against the entry written second, with the first labelled.

A **runtime built-in** fills the same array under the same key (grammar 5.5), so
`builtin.bash` on an agent that also attaches a `tool.bash` is the same event
once more, and so is a built-in whose name a provider's suite takes. The repair
there is one-sided: a built-in's name — `bash`, `read_file`, `write_file`,
`list` — is the compiler's rather than an author's, so what moves is the other
entry, or the built-in attachment goes.

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

Where the other name is a **server tool**, the same three repairs read
differently: keep one of two dated revisions, rename the attachment the suite
collides with, or — since a suite belongs to the connection rather than to one
agent — declare it on a second provider that this agent's model does not reach.
Providers are cheap.

Grammar: `docs/grammar.md` §5.5, §11.5, §12.1, Decision D123. Topics:
`agent-compose docs stores`, `agent-compose docs agents`,
`agent-compose docs models`, `agent-compose docs tools`.
