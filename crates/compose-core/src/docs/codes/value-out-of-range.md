# value-out-of-range

## What it protects

A numeric value outside its declared range. Every bound in this grammar exists
because something downstream depends on it being finite and sane:
`max_concurrency` is 1..256 because it is an admission bound on real work,
`retry.max` is 1..10 because retries consume a timeout budget,
`max_iterations` is 1..1000 because it is a termination proof, `top_k` is
1..100 and `limit` is 1..1000 because a store op's cardinality is meant to be
readable without running the graph.

`min_length` and `max_items` and their neighbours are the schema-level version
of the same idea.

## A spec that triggers it

```yaml triggers
version: "0.1"
tool.title:
  description: Takes a title.
  input:
    title: { type: string, min_length: -1 }
  output: {}
  exec:
    command: title
```

## The fix

Bring the value inside the range the message names. A bound you genuinely want
outside it is a sign the construct is being asked to do something it is not for
— a `max_concurrency` above 256 wants a queue, not a fan-out.

Grammar: `docs/grammar.md` §3.3, §3.5, §8.6, §9.1, §11.4. Topic:
`agent-compose docs schemas`.
