# unbalanced-convergence

## What it protects

A node reached from one fork at two different step distances runs **twice** —
once per arrival. That is well defined and almost never intended: a merge node
that runs a second time re-reads state, re-writes channels, and makes a second
model call.

The check is per fork and per **co-takeable pair** of its outgoing edges,
because only two edges that can both be taken can deliver twice. Two edges the
validator can prove exclusive — one `else:` against one `when:`, or guards whose
possible variant sets are disjoint — are not compared at all.

`end` is exempt: branches legitimately retire at different depths.

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
    first: { agent: agent.a, input: "'x'" }
    merge: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: merge }
    - { from: start, to: first }
    - { from: first, to: merge }
    - { from: merge, to: end }
```

`start` is a fork like any other vertex: `merge` is one step away down one edge
and two steps away down the other.

## The fix

Either **route the short branch through the same depth** — send it through the
same intermediate node the long branch uses — or **make the pair exclusive**, by
putting `else: true` on one edge or writing guards the exclusivity table can
prove disjoint. Two edges that cannot both fire cannot both deliver, so the pair
leaves the check entirely.

The diagnostic names the fork, the convergence, the two edges, and the two
distances, so the shorter one is the edge to change.

Grammar: `docs/grammar.md` §7.6, §7.6.1, §7.6.2, Decisions D69, D99, D112.
Topic: `agent-compose docs routing`.
