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

**How this document is laid out**, because the check above reads it: each record
type is named once, as a bare backticked type name, in the section that
specifies it — and every field of that type has a **row in that section's
table**. A field explained only in the surrounding prose is an undocumented
field, and the fix is a row.

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
| `writes` | array of strings | `"completed"`, `"skipped"` | The state channels this node wrote, by name (grammar §10.1). Empty on a skipped node, which writes nothing. Absent on a failed entry: a node that failed produced no output to write from, and where the failure ended the run the superstep it died in lands nothing at all (§9). |
| `routing` | [routing decision](#4-routing-decisions) | `"completed"`, `"skipped"`; conditionally on `"failed"` | What this node's outgoing edges answered. See §4 and §9. |
| `dispatches` | array of [dispatch records](#5-dispatch-records) | `map` nodes that dispatched | What a fan-out dispatched, one record per source item in **index** order (grammar §8.6, PRD 5.6). Absent rather than empty on a map that resolved nothing to report: one whose input binding failed before the plan was built, and one whose own `timeout:` caught every joined instance mid-flight (§5.2). See §5. |
| `inner` | array of entries | `flow:` nodes | The trace of the subflow instance this node ran (grammar §8.5). See §8. |
| `stores` | array of [store records](#6-store-records) | when the node performed any | Every store op this node performed, in the order it performed them (PRD 5.8). See §6. |
| `models` | array of [model calls](#7-model-calls) | when the node made any | Every model call this node execution made (PRD 5.9). See §7. |
| `error` | string | see §3.2 | What went wrong, as `<error name>: <message>` — the failure's class and its text, in that one shape on **every** entry that carries the field, whether the node aborted the run, took a `fallback:`, or had its failure absorbed by `on_error: skip`. Written for a person: §10.1 makes the text something a reader must not parse, and it can quote what the other side of an activity answered — §11.1 is what it may and may not hold. |
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
  it (grammar §9.2). It carries an empty `writes`, and a `routing` decided under
  grammar §7.3 unchanged, with the one substitution grammar §9.2 and Decision D97
  make: a guard that reads the node's own output is `false` **without being
  evaluated**, while a guard over `input`, `state` or `execution` is evaluated
  normally. It carries `error` naming the failure too — always, and in §3's one
  shape: a failure a node absorbed is described exactly as one that ended the run
  is, because a reader handed the same field should not have to know which of the
  two it is reading to know what it is holding.
* **`"failed"`** — the node left no result for the run to carry on from, and its
  policy did not absorb it. Usually that is its *activity* failing. It also
  covers the node whose activity completed and whose **routing** then failed —
  no viable route (grammar §7.3 rule 7), or a guard that could not be evaluated
  at all — which is one entry a reader should not read as "the work did not
  happen": everything the node did is on it (§9). Two shapes, told apart by
  `fallback`:
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
| `targets` | array of strings | The nodes scheduled next, in the declaration order of the edges that reached them, deduplicated — the same order `edges` is in, filtered to the taken ones. `"__end__"` is the terminal pseudo-node. A multicast (grammar §7.3 rule 6) names more than one. |
| `counters` | object, string → integer | The `max_iterations` counters this step **spent**, by key, holding their new value (grammar §7.4). A key appears only when this step spent it; the value is the count after the spend. |

### 4.1 Edge decisions

`EdgeDecision`. One per outgoing edge, in declaration order — including the edges
that were **not** taken, which is the half a reader most often needs.

| field | type | presence | meaning |
|---|---|---|---|
| `to` | string | always | The target node id, or `"__end__"`. |
| `when` | string | guarded edges | The `when:` guard, as CEL source, verbatim from the composition (grammar §4.1). |
| `else` | `true` | `else:` edges | Marks the edge as the `else:` catch-all (grammar §7.3 rule 4). Never `false`: an edge that is not the catch-all omits the field. |
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
| `"a guarded sibling was taken"` | an `else:` edge suppressed because a guarded sibling of this node was taken (grammar §7.3 rule 4) |

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
| `route` | string | routed maps | The **route** the item was dispatched through: its variant tag, or `"$default"` for the `default:` catch-all (grammar §8.6 rule 4, Decision D30). Absent on the homogeneous form, which has one target and no tags. The catch-all's sigil is not a name an author could have written, because a union may declare a variant *called* `default` beside a `default:` catch-all. |
| `variant` | string | routed maps | The **discriminator value the item carried** — the value at the map's `route_by:` field. On a named route it repeats `route`; on the catch-all it is the only record of which variant fell through, since `route` names the catch-all rather than the variant. It is always one of the union's declared variant tags: the item was parsed against its producer's declared schema before any of this ran (PRD 5.2). |
| `target` | string | always | The component the item was dispatched to, as a typed address. |
| `outcome` | `"completed"` \| `"skipped"` \| `"failed"` \| `"detached"` | always | See §5.1. |
| `attempts` | integer | always | How many attempts the item's `on_item_error: { retry: … }` policy **made** (grammar §8.6 rule 10). `0` for a detached dispatch, which has no observed outcome for a policy to have acted on. |
| `idempotencyKey` | string | always | The key this dispatch's effect site derives (grammar §9.4). See §8. |
| `inner` | array of [entries](#3-entries) | **joined** `flow.*` targets | The dispatched instance's own trace, whether it completed or failed. Absent on a `"detached"` record whatever its target: see below. |
| `error` | string | `"skipped"`, `"failed"` | Why the item did not complete. |

A **detached** dispatch is the one target shape that carries no `inner` even
where it points at a `flow.*`. Grammar §8.6 rule 7 admits `detach: true` on a
route whose `node:` is a `flow.*` (under `--target local`, which is where the key
is legal at all), and this record is written when the dispatch is *issued*:
before the instance has run a node, and the join never comes back for it
(Decision D94). So a reader reconstructing subgraph traces reads `inner` on the
joined dispatches and gets nothing from the detached ones — which is the same
thing `outcome: "detached"` says, stated where the presence column is what §10.1
makes reliable.

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
  entry's `error` names the budget that ended it. A deadline that caught *every*
  instance leaves no records at all, and the entry then carries no `dispatches`
  key rather than an empty array — an absent key says "nothing resolved", where
  an empty array would say "nothing was dispatched".

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
| `idempotencyKey` | string | `"write"` **via `"node"`** | The key the write carried (grammar §9.4). See §8, and the paragraph below for the surface that carries none. |
| `deduped` | boolean | `"write"` **via `"node"`** | Whether the backend had already applied that key — the difference between "this run wrote it" and "an earlier attempt of this same effect did". |

**A write through a synthesized store tool carries neither.** The two fields go
together, and they are a property of the store-op **node** catalog rather than of
writing: a record with `via: "tool"` and `effect: "write"` — an agent that called
`notes_upsert` or `prefs_set` inside its loop (grammar §11.5) — has no
`idempotencyKey` and no `deduped`, whichever of §6's four write ops it ran. That
is grammar §9.4's own reading: it names exactly two carriers, "a detached `map`
dispatch (§8.6 rule 7) and a store write (§11.4)", and §11.4 is the node catalog.
It is also what the mechanism is for — a key stands in for an outcome nobody
observed, and a tool call's outcome goes straight back to the model that asked
for it. A reader indexing writes by key indexes the `via: "node"` ones and must
carry the rest some other way.

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
idempotency key: `DispatchRecord.idempotencyKey`, which every dispatch record
carries, and `StoreRecord.idempotencyKey`, which a store-op **node**'s write
carries and a write through a synthesized store tool does not (§6). Both are

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
* it records **`routing` in exactly one case: no viable route** (grammar §7.3
  rule 7). A node whose activity failed never got to evaluate its edges, so
  there is nothing to record. A node that completed and then matched no edge
  did evaluate them all: that entry carries a `routing` whose `edges` are the
  guard values that decided it, `targets: []` — which is what went wrong — and
  `counters: {}`. The near case is worth stating too, because "routing failed"
  would suggest otherwise: a guard that could not be **evaluated** — a CEL
  expression that threw — also fails the node at routing time and records
  **no** `routing`, since the decision was abandoned part way and there is no
  complete set of edge decisions to hand over. The entry's `error` names the
  guard.

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
  `TraceEntry.outcome`, `DispatchRecord.outcome`, `StoreRecord.op`,
  `StoreRecord.effect`, `StoreRecord.via`, `StoreRecord.scope`, and a refusal's
  `condition`. `op` is the one whose type in `src/runtime.ts` is `string` rather
  than the union — the union is the emitted `src/stores.ts`'s `StoreOp`, and a
  record type declared under the runtime cannot name it without inverting that
  dependency — so §6's seven are its vocabulary, and the inventory checks them
  against `StoreOp` itself;
* the orders §3.1, §4.1 and §5 fix — entries by `(step, node)`, edge decisions in
  declaration order, dispatch records in source-item index order;
* the nesting structure of §8, and the shape of the keys it describes.

A reader MUST NOT rely on:

* **the absence of a field.** A later version may add one, and a reader that
  rejects unknown keys will break on a compatible change. Ignore what you do not
  recognize.
* **the text of any message field** — `TraceEntry.error`, `DispatchRecord.error`,
  `Refusal.detail`, and the envelope's `error`. These are diagnostics written for
  a person (PRD G3) and are improved between releases. §3's `<error name>:
  <message>` is the shape they are written in, not a parse: the class in front of
  the colon is whatever the failing activity raised, and neither the set of
  classes nor the text after it is fixed by this format. The `reason` field of an
  edge decision is the exception, and only because §4.1 enumerates its values: it
  is a closed vocabulary that happens to be spelled as a sentence.
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
  of it. Use `--format json`, or the file it names. §11.1's promise is about the
  format, and this report is outside it in one way worth naming: under a failure
  it also prints the platform's own `cause` chain, which is where a resolved
  `${ENV}` value can still reach a terminal — a DNS failure naming the host an
  interpolated `url:` resolved to, say. No field of the format carries it.
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

**No resolved `${ENV}` value appears in this format.** Grammar §4.3 classifies
every string surface of a composition, and the two classes that can hold one are
kept out for different reasons.

**Class 1 — env-ref only.** A provider's `api_key:`, a backend's `url:`, `dsn:`
or `token:`: the whole value is one `${NAME}` reference, and nothing here is
derived from one. A trace names a `model.*` and a `store.*` by its typed address
(grammar §2.2), never by what the provider or backend behind it is configured
with. No field carries a request header, a connection string or a signed URL.
A provider whose resolved `base_url:` is not a URL at all is reported as
`` `provider.acme`'s resolved `base_url:` is not a URL ``, which is the whole of
what the runtime says about it.

**Class 2 — interpolable.** An `http:` binding's `url` and `headers` values, and
the whole `exec:` block — `command`, every `args` entry, `cwd`, and `env` values.
This class is wider than a reader might guess: `url: "${SIGNED_ENDPOINT}/reports"`
and `command: "${TOOLBIN}/rg"` are compositions the grammar admits, and a failing
activity quotes what it was pointed at. It quotes it **as the author wrote it** —
`` `${SIGNED_ENDPOINT}/reports` answered 404 … `` — rather than as it resolved.
The emitted runtime's `asWritten` is where that happens, and it buys a second
property besides: one composition produces one message whatever environment it
runs in, so an error is reproducible and a trace snapshot is comparable across
machines.

What a message *does* quote from outside this process is **what the other side
answered**, and a reader should treat that text as untrusted:

| field | what it can carry from outside |
|---|---|
| `TraceEntry.error` | the failure the node's own activity raised — for an `http:` binding, the rejected response body truncated to 200 characters; for an `exec:` binding, the child's stderr |
| `TraceDocument.error` | the same text, when that failure is what stopped the run |
| `Refusal.detail` | what a provider answered: a status and a response body, truncated, or the socket failure that came back instead. Never the request, so the key it was signed with is not in it |

A target that echoes back what it was sent puts that echo in the trace — a 404
body naming the path it did not route, a command that prints its own arguments
on stderr. This format records what it was answered; it does not audit it.

One field is the composition's own to fill: `StoreRecord.answer` is what a read
answered, verbatim (§6). A run that reads a secret out of a store has put it
there itself, and the trace records the read like any other.

### 11.2 Where a resolved value would otherwise have escaped

§11.1's promise is about **every** message this format carries, not only the ones
an author is likely to hit — and three of them are messages the *platform* would
have written if the runtime had let it. A failure the runtime never composed
itself is the shape a promise like this leaks through, so each of the three is
caught and restated:

| failure | what the platform says | what the runtime says instead |
|---|---|---|
| an `exec:` command that could not be started — a `command:` or `cwd:` that is missing or not executable | Node: `spawn /opt/tokens/rg ENOENT`; Bun: `ENOENT: no such file or directory, posix_spawn '/opt/tokens/rg'` | `` `${TOOLBIN}/rg` could not be run (ENOENT) `` — the reference as written, plus the platform's error **code**, which names what went wrong without naming what it went wrong on |
| an `http:` binding whose interpolated `url` is not a URL | Bun: `"secret/reports" cannot be parsed as a URL.` | `` `${SIGNED_ENDPOINT}/reports` is not a URL once its `${ENV}` references are resolved `` |
| a provider whose resolved `base_url:` is not a URL | Node: `Failed to parse URL from https://secret…` | `` `provider.acme`'s resolved `base_url:` is not a URL `` |

An **abort** is not in this table and is not restated: a node deadline (grammar
§9.2) and a cancelled run reach the same place, and such a failure is raised as
it came, because it is the run's own and carries nothing of the binding in it.

Restating costs the platform's own wording, and the exchange is deliberate: what
is lost is a path the reader can print for themselves from the reference the
message names, and what is bought is the property §11.1 opens with — plus the
one it buys alongside, that one composition produces one message whatever
environment it runs in, so an error is reproducible and a trace snapshot is
comparable across machines.

The whole set is held mechanically, in three places, because a promise about a
public surface is worth no more than what checks it:

* `crates/compose-core/tests/trace_format_inventory.rs` fails when any of the
  five message sites — the two that quote a *refused* activity (a non-2xx
  response, an unexpected exit code) and the three above — stops naming the
  reference or starts quoting the resolved string;
* `crates/compose-core/tests/generated_code_gates.rs` plants one value in all
  three surfaces and drives a generated project's own runtime into each failure,
  **under both** supported runtimes (PRD §9.18). That column is not optional
  here: which engine quotes a resolved string differs by failure, so a check that
  asked one of them would be evidence for whichever half it happened to run;
* `crates/agent-compose/tests/trace_format_stability.rs` reads the promise off a
  real run's trace.
