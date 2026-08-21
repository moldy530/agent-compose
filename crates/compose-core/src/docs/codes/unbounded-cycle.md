# unbounded-cycle

## What it protects

A strongly connected component of the flow graph carries no bounded edge. Every
cycle must have a reason it stops, and there is no such thing as a loop this
compiler accepts on trust — an unbounded one is a graph that can run for ever,
burning model calls with nothing to stop it.

A self-edge is a one-node cycle and is bounded like any other.

An SCC is bounded when either a **counting bound** exists — an edge whose `from`
and `to` are both inside it carries `max_iterations` — or a **CEL exit
condition** does: some node in the SCC has an outgoing edge that leaves it, and
its in-SCC edges admit a pass on which none is taken.

## A spec that triggers it

```yaml triggers
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${K}
model.m:
  provider: provider.p
  id: some-model
agent.a:
  model: model.m
  prompt: Do it.
  output:
    verdict: { enum: [approve, revise, escalate] }
flow.f:
  outputs: {}
  nodes:
    n: { agent: agent.a, input: "'x'" }
  edges:
    - { from: start, to: n }
    - { from: n, to: n }
    - { from: n, to: end, when: "n.output.verdict == 'approve'" }
```

The self-edge above is unconditional, so it fires on every pass whatever the
guarded exit says. That is the second half of the CEL exit condition failing.

## The fix

Guard the in-cycle edge and give the node a way out:

```yaml
- { from: n, to: n,   when: "n.output.verdict == 'revise'", max_iterations: 3 }
- { from: n, to: end, else: true }
```

The `max_iterations` is what makes the loop **provably** finite — a CEL exit
condition alone is a runtime value, so a model that never emits the exit value
keeps looping. Adding `max_iterations` also brings its own rule with it: the
source node then needs an escape that is unconditional or `else: true`, which
the second line above is.

Grammar: `docs/grammar.md` §7.2, §7.4, Decisions D57, D98. Topic:
`agent-compose docs cycles`.
