# policies

The compiled graph is the *workflow* — deterministic and replayable — and nodes
are *activities*: effectful and retryable. `retry:`, `timeout:` and `on_error:`
are how an activity's failure is handled, and each resolves independently
through four levels.

```yaml spec
version: "0.1"
provider.p:
  kind: anthropic
  api_key: ${ANTHROPIC_API_KEY}
model.m:
  provider: provider.p
  id: claude-sonnet-4-5

defaults:                       # level 3, composition-wide
  timeout: 60s
  retry: { max: 1, backoff: 1s }
  on_error: fail

agent.fetcher:
  model: model.m
  prompt: Fetch and summarize.
  output:
    summary: { type: string }

agent.cleanup:
  model: model.m
  prompt: Record that the step did not complete.
  output:
    note: { type: string }

flow.pipeline:
  outputs: {}
  nodes:
    fetch:
      agent: agent.fetcher
      input: "'the source'"
      retry: { max: 3, backoff: 2s, multiplier: 2.0, max_backoff: 30s, jitter: true }
      timeout: 90s
      on_error: { fallback: recover }       # level 2, this node
    recover:
      agent: agent.cleanup
      input: "'the failure'"
      on_error: skip
  edges:
    - { from: start, to: fetch }
    - { from: fetch, to: end }
    - { from: recover, to: end }
```

## `retry`

```yaml
retry: { max: 3, backoff: 2s, multiplier: 2.0, max_backoff: 30s, jitter: true }
```

| Key | Required | Default | Notes |
|---|---|---|---|
| `max` | yes | — | integer 1..10; **additional** attempts after the first |
| `backoff` | yes | — | a duration; the initial delay |
| `multiplier` | no | `2.0` | number ≥ 1; exponential factor |
| `max_backoff` | no | — | cap on a single delay |
| `jitter` | no | `true` | full jitter |

Retries are exhausted **before** `on_error` applies. Retry attempts consume the
node's `timeout` budget, and a sync trigger's timeout budget, with no special
casing.

## `timeout`

A duration bounding **one node execution**, all retry attempts included.

## `on_error`

The strategy applied once retries are exhausted:

| Value | Semantics |
|---|---|
| `fail` | abort the execution with the node's error; sibling branches stop with it. The built-in default |
| `skip` | the node produces no output and writes nothing; routing proceeds normally, with one substitution |
| `{ fallback: <node id or end> }` | the node's own outgoing edges are **not** evaluated; the fallback target is scheduled instead, or the branch retires if the target is `end` |

**After a `skip` there is no second routing algorithm.** Declaration order,
multicast, `else:` last, an exhausted budget untakeable — all unchanged. The
*only* thing the skip changes is one class of guard value: a `when:` referencing
the skipped node's output evaluates **`false`**, while a guard referencing only
`input`, `state`, or `execution` evaluates normally. That is why a node
declaring `skip` must have an unconditional or `else: true` outgoing edge —
otherwise it could take no edge at all.

A node that *ran* and whose result legally omits a field resolves the other way:
there **is** an output object, so a guard reading a field it does not carry
fails the execution. The substitution belongs to `skip` alone.

`fallback` targets a **flow-local node id or `end`** — never `start`, never a
node of another flow. A fallback target is reachable *because* it is one, so a
dedicated `cleanup` node reached only that way is live code and needs its own
outgoing edge.

## The four levels

For each policy field independently, highest precedence first:

1. **Flow override** — `policy:` on the `flow:` node that instantiated the
   enclosing flow, propagated into nested instantiations. It applies to the
   nodes *inside* that instance, never to the instantiating node itself. Where
   several instantiation sites in a nesting chain set the same field, the
   **outermost** wins: a caller's hardening of a module it does not own cannot
   be undone by that module's own instantiation of a deeper one. A `map`
   contributes no level 1 — it has no `policy:` key.
2. **Node** — the node's own `retry`/`timeout`/`on_error`.
3. **Defaults** — the composition's `defaults:` section, which accepts exactly
   those three keys and appears at most once per composition.
4. **Built-in** — no retry, no timeout, `on_error: fail`.

## Two exemptions worth memorizing

**A `human` node resolves no `timeout` and no `retry`, at any level.** Levels 1
and 3 are skipped for those two fields on a `human` node, because a wait is not
an activity timeout and re-prompting a person is not a retry. Its wait semantics
live exclusively in the `human:` block. `on_error` is not exempt and resolves
through all four levels — it covers delivery failures.

Nor does the budget of a node *above* a wait cut it short: an enclosing node's
deadline is **held still** for as long as a wait inside it is open, and resumes
with the time it had left.

**Levels 1 and 3 take no `fallback`.** `on_error:` in `defaults:` and in a
`flow:` node's `policy:` is `fail` or `skip` only. A fallback target is a node
id of the flow the declaring node sits in, and neither of those levels names a
flow — `defaults:` reaches every node of every flow, and a `policy:` reaches
every node of the instantiated subflow and everything it instantiates in turn.
"Retry, then give up" stays expressible at both levels, because `retry` is a
field of its own.

## Where policy does not live

Tool definitions carry **no** policy keys. Policy is a property of a use site,
so it goes on the node.

A `map`'s per-item strategy is `on_item_error:` inside the `map:` block, and it
is deliberately not part of this chain: `defaults:` applies to nodes, and a
dispatched instance is not a node of this flow. See `agent-compose docs maps`.

Normative source: `docs/grammar.md` §9, §9.1, §9.2, §9.3
