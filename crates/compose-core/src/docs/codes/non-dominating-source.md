# non-dominating-source

## What it protects

In `over: <node>.output.…`, the producing node must **dominate** the map node:
every path from the flow's `start` to the map must pass through it.

Mere path-existence is not enough. A producer sitting on a guarded sibling
branch may not have run when the map dispatches, and then `over:` has no value
at all — a run-time failure on exactly the pass where the guard went the other
way.

Dominance is computed on the same graph the cycle analysis already builds, and
stays decidable with cycles present: a back-edge adds no new path from `start`.

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
agent.planner:
  model: model.m
  prompt: Plan.
  output:
    tasks:
      type: array
      max_items: 5
      items: { type: string }
agent.worker:
  model: model.m
  prompt: Work.
  output:
    result: { type: string }
flow.f:
  outputs: {}
  nodes:
    gate: { agent: agent.a, input: "'x'" }
    plan: { agent: agent.planner, input: "'y'" }
    work:
      map:
        over: "plan.output.tasks"
        node: agent.worker
        max_concurrency: 2
  edges:
    - { from: start, to: gate }
    - { from: gate, to: plan, when: "gate.output.verdict == 'approve'" }
    - { from: gate, to: work, else: true }
    - { from: plan, to: work }
    - { from: work, to: end }
```

`work` is reachable without passing through `plan` — down the `else:` edge.

## The fix

Route every path to the map through the producer. Above, deleting the
`gate → work` shortcut does it: the gate's other branch then has to go somewhere
else, or the map is simply not on it.

The alternative is to move the array into **state**: have the producer write a
channel, give the channel a `default:` so it is readable before the first write,
and point `over:` at `state.<channel>`. That trades the static guarantee for a
declared empty default — and a map over an empty array dispatches zero
instances and routes exactly as if every instance had finished.

Grammar: `docs/grammar.md` §8.6, Decision D76. Topic:
`agent-compose docs maps`.
