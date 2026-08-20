# type-mismatch

## What it protects

Two declared types meet and the source cannot land in the target. This is the
most widely reused code in the compiler, because "these two declarations have to
fit" is the same question at a dozen sites: an edge payload, a node input
binding, a `writes:` destination, a flow `outputs:` field read from its channel,
a store-op `value:` against a `value_schema`, a trigger `input:` binding, a
model `settings:` value against the provider plugin's schema.

Every one of them is decided from **declared** types alone, never from runtime
values, which is what makes it a compile error rather than a class of run-time
surprise.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
  settings:
    max_tokens: "lots"
```

## The fix

The message names both sides — what the target takes and what the source
supplies. Usually one of the two declarations is simply wrong.

Two mismatches have a shape worth knowing. A write to an **`append`** channel
supplies **one element**, never the whole array: a channel of
`items: { type: string }` accepts a `string`. And a name-based read requires the
source to *satisfy* the field — every value the channel can hold must be legal
for the field — so widening a channel can break a reader that was fine before.

Grammar: `docs/grammar.md` §7.5, §8.0, §10.2, §11.4, §12.2, Decisions D58, D111.
Topics: `agent-compose docs state`, `agent-compose docs schemas`.
