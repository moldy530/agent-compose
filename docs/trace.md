# agent-compose — Trace Format

**Trace version:** `1`
**Status:** Normative for the trace a compiled project emits
**Companion artifacts:** [`docs/grammar.md`](grammar.md) (the DSL this describes runs of), [`prd.md`](../prd.md) §5.3, §5.6, §5.8, §5.9

A run of a compiled graph records what it did. This document defines that record:
where it is delivered, what every field means, and what a reader may rely on
across compiler releases.

The record exists because of a design decision rather than as a debugging
convenience. PRD 5.3 makes the runtime — not the model — decide every
transition, and says the consequence out loud: "routing decisions appear in
traces as data, not opaque model behavior". PRD §7 M2 turns that into a
deliverable — "a documented, stable trace format; routing decisions (which edge
fired and the guard values that decided it, which map variant a discriminator
chose, what terminated a cycle) as trace data" — and this document is that
format.

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense.

**Where the fields are implemented.** Every record type here is a TypeScript
interface in the emitted `src/runtime.ts`, which is byte-identical in every
project a given compiler release builds. This document and those declarations are
held together mechanically: `crates/compose-core/tests/trace_format_inventory.rs`
fails when a field exists in `src/runtime.ts` and is not named here.

---

## Table of contents

1. [Delivery surfaces](#1-delivery-surfaces)
2. [The envelope](#2-the-envelope)
3. [Entries](#3-entries)
4. [Routing decisions](#4-routing-decisions)
5. [Dispatch records](#5-dispatch-records)
6. [Store records](#6-store-records)
7. [Model calls](#7-model-calls)
8. [Nesting, instance paths, and idempotency keys](#8-nesting-instance-paths-and-idempotency-keys)
9. [Failed runs](#9-failed-runs)
10. [Stability](#10-stability)
11. [What is not part of this format](#11-what-is-not-part-of-this-format)

---

## 1. Delivery surfaces

A compiled project delivers a trace on three surfaces. All three carry the same
entries; they differ in what surrounds them.

| surface | what carries the trace | version key |
|---|---|---|
| `run --format json` | one JSON object on **stdout**, whose `trace` is the array of entries | `trace_version`, beside `trace` |
| the trace **file** | one JSON object — the whole [envelope](#2-the-envelope) — under the project's data directory | `trace_version`, at the head of the envelope |
| `serve` status | `GET /executions/:id` and the `callback:` webhook body, whose `trace` is the array of entries | `trace_version`, beside `trace` |

The rule that spans them: **wherever a `trace` appears, the `trace_version` that
describes it appears beside it.** A `serve` report for a run that is still going
carries neither.

### 1.1 `run --format json`

`agent-compose run <flow> --format json` prints one document (grammar §13.2):

```json
{
  "flow": "flow.review_loop",
  "execution_id": "exec_0f1e…",
  "status": "completed",
  "outputs": { "draft": "…" },
  "trace_version": 1,
  "trace": [ /* entries */ ],
  "trace_path": "/…/.agent-compose/traces/flow.review_loop-exec_0f1e….json"
}
```

`status` is `"completed"` or `"failed"`. A failed run answers with the same
document, `error` in place of `outputs`, and the trace it did make. `trace_path`
is absent when the file could not be written (an unwritable data directory is not
a reason to lose a run that otherwise succeeded) and when the run made no entries
at all.

### 1.2 The trace file

Every `run` also writes the whole trace to
`.agent-compose/traces/<flow>-<execution id>.json` under the project's data
directory, and names the path on stderr under `--format human`. The file is one
[envelope](#2-the-envelope) — **not** a bare array of entries — because it is the
one surface that arrives without a record around it: a reader who opens the file
later has nothing else to tell them which format it is in or which run it belongs
to.

A run that made **no** entries writes no file, and names none.

### 1.3 `serve` status

The generated app's `GET /executions/:id` route, and the `callback:` webhook it
POSTs on completion, report an execution as (grammar §13.3):

```json
{
  "execution_id": "exec_0f1e…",
  "flow": "flow.review_loop",
  "trigger": "on_request",
  "status": "completed",
  "outputs": { "draft": "…" },
  "trace_version": 1,
  "trace": [ /* entries */ ]
}
```

`status` here is the *execution's* — `running`, `completed`, `failed` or
`interrupted` — which is a larger vocabulary than the envelope's, because an
execution tracked by a live process can be in states a finished run cannot.
`trace` and `trace_version` appear together once the run has stopped; a `running`
execution reports neither.

---

## 2. The envelope

`TraceDocument`, in the emitted `src/runtime.ts`. It is what the trace file holds
in full, and what `run --format json` spreads into the record it prints.

| field | type | presence | meaning |
|---|---|---|---|
| `trace_version` | integer | always | The format the `entries` are written in. `1` is this document, and a compiled project spells it `TRACE_VERSION` (exported from its `src/runtime.ts`). See [Stability](#10-stability). |
| `flow` | string | always | The flow that was run, as its typed address (grammar §2.2). |
| `execution_id` | string | always | The execution the entries belong to — grammar §4.1's `execution.id`, and the prefix of every idempotency key in the document (grammar §9.4). |
| `status` | `"completed"` \| `"failed"` | always | Whether the run produced an answer. |
| `error` | string | on `"failed"` | What stopped the run. Present because a failed run's last entry does not always say: a run stopped by the superstep ceiling has no aborting node to carry one. |
| `entries` | array of [entries](#3-entries) | always | Every entry the run recorded, in the order §3.1 fixes. |

**Key spelling.** The envelope's keys are `snake_case`; an entry's are
`camelCase`. The seam is deliberate rather than an oversight: the envelope's keys
are *document* keys, alongside `execution_id`, `trace_path` and `status_url` on
the same surfaces, while an entry is a runtime record — the very object the
emitted `runFlow` answers with in `FlowRun.trace`, read by in-process callers as
JavaScript rather than as a document.

---

## 3. Entries

`TraceEntry`. **One entry per node execution.** A node that runs twice — a second
traversal of a bounded cycle (grammar §7.4), a node reached again through a
`fallback:` — produces two entries. A node's own `retry:` attempts do **not**:
retries are attempts at one execution, and `attempts` is where they are recorded.

| field | type | presence | meaning |
|---|---|---|---|
| `step` | integer | always | The superstep this node ran in, counting from `1`. Every node of one superstep shares it. |
| `flow` | string | always | The typed address of the flow this node belongs to (grammar §2.2). It is the *instance's* flow, so entries nested under [`inner`](#8-nesting-instance-paths-and-idempotency-keys) name the subflow rather than the caller. |
| `node` | string | always | The flow-local node id (grammar §2.4). |
| `traversal` | integer | always | How many times this node had **already** begun executing in this flow instance — `0` on the first, `1` on the second traversal of a bounded cycle. It is grammar §9.4's traversal ordinal, the same number the instance path is built from. |
| `outcome` | `"completed"` \| `"skipped"` \| `"failed"` | always | See §3.2. |
| `attempts` | integer | always | How many attempts the node's `retry:` policy **made**, not how many it allowed (grammar §9.1) — a budget that ran out during the second of three made two. `0` when the node never ran: an input binding that could not be evaluated fails the execution before any attempt (grammar §4.1, §10.1). |
| `writes` | array of strings | `"completed"`, `"skipped"` | The state channels this node wrote, by name (grammar §10.1). Empty on a skipped node, which writes nothing. Absent on a failed entry, because the superstep a run dies in lands no writes at all. |
| `routing` | [routing decision](#4-routing-decisions) | `"completed"`, `"skipped"`; conditionally on `"failed"` | What this node's outgoing edges answered. See §4 and §9. |
| `dispatches` | array of [dispatch records](#5-dispatch-records) | `map` nodes | What a fan-out dispatched, one record per source item in **index** order (grammar §8.6, PRD 5.6). See §5. |
| `inner` | array of entries | `flow:` nodes | The trace of the subflow instance this node ran (grammar §8.5). See §8. |
| `stores` | array of [store records](#6-store-records) | when the node performed any | Every store op this node performed, in the order it performed them (PRD 5.8). See §6. |
| `models` | array of [model calls](#7-model-calls) | when the node made any | Every model call this node execution made (PRD 5.9). See §7. |
| `error` | string | see §3.2 | What went wrong, as `<error name>: <message>`. |
| `fallback` | string | when `on_error: { fallback: … }` fired | The node id the failure routed to instead of this node's own edges (grammar §9.2). `"__end__"` for the terminal pseudo-node. |

### 3.1 Order

Entries are ordered by `(step, node)` — ascending superstep, then ascending node
id within a superstep. Node id rather than completion order is grammar §7.6.4's
canonical write order read for the trace: two runs of one composition produce the
entries in the same order whatever order the scheduler and the providers finished
them in.

One entry is outside that ordering, and it is always last: the entry of the node
a **failed** run aborted at (§9).

Nested entries (`inner`, and a dispatch record's `inner`) have step numbers of
their **own instance**, starting again at `1`. They are not interleaved with the
caller's.

### 3.2 Outcomes

* **`"completed"`** — the node ran and produced a result. It carries `writes` and
  `routing`; it carries no `error`.
* **`"skipped"`** — the node's activity failed and its `on_error: skip` absorbed
  it (grammar §9.2). It carries `error` naming the failure, an empty `writes`,
  and a `routing` decided under grammar §7.3 rule 6: a guard that reads the
  node's own output is `false` **without being evaluated**, while a guard over
  `input`, `state` or `execution` is evaluated normally.
* **`"failed"`** — the node's activity failed and its policy did not absorb it.
  Two shapes, told apart by `fallback`:
  * with `fallback`, the failure routed to the named node (grammar §9.2,
    Decision D21). The node's own edges were not evaluated, so there is no
    `routing`. The run continues, and the entry lands in the trace like any
    other.
  * without `fallback`, the failure ended the run. That entry is the last one in
    the document — see §9.

A node that never ran at all — one whose *input* could not be built — is
`"failed"` with `attempts: 0`.

### 3.3 Synthetic nodes

A flow whose `start` edges carry guards gets a synthetic entry node, because
`start` is not a node and has no output for a guard to read (grammar §7.2,
§7.6.3). Its entries are ordinary entries with `node: "$start"`. The `$` sigil is
outside grammar §2.1's identifier, so the id collides with nothing an author can
declare.

---

## 4. Routing decisions

`RoutingDecision`, on `TraceEntry.routing`. This is the record PRD 5.3 asks for:
which edge fired, and the guard values that decided it.

| field | type | meaning |
|---|---|---|
| `edges` | array of [edge decisions](#41-edge-decisions) | What every outgoing edge answered, in **declaration order** (grammar §7.3). |
| `targets` | array of strings | The nodes scheduled next, in the declaration order of the edges that reached them, deduplicated. `"__end__"` is the terminal pseudo-node. A multicast (grammar §7.3 rule 4) names more than one. |
| `counters` | object, string → integer | The `max_iterations` counters this step **spent**, by key, holding their new value (grammar §7.4). A key appears only when this step spent it; the value is the count after the spend. |

### 4.1 Edge decisions

`EdgeDecision`. One per outgoing edge, in declaration order — including the edges
that were **not** taken, which is the half a reader most often needs.

| field | type | presence | meaning |
|---|---|---|---|
| `to` | string | always | The target node id, or `"__end__"`. |
| `when` | string | guarded edges | The `when:` guard, as CEL source, verbatim from the composition (grammar §4.1). |
| `else` | `true` | `else:` edges | Marks the edge as the `else:` catch-all (grammar §7.3 rule 5). Never `false`: an edge that is not the catch-all omits the field. |
| `value` | boolean | guarded edges | What the guard answered. |
| `budget` | object | see below | The `max_iterations` budget on this edge, and its state at this decision: `key` (the counter this edge spends), `used` (the count **after** this decision), `max` (the declared budget). Present on a budgeted edge whose guard answered `true` — which is when a budget is either spent or found spent. |
| `taken` | boolean | always | Whether this edge scheduled its target. |
| `reason` | string | untaken edges, and unconditional ones | Why, when the guard value alone does not say. See below. |

`reason` takes exactly three values in this version, and each names a rule rather
than describing one:

| `reason` | when |
|---|---|
| `"unconditional"` | an edge with no `when:` and no `else:`, which is always taken (grammar §7.3 rule 2) |
| `"the \`max_iterations\` budget is spent"` | a guarded edge whose guard answered `true` and whose budget was already at `max`, so it was not taken (grammar §7.4) |
| `"a guarded sibling was taken"` | an `else:` edge suppressed because a guarded sibling of this node was taken (grammar §7.3 rule 5) |

A guarded edge that was simply not taken carries `value: false` and **no**
`reason`: the guard value is the whole explanation.

### 4.2 What terminated a cycle

A cycle (grammar §7.4) leaves through one of two decisions, and both are in the
`edges` array of the entry where it happened:

* **a counting bound ran out** — the back-edge's decision has `value: true`,
  `taken: false`, the `"the \`max_iterations\` budget is spent"` reason, and a
  `budget` naming the counter, the count and the declared `max`. The `else:` edge
  beside it is `taken: true`.
* **a CEL exit condition went false** — the back-edge's decision has
  `value: false`, `taken: false`, and no reason. The `else:` edge beside it is
  `taken: true`.

The passes before the last are the same edges seen `taken: true`, one entry per
traversal, with `budget.used` climbing on a counted loop. A cycle that never left
at all — grammar §7.4 clause 2 admits one whose guard never goes false — is a run
the superstep ceiling stopped: `status: "failed"` with an envelope `error` naming
the ceiling, and every traversal it did make in `entries`.

---

## 5. Dispatch records

`DispatchRecord`, on `TraceEntry.dispatches`. PRD 5.6 makes a fan-out's
cardinality and destination *data*; this is that data. One record per source item,
in ascending `index` — never completion order.

| field | type | presence | meaning |
|---|---|---|---|
| `index` | integer | always | The source-item index, which is what identifies the item and orders every write it made (PRD 5.6, grammar §7.6.4 clause 2). |
| `route` | string | routed maps | The **route** the item was dispatched through: its variant tag, or `"$default"` for the `default:` catch-all (grammar §8.6 rule 8, Decision D30). Absent on the homogeneous form, which has one target and no tags. The catch-all's sigil is not a name an author could have written, because a union may declare a variant *called* `default` beside a `default:` catch-all. |
| `variant` | string | routed maps | The **discriminator value the item carried** — the value at the map's `route_by:` field. On a named route it repeats `route`; on the catch-all it is the only record of which variant fell through, since `route` names the catch-all rather than the variant. It is always one of the union's declared variant tags: the item was parsed against its producer's declared schema before any of this ran (PRD 5.2). |
| `target` | string | always | The component the item was dispatched to, as a typed address. |
| `outcome` | `"completed"` \| `"skipped"` \| `"failed"` \| `"detached"` | always | See §5.1. |
| `attempts` | integer | always | How many attempts the item's `on_item_error: { retry: … }` policy **made** (grammar §8.6 rule 10). `0` for a detached dispatch, which has no observed outcome for a policy to have acted on. |
| `idempotencyKey` | string | always | The key this dispatch's effect site derives (grammar §9.4). See §8. |
| `inner` | array of [entries](#3-entries) | `flow.*` targets | The dispatched instance's own trace, whether it completed or failed. |
| `error` | string | `"skipped"`, `"failed"` | Why the item did not complete. |

### 5.1 Dispatch outcomes

* **`"completed"`** — the instance ran and its writes were joined.
* **`"skipped"`** — the item failed and `on_item_error: skip` dropped it; the
  fan-out carried on (grammar §8.6 rule 10).
* **`"failed"`** — the item failed under `on_item_error: fail`, or a
  parameterized `retry:` ran out of attempts, which "resolves as `fail` does".
  The map node fails with it, and its own `on_error:` is what decides the run.
* **`"detached"`** — the dispatch was `detach: true` and was **resolved at
  dispatch** (grammar §8.6 rule 7, Decision D94). The join counted it the moment
  the delivery was issued and never learned its outcome, so no strategy ever
  applied to it and `attempts` is `0`. A `"detached"` record says a delivery was
  issued; it says nothing about whether the sink accepted it.

### 5.2 A map node that failed

A map node that failed still dispatched, and its entry still carries what it
dispatched — items that ran already had their effects. Two failure paths differ
in what survives:

* the map node **raised** (an item failed, or its own `on_error:` absorbed the
  fan-out): the records are complete, one per source item.
* the map node's own **`timeout:`** fired (grammar §9.2, §8.6 rule 9): the
  deadline is raced, so the records are those that had **resolved** — every
  detached delivery, and every joined instance that had settled. An instance
  still in flight when the budget ran out has no outcome and so no record; the
  entry's `error` names the budget that ended it.

---

## 6. Store records

`StoreRecord`, on `TraceEntry.stores`. PRD 5.8's replay discipline is what makes
these records part of the format rather than logging: "store ops are effects
(activities). Reads are recorded — replay consumes history, not the live store.
Writes are at-least-once with idempotency keys."

| field | type | presence | meaning |
|---|---|---|---|
| `store` | string | always | The store's typed address (grammar §2.2). |
| `op` | string | always | The op, spelled as grammar §11.4 spells it: `get`, `set`, `delete`, `list`, `search`, `upsert`, `put`. |
| `effect` | `"read"` \| `"write"` | always | Which half of the replay discipline this record belongs to. `set`, `delete`, `upsert` and `put` are writes; the rest are reads. |
| `via` | `"node"` \| `"tool"` | always | Which of PRD 5.8's two consumption surfaces ran it: a store-op node, or a synthesized store tool an agent called inside its loop. |
| `scope` | `"execution"` \| `"session"` \| `"global"` | always | The store's declared lifetime, and with it which partition was addressed (grammar §11.3). |
| `key` | string | ops that address one | The key the op addressed. |
| `answer` | any | `"read"` | What the read answered — the history a replay consumes. Its shape is the store's own (grammar §11.4), not this format's. |
| `idempotencyKey` | string | `"write"` | The key the write carried (grammar §9.4). See §8. |
| `deduped` | boolean | `"write"` | Whether the backend had already applied that key — the difference between "this run wrote it" and "an earlier attempt of this same effect did". |

Every op a node performs lands on that node's entry, across **every attempt** its
`retry:` policy made: an effect that happened is an effect that happened, and a
record that kept only the last attempt's would describe a run the store did not
see.

---

## 7. Model calls

`ModelCall`, on `TraceEntry.models`. PRD 5.9 asks for failover to be
"deterministic runtime behavior recorded in the trace (`served by model.fast,
fallback #1`)" — this is that record, so failover is data rather than something a
reader infers from a provider's own logs.

| field | type | presence | meaning |
|---|---|---|---|
| `model` | string | always | The `model.*` the composition named — a direct binding or a route (grammar §12.2). |
| `servedBy` | string | when a member answered | The route member that answered. |
| `fallback` | integer | with `servedBy` | Its ordinal in the route, `0` for the first — so `1` reads as "fallback #1". |
| `failovers` | array of [refusals](#71-refusals) | always | Every member that refused **and moved the ladder on**, in the order they were tried. Empty when none did. |
| `refused` | [refusal](#71-refusals) | when no member answered | What ended the call. Such a record carries no `servedBy` and no `fallback`. |

A **direct** binding produces a record too, with `servedBy === model`,
`fallback: 0` and no failovers: a trace that recorded only the interesting calls
would leave a reader unable to tell a call that did not fail over from a call
nothing recorded.

### 7.1 Refusals

`Refusal`, and `Failover` — the same shape, narrowed: a failover's `condition` is
never absent, because a failover is by definition a refusal the route declared.

| field | type | presence | meaning |
|---|---|---|---|
| `model` | string | always | The member that refused. |
| `condition` | `RouteCondition` | when the refusal classifies as one | Which of grammar §12.2's `route_on:` conditions it refused with: `"rate_limit"`, `"overloaded"`, `"timeout"` or `"server_error"`. Absent when the refusal is none of them — a 400, a body that is not JSON — which is a refusal that ends a call rather than one that moves the ladder. |
| `detail` | string | always | What the member said, for a reader who has to fix it. It is a message rather than a field a reader should parse. |

### 7.2 Which calls land on which entry

The entry's `models` is **every model call that node execution made** — across
every attempt its `retry:` policy made and every instance a `map` dispatched —
not only the calls of the attempt that answered. A node that succeeded on its
second attempt reports the first attempt's spent ladder too.

Where the two sets differ, calls the node could **order** come in that order (a
`map`'s, which is source-item order) and the rest come ahead of them in the order
they were made. This format does **not** attribute a call to a source item: a map
node's `models` is the concatenation, and the item that made a given call is not
recorded.

---

## 8. Nesting, instance paths, and idempotency keys

A composition is a module system, and a flat trace would either lose a
subgraph's routing decisions or pretend they were the caller's. So they nest:

* `TraceEntry.inner` — the instance a `flow:` node ran (grammar §8.5). A node
  under `retry:` runs an instance per attempt, and every attempt's entries are in
  this one array, in the order they ran, the successful one last. A reader tells
  the boundary the way the node's own `attempts` says to: an instance starts at
  `step: 1`, so a second `step: 1` entry for the same node is the next attempt
  beginning.
* `DispatchRecord.inner` — the instance a `map` dispatched to a `flow.*`
  (grammar §8.6), whether it completed or failed.

Nesting moves nobody's step numbers: an inner instance numbers its own supersteps
from `1`.

**Instance paths surface in exactly two fields**, and only as the prefix of an
idempotency key: `DispatchRecord.idempotencyKey` and `StoreRecord.idempotencyKey`
(grammar §9.4). Both are

```
<execution.id> "/" <frame> { "/" <frame> }
```

where each frame is `<node id> "/" <traversal ordinal>` — plus `"/" <item index>`
for a `map` node — for every node crossed from the **root** flow instance down to
the effect site, outermost first. The envelope's `execution_id` is that first
component, so a reader can strip it and read the remainder as the path.

```
exec_01/dispatch/0/7                 the detached dispatch of item 7 by map node `dispatch`
exec_01/outer/0/3/inner/0/0/save/0   store node `save`, under item 0 of `inner`, itself item 3 of `outer`
```

Grammar §9.4 is normative for the derivation; this document only fixes where it
appears. Two properties are what make it worth reading out of a trace: distinct
effects get distinct keys, and a repeated attempt at one effect reuses its key —
so `deduped: true` on a store write means an earlier attempt of *this* effect had
already been applied, rather than some other write colliding.

---

## 9. Failed runs

A failed run has a trace, and it is the trace a reader most often wants. What is
in it:

* every entry that **landed** — the supersteps that completed before the failure;
* then, last, the entry of the node the run **aborted at**.

The last entry is a `"failed"` entry with no `fallback`, and it is special in two
ways, both consequences of how a superstep dies rather than choices:

* it records **no `writes`**. The superstep a run dies in lands none of them —
  every task's update in that step is discarded — so a `writes` array would name
  channels that were never written.
* it records **`routing` only when the routing decision is what failed.** A node
  whose activity failed never got to evaluate its edges. A node that completed
  and then found **no viable route** (grammar §7.3 rule 7) did: that entry
  carries a `routing` whose `edges` are the guard values that decided it,
  `targets: []` — which is what went wrong — and `counters: {}`.

Everything else the node did is still there: `stores`, `models`, `dispatches` and
`inner` are all recorded on the aborting entry, because those effects really
happened and the failure alone says nothing about them.

A run stopped by the **superstep ceiling** has no aborting node at all: the
ceiling is the compiler's safety net rather than one of the composition's own
bounds, and it stops the run between supersteps. Such a document has every entry
that landed and no final failed entry; the envelope's `error` is what names the
ceiling.

---

## 10. Stability

`trace_version` is what a reader pins. This section is what pinning it buys.

### 10.1 What a reader may rely on

At a given `trace_version`, a reader MAY rely on:

* every field this document names, under the name and with the meaning given
  here;
* the presence rules stated in each table's *presence* column;
* the vocabularies of the closed enumerations: `TraceDocument.status`,
  `TraceEntry.outcome`, `DispatchRecord.outcome`, `StoreRecord.effect`,
  `StoreRecord.via`, `StoreRecord.scope`, and a refusal's `condition`;
* the orders §3.1, §4.1 and §5 fix — entries by `(step, node)`, edge decisions in
  declaration order, dispatch records in source-item index order;
* the nesting structure of §8, and the shape of the keys it describes.

A reader MUST NOT rely on:

* **the absence of a field.** A later version may add one, and a reader that
  rejects unknown keys will break on a compatible change. Ignore what you do not
  recognize.
* **the text of any message field** — `TraceEntry.error`, `Refusal.detail`, and
  the envelope's `error`. These are diagnostics written for a person (PRD G3) and
  are improved between releases. The `reason` field of an edge decision is the
  exception, and only because §4.1 enumerates its values: it is a closed
  vocabulary that happens to be spelled as a sentence.
* **anything printed by the human report.** See §11.
* **the shape of `StoreRecord.answer`**, which is the store's, not this format's.

### 10.2 What is a compatible change

Compatible, and made **without** a version bump:

* adding a field to an existing record type;
* adding a new record type reachable from an existing one;
* recording a field in cases where it was previously absent, provided the
  presence rule stated here is widened rather than contradicted;
* improving the text of a message field.

### 10.3 What requires a version bump

`trace_version` MUST be incremented for any change that would make a reader
written against the previous version wrong:

* removing a field, or renaming one;
* changing a field's type, or the meaning of its value;
* narrowing a presence rule — recording a field in fewer cases than this document
  promises;
* adding a member to one of §10.1's closed enumerations, or removing one;
* changing one of the fixed orders;
* changing the derivation of an idempotency key, or where instance paths appear.

A bump changes the number on every surface at once — the file, `run --format
json`, and `serve` — because they carry one format. A release that bumps it says
so in its notes, and this document's *Trace version* header moves with it.

### 10.4 How the two are held together

The version number alone is a promise; two tests make it a checkable one:

* `crates/compose-core/tests/trace_format_inventory.rs` reads the emitted
  `src/runtime.ts` and fails when a record type reachable from the envelope, or
  any field of one, is not named in this document. Documentation cannot rot
  behind the code.
* `crates/agent-compose/tests/trace_format_stability.rs` runs real compiled
  graphs against the mock provider and snapshots the **shape** of the traces they
  produce. A renamed or removed field is a snapshot diff in a pull request rather
  than a discovery made by a downstream reader.

---

## 11. What is not part of this format

* **The human report.** `run --format human` writes a summary to stderr — one
  line per node, plus what its models and stores did and which edges were taken.
  It is written for a terminal and is not versioned: nothing should be parsed out
  of it. Use `--format json`, or the file it names.
* **`FlowRun.trace` as a TypeScript type.** An in-process caller of the emitted
  `runFlow` receives the same entries as JavaScript objects. The *data* is this
  format; the declarations in `src/runtime.ts` are generated code, and an ejected
  project owns them.
* **The trace file's path and name.** `<flow>-<execution id>.json` under the
  project's data directory is where a run puts it, and `trace_path` is how a
  caller learns where; the layout under `.agent-compose/` is the emitted
  project's, described in its own `README.md`.
* **Anything a store answered.** `StoreRecord.answer` is carried verbatim; its
  shape is the store's declared schema (grammar §11.4).
* **Provider transcripts.** A trace records that a call was made and which member
  served it, never the prompt or the completion.

### 11.1 Secrets

**No record type in this document holds a credential.** Grammar §4.3 admits
`${ENV}` references in exactly the places a secret belongs — a provider's
`api_key:`, a backend's URL — and nothing in this format is derived from one: no
field carries a resolved environment value, a request header, or a signed URL.

`Refusal.detail` is the field to be clear about, because it is the one that
quotes something from outside the process. What it quotes is what the
**provider answered** — a status and a response body, truncated — or the socket
failure that came back instead. It is never the request, so the key the request
was signed with is not in it.
