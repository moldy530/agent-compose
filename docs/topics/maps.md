# maps

Agent-controlled cardinality with deterministic dispatch. An agent never spawns
work: it emits an array, and a `map` node fans out over it. Both axes are
bounded — the array declares `max_items`, the node declares `max_concurrency` —
and neither is optional.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

agent.planner:
  model: model.m
  prompt: Break the goal into tasks.
  output:
    tasks:
      type: array
      max_items: 20
      items: { type: string }

agent.worker:
  model: model.m
  prompt: Do one task and report what you did.
  input:
    task: { type: string }
  output:
    result: { type: string }

state:
  results:
    type: array
    max_items: 20
    items: { type: string }
    reduce: append

flow.fanout:
  inputs:
    goal: { type: string }
  outputs:
    results:
      type: array
      max_items: 20
      items: { type: string }
  nodes:
    plan: { agent: agent.planner, input: "input.goal" }
    work:
      map:
        over: "plan.output.tasks"      # a path expression
        as: task                       # names the item; `item` by default
        node: agent.worker
        input: { task: "task" }
        max_concurrency: 5
        on_item_error: skip
        writes: { result: results }    # `results` must be a reduced channel
  edges:
    - { from: start, to: plan }
    - { from: plan, to: work }
    - { from: work, to: end }
```

## Keys

| Key | Required | Notes |
|---|---|---|
| `over` | yes | a path expression resolving to an array with `max_items` |
| `as` | no | identifier naming the item; default `item` |
| `node` | homogeneous only | `agent.*` / `tool.*` / `flow.*`; mutually exclusive with `route_by` |
| `route_by` | heterogeneous only | a literal field name equal to the item union's `discriminator` |
| `routes` | with `route_by` | tag → route, **≥ 1 entry** |
| `default` | no | catch-all route; legal only with `route_by` |
| `max_concurrency` | **yes** | integer 1..256, node-wide admission bound |
| `on_item_error` | no | `fail` (default) \| `skip` \| `{ retry: … }` |
| `input` | homogeneous only | per-item binding; default: the whole item |
| `writes` | homogeneous only | output field → channel; targets must be reduced |
| `detach` | homogeneous only | boolean, default `false` |

A **route** takes `node` (required) plus optional `max_concurrency`, `input`,
`writes`, `detach`. A route's `max_concurrency` must be ≤ the map's.

Node-level `input:` and node-level `writes:` are **illegal** on a map node — a
map has no input or output of its own, only dispatched instances. Node-level
`retry`/`timeout`/`on_error` are legal and apply to the map node as a whole,
while `on_item_error` governs individual items.

## Heterogeneous dispatch

```yaml
dispatch:
  map:
    over: "triage.output.findings"
    as: finding
    route_by: kind
    max_concurrency: 8
    routes:
      auto_fixable: { node: agent.fixer, max_concurrency: 5, writes: { patch: patches } }
      needs_human:  { node: tool.review_queue }
    default: { node: tool.dead_letter }
```

If the item schema is a discriminated union, `route_by` is **required**; if it
is not a union, `route_by` is **illegal**. Every variant needs a `routes:` entry
or a `default:`, and a route tag that is not a variant tag is an error.

A named route's target is type-checked against **its variant's payload only** —
that is the narrowing. The `default:` route is type-checked against the
**unrouted** variants: its per-item CEL may select the discriminator and any
field declared by *every* unrouted variant. A `default:` with no unrouted
variant is unreachable and is an error.

`input:`, `writes:` and `detach:` describe a dispatch *target*, so a map
declaring `route_by:` must declare them **per route**, never at map level: the
routes are independently typed, a map-level `input:` would have to type-check
against every variant at once, and a blanket `detach:` would silently detach
sinks written to be joined. `max_concurrency:` and `on_item_error:` stay
map-wide, because neither is typed against a target.

## Per-item bindings

Both `input:` forms are legal here. A **field map** binds the target's declared
input fields from the item; a **bare scalar CEL string** binds a string-in
agent's single unnamed input, so `input: "item.summary"` is how an object item
feeds a string-in target. The two are not interchangeable, and only an agent
target can be string-in — `tool.*` and `flow.*` always declare an input object.

Omitting `input:` passes the whole item, which requires the item's type to be
compatible with the target's input.

## The join

The map node completes when every instance has completed, been resolved by
`on_item_error`, or been detached. Its outgoing edges fire in the next step. A
dispatch of **zero** instances — an empty source array, or a producer that was
skipped — completes immediately, writes nothing, and routes exactly as if every
instance had finished.

Anything a dispatched instance writes to shared state must target a channel with
a declared `reduce:` policy, because instances are concurrent writers. Their
writes land **by source-item index**, never by completion order: appended
results are index-tagged and reordered before the join, and a `merge` or
`last_wins` channel resolves to the highest-indexed item's write. Nothing here
depends on which instance finished first.

## `on_item_error`

`fail` (the default), `skip`, or `{ retry: <retry block> }` — the retry block
being the ordinary one, with `max` and `backoff` required. A bare
`on_item_error: retry` is an error: a retry with no bound is exactly the
unbounded loop this construct exists to prevent, and there is nowhere to inherit
one from.

It is read from the `map:` block alone. `defaults:` applies to *nodes*, and a
dispatched instance is not a node of this flow, so nothing supplies an item
policy behind your back. A retry re-executes the whole instance from its entry;
the item keeps its index, so every attempt derives the same idempotency key.
When retries are exhausted the item fails, and `on_error:` **on the map node**
is what absorbs that — `on_item_error: { retry: … }` with `on_error: skip` reads
"retry each item, and if one still fails, skip the fan-out".

## `over` reads a dominating node

In `over: <node>.output.…`, `<node>` must **dominate** the map node: every path
from `start` to the map passes through it. Path-existence is not enough — a
producer on a guarded sibling branch may not have run when the map dispatches,
leaving `over` with no value at all. Violations are `non-dominating-source`.

## `detach:`

`detach: true` is fire-and-forget, and it is a statement about the **join**:

- the join counts the dispatch resolved the moment it is issued, so the map node
  can complete — and its edges fire — while delivery is still in flight;
- `on_item_error` never applies: there is no observed outcome to apply a
  strategy to;
- nothing it does can fail or delay the enclosing instance.

So a detached dispatch **must not** declare `writes:` and must not write reduced
state (`detached-write`), and its target **must not reach a `human` node**
(`detached-interrupt`) — a pause inside one is a question nothing is left to
answer. `max_concurrency` still admits it: a detached dispatch waits for a
permit to *start*, the join just never waits on its outcome.

In the trace, the map node's entry records a detached dispatch as a stub —
`outcome: "detached"`, `attempts: 0`, no `inner` — and nothing the delivery goes
on to do. A detached **`flow.*`** dispatch starts a **child execution**: an
execution of its own, with an id derived from the parent's and the dispatch's
idempotency key, its own journal and recovery — a child still running when its
parent settles is resumed by the next `serve` — and, under a target that
declares a `trace_sink:`, its own export, headed `detached: true` and an
`idempotency_key` equal to the stub's (`agent-compose docs trace`). The parent
never waits for it; a `run` command does, before it exits.

In v0, `detach: true` is a validation error under any target whose execution
state is durably checkpointed — every target except `local`. The same
composition is legal under `--target local` and rejected under
`--target staging` (`unsupported-detach`).

## History

A dispatched instance runs on a **fresh** conversation history that is discarded
when it completes, for every dispatch form and every target. A `map` block
accepts no `context:` key, so the isolation is not waivable — which is what
keeps `max_concurrency: 5` from interleaving five conversations into one
channel.

Normative source: `docs/grammar.md` §8, §8.6
