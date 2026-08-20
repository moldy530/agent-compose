# conflicting-keys

## What it protects

Two keys that may not appear together both appear. The pairs are not arbitrary
— in each one, honouring both is impossible or one of them is inert:

- `when:` and `else:` on one edge; `metadata_schema:` on a `blob` store, which
  no `blob` op and no synthesized `blob` tool reads;
- `node:` and `route_by:` on one `map`, which are the two dispatch forms;
- a map-level `input:`/`writes:`/`detach:` on a `route_by:` map, where each
  belongs per route;
- `callback:` with `respond: sync`, where the response already carries the
  outputs, and `timeout:` with `respond: async`, where there is no response for
  a budget to bound;
- an inline node's `input:` beside the in-block key that would carry it —
  `body:` on a body-bearing method, `query:` on `GET`/`HEAD`;
- two implementation bindings on one `tool.*`.

## A spec that triggers it

```yaml triggers
version: "0.1"
store.artifacts:
  kind: blob
  scope: global
  metadata_schema:
    source: { type: string }
```

## The fix

Drop one. The message names both keys and which construct they are on, and in
most of these one of the two is the one you meant — the other arrived by
copy-paste from a construct that does take it.

Grammar: `docs/grammar.md` §6, §7.2, §8.3, §8.6, §11.1, §13.3, Decision D113.
Topics: `agent-compose docs stores`, `agent-compose docs maps`.
