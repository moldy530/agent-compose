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

Either **route the short branch through the same depth** — give it the step it
is missing, so both edges of the pair reach the convergence the same distance
from the fork — or **make the pair exclusive**. Two edges that cannot both fire
cannot both deliver, so an exclusive pair leaves the check entirely.

Exclusive means one of two things, and both cost a guard. Either the two edges
carry guards the exclusivity table can prove disjoint, or one carries `when:`
and the other `else: true` — the `else:` spelling is the one to read twice,
because an `else:` edge is itself required to have a `when:`-guarded sibling
(`invalid-value`, D107). Putting `else: true` on one edge of an *unguarded* pair
does not make it exclusive; it trades this diagnostic for that one.

A guard on an edge leaving `start` reads only `input`, `state` and `execution`,
and the flow above declares neither `inputs:` nor `state:`. That leaves
`execution`, which is the run's own identity rather than anything the
composition decided: `when: "execution.session_key == 'direct'"` against an
`else: true` sibling does make this pair exclusive and does validate clean, and
it decides which branch a graph takes by how the run was started. The route is
open and it is the wrong one, so this example takes the depth fix.

The diagnostic names the fork, the convergence, the two edges, and the two
distances, so the shorter one is the edge to change.

## The fix, applied

The spec above with the short branch given a step of its own. `merge` is now two
steps from `start` down both edges, so it is scheduled once.

```yaml spec
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
    second: { agent: agent.a, input: "'z'" }
    merge: { agent: agent.a, input: "'y'" }
  edges:
    - { from: start, to: first }
    - { from: start, to: second }
    - { from: first, to: merge }
    - { from: second, to: merge }
    - { from: merge, to: end }
```

Grammar: `docs/grammar.md` §7.3, §7.6, §7.6.1, §7.6.2, Decisions D69, D99, D107,
D112. Topic: `agent-compose docs routing`.
