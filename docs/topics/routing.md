# routing

Every transition is the runtime's, decided from declared schemas and recorded
outputs. A model emits data; edges decide where the data goes. That is what
makes a run replayable and a change reviewable.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.reviewer:
  model: model.m
  prompt: Review the draft.
  output:
    verdict: { enum: [approve, revise, escalate] }

agent.publisher:
  model: model.m
  prompt: Publish the draft.
  output:
    url: { type: string }

flow.review:
  outputs: {}
  nodes:
    review:  { agent: agent.reviewer,  input: "'a draft'" }
    publish: { agent: agent.publisher, input: "'a draft'" }
    rework:  { agent: agent.publisher, input: "'rework'" }
  edges:
    - { from: start, to: review }
    - { from: review, to: publish, when: "review.output.verdict == 'approve'" }
    - { from: review, to: rework,  when: "review.output.verdict == 'revise'" }
    - { from: review, to: rework,  else: true }     # covers `escalate`
    - { from: publish, to: end }
    - { from: rework,  to: end }
```

## Edges

| Key | Required | Notes |
|---|---|---|
| `from` | yes | a node id, or `start` |
| `to` | yes | a node id, or `end` |
| `when` | no | a CEL guard returning bool, over the **source node's** output |
| `else` | no | the literal `true`; mutually exclusive with `when` |
| `max_iterations` | no | integer 1..1000; a cycle bound, and **requires `when:`** |

`start` is the flow's entry: nothing may target it, and edges leaving it may
carry guards — but at least one must be unconditional or `else: true`, so an
execution always has a first step. `end` **retires the branch that reaches it**;
it does not stop the run. Self-edges are legal and form a one-node cycle.
Duplicate edges (same `from`, `to`, `when`) are an error.

## How selection works

Evaluated after the source node's output has been validated:

1. Every outgoing edge of the completed node is evaluated **in declaration
   order**.
2. An edge with no `when:` and no `else:` is **unconditional** — always taken.
3. An edge with `when:` is taken iff its guard is true.
4. An edge with `else: true` is taken iff **no guarded sibling** from the same
   source was taken. At most one `else:` per node.
5. An edge whose `max_iterations` budget is exhausted is not taken, whatever its
   guard says.
6. **All taken edges fire.** Two taken edges are two concurrent branches.
7. If no edge is taken, the execution fails with "no viable route".

Selection is **multicast, not first-match-wins**. That is the single most
important thing to know about this section.

**An `else:` edge needs a guarded sibling.** With none, rule 4's suppression
clause can never fire, so the edge is taken on every pass — which is what an
edge carrying neither keyword already is. It is a compile error rather than a
keyword that states nothing.

## Exhaustiveness

If a node routes on an enum field of its output, every variant must be covered.
"Routes on `f`" means at least one outgoing `when:` mentions `<node>.output.<f>`
syntactically.

Coverage is decided over a **closed set of guard shapes**, so two conforming
validators accept exactly the same compositions. For an enum field `f` with
variant set `V`:

| Guard shape | `guaranteed` | `possible` |
|---|---|---|
| `n.output.f == L` | `{L}` | `{L}` |
| `n.output.f != L` | `V \ {L}` | `V \ {L}` |
| `n.output.f in [L₁ … Lₖ]` | `{L₁ … Lₖ}` | `{L₁ … Lₖ}` |
| `!g` | `V \ possible(g)` | `V \ guaranteed(g)` |
| `g₁ && g₂` | intersection of `guaranteed` | intersection of `possible` |
| `g₁ \|\| g₂` | union of `guaranteed` | union of `possible` |
| **anything else** | `∅` | `V` |

The last row is load-bearing: a term the table does not recognize —
`size(state.feedback) > 0`, any call, a comparison against a different field —
contributes **no** guarantee and excludes **no** variant.

A node routing on one or more enum fields is exhaustive when either:

1. it has an outgoing edge that is unconditional or carries `else: true`; or
2. there is **one** enum field it routes on whose variants are fully covered by
   the union of `guaranteed` over its guarded edges.

Otherwise it is `non-exhaustive`, naming the field with the largest covered set
and that field's uncovered variants. One field suffices, because edges are
multicast: full coverage of one already proves the node always has an edge.

**A guarantee cannot expire**, which is why `max_iterations` is legal only on a
`when:`-guarded edge: an edge whose budget ran out is not taken, so it could
never have been the guaranteed one.

## Steps, forks, and convergence

A flow executes in **steps**. Step 0 runs the targets of the taken `start`
edges. A node's outgoing edges are evaluated when **that node** completes, over
the state it started its step on plus its own writes — a concurrent sibling's
same-step write is not visible to a guard. When every node of step *k* has
completed, step *k*'s writes are applied in canonical order, and the union of
the targets of edges taken in step *k* is step *k+1*.

Two properties hold: a node's edges are evaluated only after it completes
(**barrier**), and a node targeted by two edges taken in the *same* step runs
**once** (**one run per step**).

Two sibling out-edges are **exclusive** when the validator can prove they are
never taken together: one carries `else:` and the other `when:`, or both carry
guards whose `possible` sets for some enum field are disjoint. Any other pair is
**co-takeable**, and a node with a co-takeable pair is a **fork**. Two nodes are
**concurrent** when both are reachable from one co-takeable pair of a fork, one
through each edge, and neither is reachable from the other.

A node with two or more incoming edges is a **convergence**. Within one step it
gets AND-join behaviour for free. A branch whose guard was false delivers
nothing and is not waited for — **a false guard can never deadlock a
convergence**, because nothing ever waits. A convergence reads *state*, never
its predecessors, so it needs no data-arrival protocol.

Arrivals in **different** steps schedule the node again. The statically visible
case is refused: for a fork, one of its co-takeable pairs, and a node reached at
different step distances by the two edges of that pair, the convergence is
`unbalanced-convergence`. Fix it by routing the short branch through the same
depth, or by making the pair exclusive.

`end` is exempt — branches legitimately reach it at different depths.

## Canonical write order

Several writers can write one channel in one step. Completion order is never
what decides the result. Writers are ordered by **node id**; a `map` node's
writers are its instances ordered by **source-item index**, at the map's own
place in that order; a `flow:` node is a single writer at its own id. The reduce
policy is then applied in that order.

## Skips

A node declaring `on_error: skip` produces no output. Routing then proceeds
exactly as above with **one substitution**: a guard referencing the missing
output evaluates `false`. Guards over `input`/`state`/`execution` evaluate
normally. That is why such a node must have an unconditional or `else:` edge —
otherwise it could dead-end on rule 7.

Normative source: `docs/grammar.md` §2.4, §7.2, §7.3, §7.3.1, §7.6, §7.6.1–7.6.4
