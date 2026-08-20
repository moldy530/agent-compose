# unkeyed-map-write

## What it protects

A `vector` or `blob` write performed inside a `map`-dispatched instance must
have an **item-derived key**, so concurrent instances address disjoint keys. N
documents landing on one vector key is N documents' worth of data in one slot,
and no error at run time to say so.

An expression is item-derived when it references `execution.item_index`, or an
`input.<field>` whose binding **at that dispatch site** references the item or
the index. A field bound from `state.*`, from the enclosing flow's `input.*`, or
from a literal is **not** item-derived, however item-derived its name looks.

`kv` writes are exempt: `set`/`delete` replace the whole value at a slot the
author named, so concurrent instances writing one key are a declared overwrite —
the store-side counterpart of `reduce: last_wins`.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.local:
  kind: openai_compatible
  base_url: ${U}
store.docs:
  kind: vector
  scope: global
  embed:
    model: text-embedding-3-small
    provider: provider.local
state:
  tasks:
    type: array
    max_items: 5
    items: { type: string }
  topic: { type: string, default: "" }
flow.ingest:
  inputs:
    doc_id: { type: string }
    text:   { type: string }
  outputs: {}
  nodes:
    save:
      store: store.docs
      op: upsert
      key: "input.doc_id"
      value: "input.text"
  edges:
    - { from: start, to: save }
    - { from: save, to: end }
flow.f:
  outputs: {}
  nodes:
    fan:
      map:
        over: "state.tasks"
        as: task
        node: flow.ingest
        max_concurrency: 4
        input:
          doc_id: "state.topic"
          text:   "task"
  edges:
    - { from: start, to: fan }
    - { from: fan, to: end }
```

## The fix

The diagnostic's second label points at the **binding** that flipped the field
to not-item-derived, which may be an instantiation away from the store node.
Above, `doc_id: "state.topic"` is the site to edit, and two things fix it:

- bind from the item: `doc_id: "task"`;
- key off the index: `key: "execution.item_index"`.

A third fix exists where the item is an **object** schema-compatible with the
target's inputs: drop the map's `input:` entirely, so the whole item is the
instance's input and every `input.<field>` is item-derived. It does not apply
above — `state.tasks` holds strings and `flow.ingest` takes `{doc_id, text}`, so
dropping `input:` there trades this diagnostic for a `type-mismatch`.

Grammar: `docs/grammar.md` §11.4, Decisions D67, D83. Topics:
`agent-compose docs stores`, `agent-compose docs maps`.
