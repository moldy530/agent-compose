# invalid-duration

## What it protects

Durations are single-segment: a positive integer followed by `ms`, `s`, `m` or
`h`. `1m30s` is a parse error — write `90s`.

Single-segment is deliberate. One spelling per duration means a diff never shows
`60s` becoming `1m`, and there is no arithmetic between a written value and the
number a generated project uses.

Used by `timeout:`, `retry.backoff`, `retry.max_backoff`, `human.timeout`, a
sync trigger's `timeout:`, a `kv` store's `default_ttl:` and a `kv` write's
`ttl:` — which alone also takes `never`, a key that does not expire. A
`default_ttl: never` is refused here: a store with no `default_ttl:` already
keeps every key a write leaves without a `ttl:`.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.demo:
  outputs: {}
  nodes:
    write:
      exec: { command: "true" }
      timeout: 1m30s
  edges:
    - { from: start, to: write }
    - { from: write, to: end }
```

## The fix

Convert to one unit: `1m30s` is `90s`, `2h30m` is `150m`. Values must be greater
than zero — a `timeout: 0s` is not "no timeout", it is a budget that has already
expired; omit the key instead.

Grammar: `docs/grammar.md` §4.4, §11.1, §11.4, Decisions D22, D153. Topics:
`agent-compose docs cel`, `agent-compose docs stores`.
