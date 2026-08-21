# missing-session-key

## What it protects

A `session`-scoped store partitions its data by the execution's **session
identity**, and that identity comes from the trigger. A declared `http`,
`schedule` or `event` trigger whose flow reaches such a store and declares no
`session_key:` would run with no partition to address.

"Reaches" is the composition-wide relation: the flow's own nodes, its maps'
dispatch targets, the flows it instantiates, the stores an agent it reaches
attaches, and the tools of any agent it reaches. So a session-scoped store three
instantiations deep still needs a session identity at the entry point.

`manual` triggers carry `session_key: "payload.session"` by default and satisfy
the check statically; supplying `--session` is then a run-time requirement.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
store.memory:
  kind: kv
  scope: session
  value_schema:
    last: { type: string }
triggers:
  on_request:
    type: http
    flow: flow.f
flow.f:
  outputs: {}
  nodes:
    forget:
      store: store.memory
      op: delete
      key: "'k'"
  edges:
    - { from: start, to: forget }
    - { from: forget, to: end }
```

## The fix

Declare where the session identity comes from — it is CEL over the trigger's
`payload`:

```yaml
triggers:
  on_request:
    type: http
    flow: flow.f
    session_key: "payload.headers['x-session-id']"
```

Or give the store another `scope:`. `global` is one partition for everything;
`execution` is one per run, which is the right answer for scratch data that
should not outlive the run.

Grammar: `docs/grammar.md` §7.7, §11.3, §13.1. Topics:
`agent-compose docs stores`, `agent-compose docs triggers`.
