# missing-key

## What it protects

A required key is absent. Which keys are required is usually decided by a
sibling: a `kv` store must declare `value_schema:`, a `vector` store must
declare `embed:`, a `human:` block with a `timeout:` must declare `on_timeout:`,
a route form must declare `route:`.

Sometimes what is required is a **choice**: a `tool.*` declares exactly one of
`exec:`/`http:`/`function:`, an `http` trigger's inbound `auth:` exactly one of
`bearer:`/`hmac:`, and its `callback_auth:` at least one of the same two. An
empty block is the same absence as a missing key, and the message names every
spelling that would fill it.

Requiredness is where this grammar refuses to guess. There is no defaulted
`value_schema` that admits anything, and no `on_timeout:` that means "carry on"
— either would turn a declaration the author forgot into a behaviour they never
chose.

## A spec that triggers it

```yaml triggers
version: "0.1"
store.memory:
  kind: kv
  scope: session
```

## The fix

Add the key. The message names it, the construct it belongs to, and — where a
sibling decided it — which sibling:

```yaml
store.memory:
  kind: kv
  scope: session
  value_schema:
    last: { type: string }
```

Grammar: `docs/grammar.md` §6, §8.7, §11.1, §12.2, §13.3. Topics:
`agent-compose docs stores`, `agent-compose docs human`,
`agent-compose docs triggers`.
