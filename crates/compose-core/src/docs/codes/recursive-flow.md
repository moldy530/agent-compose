# recursive-flow

## What it protects

A flow that reaches itself is recursion, and recursion has no termination proof
here. The cycle rule bounds loops *inside* one flow by counting traversals of an
edge; there is no analogous bound on instantiation depth, and an unbounded one
would break the fan-out bounding guarantee too — each level multiplies the
instances the level above can create.

"Reaches" is the composition-wide relation, so all three routes count: a `flow:`
node, a `map` dispatch target, and a `flow.*` in the `tools:` list of an agent
the flow reaches. Flow-as-tool attachment is a call, and the two surfaces are
interchangeable by design, so it carries exactly the reachability a `flow:` node
does.

## A spec that triggers it

```yaml triggers
version: "0.1"
flow.f:
  outputs: {}
  nodes:
    inner: { flow: flow.f }
  edges:
    - { from: start, to: inner }
    - { from: inner, to: end }
```

## The fix

Break the cycle. Extract the shared part into a third flow that neither of the
two instantiates, and have both call that.

Where the recursion was the point — walking a tree, refining until a condition
holds — express it as a **bounded cycle** inside one flow instead: a back-edge
carrying `max_iterations` with an `else:` escape gives you the same repetition
with a proof that it stops.

Grammar: `docs/grammar.md` §7.5, §7.7, Decisions D26, D86. Topics:
`agent-compose docs flows`, `agent-compose docs cycles`.
