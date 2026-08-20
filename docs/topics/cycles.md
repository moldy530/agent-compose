# cycles

Back-edges are allowed, and every one of them has to come with a reason it
stops. The compiler computes strongly connected components and requires each to
be **bounded**; there is no such thing as a loop this compiler accepts on trust.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.writer:
  model: model.m
  prompt: Write or rewrite the draft, using the feedback if there is any.
  input:
    goal:     { type: string }
    feedback: { type: string }
  output:
    draft: { type: string }

agent.reviewer:
  model: model.m
  prompt: Approve only when the draft fully satisfies the goal.
  input:
    goal:  { type: string }
    draft: { type: string }
  output:
    verdict:  { enum: [approve, revise] }
    feedback: { type: string }

state:
  draft:    { type: string, default: "" }
  feedback: { type: string, default: "" }

flow.review_loop:
  inputs:
    goal: { type: string }
  outputs:
    draft: { type: string }
  nodes:
    write:
      agent: agent.writer
      input: { goal: "input.goal", feedback: "state.feedback" }
    review:
      agent: agent.reviewer
      input: { goal: "input.goal", draft: "state.draft" }
  edges:
    - { from: start, to: write }
    - { from: write, to: review }
    # the back-edge, with its budget
    - { from: review, to: write,
        when: "review.output.verdict == 'revise'",
        max_iterations: 3 }
    # the escape the budget requires: unguarded enough to always fire
    - { from: review, to: end, else: true }
```

## What "bounded" means

An SCC with at least one edge is bounded when **either** holds:

1. **Counting bound** — some edge whose `from` *and* `to` are both in the SCC
   carries `max_iterations` (and therefore a `when:` guard); or
2. **CEL exit condition** — some node `n` in the SCC satisfies both halves:
   - **there is a way out**: at least one outgoing edge of `n` leaves the SCC;
     and
   - **there is a pass that takes it**: no outgoing edge of `n` that stays
     inside the SCC is unconditional, and if one of them carries `else: true`,
     then `n` also has a `when:`-guarded outgoing edge that **leaves** the SCC.

Both halves of clause 2 are load-bearing. Without the first, the pass on which
the in-SCC guards all go false takes no edge at all — that is a dead end, not an
exit. Without the second the loop re-enters whatever the guards say: an
unguarded in-SCC edge fires every pass, and an in-SCC `else:` edge fires unless
a guarded sibling was *taken*, so only a taken guarded sibling that **leaves**
the SCC can suppress it without re-entering.

An SCC satisfying neither is `unbounded-cycle`, naming the SCC's nodes.

These two spellings are the same loop, and both are bounded:

```yaml
# guarded back-edge + guarded exit
- { from: review, to: write, when: "review.output.verdict == 'revise'" }
- { from: review, to: end,   when: "review.output.verdict != 'revise'" }

# guarded back-edge + else: escape — the usual spelling
- { from: review, to: write, when: "review.output.verdict == 'revise'" }
- { from: review, to: end,   else: true }
```

A counting bound is a **static** termination proof; a CEL exit condition is not
— its guard is a runtime value, so a model that never emits the exit value keeps
looping. Both are accepted; only the first makes the loop provably finite, which
is why the examples here use it.

## The escape rule

The **source node of each `max_iterations` edge** must have at least one
outgoing edge that

- **leaves the SCC**, and
- is **unconditional** (no `when:`, no `else:`) or carries **`else: true`**.

Both halves matter. An unconditional escape fires every pass. An `else:` escape
fires whenever no guarded sibling was *taken* — which includes the pass where
the budget runs out, because an exhausted edge is not taken and so cannot
suppress it. A **guarded** escape is not enough: with
`when: "…verdict == 'approve'"` as the only way out, the pass that exhausts the
budget while that guard is false takes no edge at all.

The escape itself carries no budget to exhaust, because `max_iterations`
requires a `when:` and an escape has none. Where the escape is spelled
`else: true`, its guarded-sibling requirement is discharged by the budgeted edge
itself.

A bounded edge whose source has no escape of this form is `dead-end`.

## What the budget counts

`max_iterations` counts **traversals of that edge within one flow instance**.
Instances of the same flow — including `map`-dispatched ones — count
independently. One counter per bounded **edge**, so an SCC carrying two budgeted
edges carries two budgets.

## Reachability, and the rest of the exits

Every node must be reachable from its flow's `start`, over edges plus the two
control-transfer positions — `on_error: { fallback: … }` and
`human.on_timeout:`. Guards are ignored: whether an edge fires is a runtime
question, and this check asks whether a node is *ever* addressable. A node
nothing addresses is `unreachable-node`; give it an inbound edge, name it as a
fallback, or delete it.

Reachability constrains *entry*; the exit rules constrain *exit*. Every node
also needs at least one outgoing edge, including a node reached only by a
fallback — `- { from: cleanup, to: end }` is the usual line, and retiring a
branch is exactly what `end` is for.

Normative source: `docs/grammar.md` §7.4, §7.8
