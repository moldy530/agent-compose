# agent-compose — Trace Format

**Trace version:** `4`
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

**Presence vocabulary.** Every field table below has a *presence* column, and
every cell in one is written in the same small vocabulary — because §10.1 makes a
reader entitled to both halves of what such a cell says: the cases a field
appears in, and the cases it therefore does not.

| in a presence column | what it says |
|---|---|
| **always** | the key is on every record of this type, without exception |
| a **condition** | the key is on exactly the records the condition describes, and on no others |
| **, possibly empty** | a qualifier on either of the above: where the key is present, the array or object it holds may have no elements — and where this document gives an empty value a meaning, an empty value is a statement rather than a way of being absent |

The qualifier is written **wherever an empty value is reachable**, so a presence
cell without it promises a value with something in it. That is the distinction
this format leans on hardest: `dispatches: []` and no `dispatches` key are two
different facts about a fan-out (§5.2), as are an empty `failovers` and a call
that named what refused it (§7). Absence is never spelled as an empty value, and
an empty value is never spelled as absence.

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

The rule that spans them: **on all three surfaces above, wherever a `trace`
appears, the `trace_version` that describes it appears beside it — and wherever
one is absent, so is the other.** Both halves are load-bearing, because one of
the three carries reports with no trace on them: a `serve` report for a run that
is still going carries neither, and so does one for a run whose failure carried
no trace at all — a request the graph refused before it ran (§1.3). `run
--format json` always carries both, `trace` empty where the run recorded
nothing. In process there is no document to put a version in, so the constant
`TRACE_VERSION` is where an in-process caller of `runFlow` reads the same number
(§11).

### 1.1 `run --format json`

`agent-compose run <flow> --format json` — grammar §13.2's invocation, plus the
emitted CLI's own `--format`, which the generated project's `README.md`
documents and no grammar section defines — prints one document:

```json
{
  "flow": "flow.review_loop",
  "execution_id": "exec_0f1e…",
  "status": "completed",
  "outputs": { "draft": "…" },
  "trace_version": 4,
  "trace": [ /* entries */ ],
  "trace_path": "/…/.agent-compose/traces/flow.review_loop-exec_0f1e….json"
}
```

`status` is the envelope's three (§2). A run that produced no answer — `"failed"`
or `"interrupted"` — answers with the same document, `error` in place of
`outputs`, and the trace it did make. `trace_path` is absent when the file could
not be written (an unwritable data directory is not a reason to lose a run that
otherwise succeeded) and when the run made no entries at all.

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
  "trace_version": 4,
  "trace": [ /* entries */ ]
}
```

`status` here is the *execution's* — `running`, `completed`, `failed` or
`interrupted` — which is a larger vocabulary than the envelope's, because an
execution tracked by a live process can be in states a finished run cannot. It
is `serve`'s own vocabulary rather than one of §10.1's closed enumerations: this
format's `status` is the envelope's three, and the word `interrupted` names a
different thing in each. Here it is a run that is **still going** and is holding
a `human` pause somebody can still answer (grammar §8.7); in the envelope it is a
run that **ended** holding one, which is what an `agent-compose run` with nobody
to ask does with every pause it reaches — a run *at a terminal* answers its
pauses and ends `completed` like any other (PRD §9.21).

`trace` and `trace_version` appear together, on a run that has **stopped**
carrying a trace — an empty one included. The gate is whether the run has stopped
and there is a trace at all, not whether it has entries in it: a run that failed
inside the graph having recorded nothing reports `trace: []`, which §2 makes a
statement about the run rather than a way of being absent.

Two kinds of report carry neither key. The first is a run that has **not
stopped**, which has nothing to report yet: a `running` execution, and an
`interrupted` one — the pause is mid-superstep, and a resume puts the graph
straight back to work, so there is no finished record to publish. The second is
an execution whose failure carried **no trace at all** — one raised before the
graph ran, so there was never a run to record one. A payload that does not fit
the trigger or the flow's `inputs:` is not among them: that is answered `400`,
before an execution exists to report on.

`outputs` and `error` are the same shape: each appears when the execution has
one, so a `running` report is `execution_id`, `flow`, `trigger` and `status`,
and nothing else.

An `interrupted` report carries one more key, and it is what makes a status poll
enough to *ask* the question rather than only to notice there is one:
`interrupts`, an array with one entry per pause the execution is holding,
ordered by `wait_id` — the same order a `409` lists them in. It is ordered by the
id rather than by the order the pauses began because the order they began is not
a fact about the composition: a `map`'s instances are admitted under
`max_concurrency` and park in whatever order the scheduler interleaved them, so
two runs of one composition would publish the same questions in different orders.
The comparison is over the string, so item `10` sorts before item `2`; what the
order buys is that it is the *same* every run, not that it counts.

Each entry names the pause (`wait_id`, `flow`, `node`), when it began and when
its budget runs out (`paused_at`, and `expires_at` where the node declares a
`timeout:`), what the human is shown (`input`, the node's own `input:`
evaluated), what their answer is held to (`output_schema`, the published JSON
Schema of the node's `output:`), and where to send it (`resume_url`). It is a key
of the `serve` **report** rather than of this format — no entry carries it, and
the emitted project's own `README.md` is where it is documented for the reader
who has to answer one.

---

## 2. The envelope

`TraceDocument`, in the emitted `src/runtime.ts`. It is what the trace file holds
in full, and what `run --format json` spreads into the record it prints.

| field | type | presence | meaning |
|---|---|---|---|
| `trace_version` | integer | always | The format the `entries` are written in. `4` is this document, and a compiled project spells it `TRACE_VERSION` (exported from its `src/runtime.ts`). See [Stability](#10-stability). |
| `flow` | string | always | The flow that was run, as its typed address (grammar §2.2). |
| `execution_id` | string | always | The execution the entries belong to — grammar §4.1's `execution.id`, and the prefix of every idempotency key in the document (grammar §9.4). |
| `status` | `"completed"` \| `"failed"` \| `"interrupted"` | always | How the run ended: with an answer, without one, or holding a `human` pause it had no way to answer (grammar §8.7, §9). The third is told apart from the second because the two ask different things of whoever is reading — one is a run to look into, the other a question to answer — and because a reader may not decide it from the message text (§10.1). |
| `error` | string | on `"failed"` and `"interrupted"` | What stopped the run. Present on every document that carries neither answer, because such a run's last entry does not always say: a run stopped by the superstep ceiling has no aborting node to carry one. On `"interrupted"` it names the node that is waiting and where an answer would come from. |
| `entries` | array of [entries](#3-entries) | always, possibly empty | Every entry the run recorded, in the order §3.1 fixes. Empty only on a run that recorded none at all — one that failed before any node produced an entry, which takes a failure the node a run aborts at cannot account for, since that node contributes one (§9). The trace **file** is not written for such a run (§1.2), so an empty array reaches a reader only as `run --format json`'s `trace` or a `serve` report's. |

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
| `writes` | array of strings | `"completed"`, `"skipped"`, possibly empty | The state channels this node wrote, by name (grammar §10.1). Empty on a skipped node, which writes nothing — and on a **completed** node that landed no channel: one whose `writes:` maps nothing, and one whose result omitted every field that is mapped (grammar §8.0, Decision D110). An empty array is therefore not a statement about `outcome`; read `outcome` for that. Absent on a failed entry: a node that failed produced no output to write from, and where the failure ended the run the superstep it died in lands nothing at all (§9). |
| `routing` | [routing decision](#4-routing-decisions) | `"completed"`, `"skipped"`; on `"failed"` in the one case §9 names | What this node's outgoing edges answered. The one failed case is *no viable route*, where the edge decisions are the whole explanation; every other failure abandoned or never reached the decision. See §4 and §9. |
| `dispatches` | array of [dispatch records](#5-dispatch-records) | `map` nodes with a fan-out to report, possibly empty | What a fan-out dispatched, one record per source item in **index** order (grammar §8.6, PRD 5.6). A map over an **empty** array records `[]` — present and empty. The key is absent, rather than empty, exactly where the fan-out has nothing resolved to report: a map whose input binding failed, so no plan was ever built, and a map whose failure abandoned its plan with no dispatch resolved in it — which §5.2's `timeout:` is the reachable case of. A node that is not a `map` never carries the key. See §5. |
| `toolDispatches` | array of [dispatch records](#5-dispatch-records) | nodes that ran an agent — an `agent:` node, or a `map` dispatching `agent.*` targets — whose tool loop resolved an outcome for a `flow.*` attached to that agent's `tools:` | What a **model** dispatched: one record per flow-as-tool call (grammar §5.4, PRD 5.1), in the order the calls were made — with the hedge §5 states for the records a `map` node could not put in source-item order. The same record type `dispatches` holds and a key of its own, because the two are different fan-outs a single entry can carry at once: a `map` over source items and a tool loop over the calls a model asked for. Present on every entry whose tool loop resolved an outcome for something, whatever became of the node — a call that ended it is in here with the ones before it — and never empty: a node that resolved none carries no key. The one call a record can be missing for is the one a node deadline caught **mid-flight** — caught while the instance it started was still running, so no outcome had resolved when the entry was written. A call the deadline catches earlier started no instance either, and has no half anywhere. §5.2 describes the shape for the other carrier and §5.3 for this one, and §5.3 is the binding account. See §5. |
| `inner` | array of entries | `flow:` nodes that ran an instance | The trace of the subflow instance this node ran (grammar §8.5). Absent on a `flow:` node that ran none — one whose *input* could not be built — and on one whose own `timeout:` abandoned its instance mid-flight, which leaves no trace to carry. A **dispatched** instance is never here: a `map`'s items report under their own dispatch records (§5), including the item whose failure ended the map node. See §8. |
| `stores` | array of [store records](#6-store-records) | when the node performed any | Every store op this node performed, in the order it performed them (PRD 5.8). Never empty: a node that performed none carries no key. See §6. |
| `models` | array of [model calls](#7-model-calls) | when the node made any | Every model call this node execution made (PRD 5.9). Never empty: a node that made none carries no key. See §7. |
| `human` | [a pause](#34-a-human-nodes-pause) | `human` nodes that began a wait | What the wait did: when it began, how long it had, and how it ended (grammar §8.7). On every entry of a `human` node that got as far as pausing, which is all three ways one ends — an answer, an expiry, and a run that ended holding it — told apart *inside* the record rather than by its absence. The key is absent on the one `human` entry with no wait behind it: a node whose *input* could not be built, which fails the execution before the activity runs (`attempts: 0`), exactly as it leaves `inner` off a `flow:` node. A node that is not a `human` node never carries it. **What the human answered is not in it**, and that is a rule of this format rather than an omission — see §11. |
| `error` | string | `"skipped"`, `"failed"` | What went wrong, as `<error name>: <message>` — the failure's class and its text, in that one shape on **every** entry that carries the field, whether the node aborted the run, took a `fallback:`, or had its failure absorbed by `on_error: skip`. On both of those outcomes without exception — including both shapes of `"failed"`, the one that ended the run and the one that took a `fallback:` — and never on `"completed"`: an outcome says what became of a failure, not whether there was one to describe (§3.2). Written for a person: §10.1 makes the text something a reader must not parse, and it can quote what the other side of an activity answered — §11.1 is what it may and may not hold. |
| `fallback` | string | when a declared control transfer replaced this node's own edges | The node id control went to instead, and there are two keys that declare one: `on_error: { fallback: … }` after a failure (grammar §9.2), and a `human` node's `on_timeout:` after its wait ran out (grammar §8.7, which gives it §9.2's targets). `"__end__"` for the terminal pseudo-node. Read `human` to tell the two apart on an entry that could be either. |

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
caller's. Both rules above are the instance's own: a nested array is sorted by
`(step, node)` within itself, and a nested instance that **failed** ends with the
entry it aborted at, exactly as the root's does.

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

### 3.4 A `human` node's pause

`HumanPause`, on `TraceEntry.human`. A `human` node stops its execution until
somebody answers, its budget runs out, or the run ends holding it (grammar §8.7,
PRD 5.5) — and without a record of that, a wait that ran for a day and one that
took a millisecond leave the same entry.

| field | type | presence | meaning |
|---|---|---|---|
| `pausedAt` | string | always | When the wait began, as an ISO 8601 instant. It is the runtime's own clock reading rather than anything derived from the trace's step numbers, because a wait is the one thing in a run whose duration is not the graph's to decide. |
| `expiresAt` | string | when the node declares a `timeout:` | When the budget runs out, as an ISO 8601 instant — `pausedAt` plus the node's `human: { timeout: … }`, which is wall-clock from the moment the pause begins. Absent where the node declares none, which grammar §8.7 makes an **unbounded** wait rather than a defaulted one: a `human` node resolves no `timeout` from any policy level (Decision D102), so there is no other budget for this to have come from. |
| `settledAt` | string | when the wait stopped waiting | When it stopped, as an ISO 8601 instant. Absent on the one entry whose wait nothing settled: a run that **ended** holding the pause, which is what an `agent-compose run` with no way to answer does with every pause it reaches — standard input is not a terminal, or the terminal it was asking at went away (grammar §8.7, PRD §9.21). That entry is the run's aborting entry (§9) and the document's `status` is `"interrupted"` (§2). |
| `settled` | `"resumed"` \| `"expired"` | when `settledAt` is | How it stopped. `"resumed"` is an answer that fit the node's `output:` — one that did not is refused where it arrived and does not consume the wait, so it never becomes part of a run's record at all. Which of the two delivery surfaces it arrived on is **not** recorded: an answer typed at a terminal `run` and one posted to the app's resume route leave the same entry, because they are the same delivery to the same wait (grammar §8.7). `"expired"` is the budget running out, and that entry also carries `fallback` naming the `on_timeout:` route control transferred to (§3, grammar §9.2). The two are settled **exactly once**: an answer racing an expiry is decided rather than applied twice. |

**How it reads beside `outcome`.** The three ways a wait ends are three shapes of
entry, and each is the ordinary reading of the fields it carries rather than a
special case:

* **resumed** — `outcome: "completed"`, the answer written through the node's
  `writes:` like any other result, and the node's own edges evaluated into
  `routing`;
* **expired** — `outcome: "failed"` with `fallback` and no `routing`, which is
  §3.2's second shape: the node left no result for the run to carry on from, and
  a declared control transfer replaced its edges. `error` names the budget that
  ran out;
* **held to the end of the run** — `outcome: "failed"` with neither `fallback`
  nor `routing`, `error` naming what was waited for and where an answer would
  have come from, and the document's `status` `"interrupted"`.

---

## 4. Routing decisions

`RoutingDecision`, on `TraceEntry.routing`. This is the record PRD 5.3 asks for:
which edge fired, and the guard values that decided it.

| field | type | presence | meaning |
|---|---|---|---|
| `edges` | array of [edge decisions](#41-edge-decisions) | always | What every outgoing edge answered, in **declaration order** (grammar §7.3). Never empty: a node with no outgoing edge is a static error (grammar §7.6.3 rule 1). |
| `targets` | array of strings | always, possibly empty | The nodes scheduled next, in the declaration order of the edges that reached them, deduplicated — the same order `edges` is in, filtered to the taken ones. `"__end__"` is the terminal pseudo-node. A multicast (grammar §7.3 rule 6) names more than one. Empty in exactly one place, and it is what went wrong there: the *no viable route* entry of §9. |
| `counters` | object, string → integer | always, possibly empty | The `max_iterations` counters this step **spent**, by key, holding their new value (grammar §7.4). A key appears only when this step spent it; the value is the count after the spend. `{}` on every step that spent none, which is most of them. |

### 4.1 Edge decisions

`EdgeDecision`. One per outgoing edge, in declaration order — including the edges
that were **not** taken, which is the half a reader most often needs.

| field | type | presence | meaning |
|---|---|---|---|
| `to` | string | always | The target node id, or `"__end__"`. |
| `when` | string | guarded edges | The `when:` guard, as CEL source, verbatim from the composition (grammar §4.1). |
| `else` | `true` | `else:` edges | Marks the edge as the `else:` catch-all (grammar §7.3 rule 4). Never `false`: an edge that is not the catch-all omits the field. |
| `value` | boolean | guarded edges | What the guard answered. |
| `budget` | object | budgeted edges whose guard answered `true` | The `max_iterations` budget on this edge, and its state at this decision: `key` (the counter this edge spends), `used` (the count **after** this decision), `max` (the declared budget). A guard that answered `true` is when a budget is either spent or found spent; a budgeted edge whose guard answered `false` spent nothing and reports no budget. Only a guarded edge can carry one at all — `max_iterations:` requires a `when:` (Decision D90) — so an unconditional or `else:` decision never has this key. |
| `taken` | boolean | always | Whether this edge scheduled its target. |
| `reason` | string | the three decisions below, and no others | Why, where neither the guard's value nor the edge's own shape says. See below. |

`reason` takes exactly three values in this version, and each names a rule rather
than describing one. The table is both the vocabulary and the presence rule: a
decision this table does not describe carries no `reason` at all.

| `reason` | when |
|---|---|
| `"unconditional"` | an edge with no `when:` and no `else:`, which is always taken (grammar §7.3 rule 2) |
| `"the \`max_iterations\` budget is spent"` | a guarded edge whose guard answered `true` and whose budget was already at `max`, so it was not taken (grammar §7.4) |
| `"a guarded sibling was taken"` | an `else:` edge suppressed because a guarded sibling of this node was taken (grammar §7.3 rule 4) |

So the two decisions a reader meets most often carry none, and an untaken edge is
**not** in itself a decision that carries one:

* a **guarded edge that answered `false`** has `value: false`, `taken: false` and
  no `reason` — the guard value is the whole explanation;
* an **`else:` edge that was taken** has `else: true`, `taken: true` and no
  `reason` — nothing suppressed it, which is all its being taken means.

A reader that wants a sentence for every edge composes those two itself. Reading
`reason` off an arbitrary untaken edge does not work and is not something this
version promises.

### 4.2 What terminated a cycle

A cycle (grammar §7.4) leaves through one of two decisions, and both are in the
`edges` array of the entry where it happened:

* **a counting bound ran out** — the back-edge's decision has `value: true`,
  `taken: false`, the `"the \`max_iterations\` budget is spent"` reason, and a
  `budget` naming the counter, the count and the declared `max`.
* **a CEL exit condition went false** — the back-edge's decision has
  `value: false`, `taken: false`, and no reason.

In both, the `else:` edge beside the back-edge is `taken: true` — on the shape a
bounded cycle has, which is a node whose only guarded edge is the back-edge. The
rule underneath is §4.1's and it is what a reader should apply where a node
guards more than one way out: an `else:` edge is taken unless some guarded
sibling was, and a sibling suppressed it says so in its `reason`.

The passes before the last are the same edges seen `taken: true`, one entry per
traversal, with `budget.used` climbing on a counted loop. A cycle that never left
at all — grammar §7.4 clause 2 admits one whose guard never goes false — is a run
the superstep ceiling stopped: `status: "failed"` with an envelope `error` naming
the ceiling, and every traversal it did make in `entries`.

---

## 5. Dispatch records

`DispatchRecord`. **Two fields carry it**, and a reader that walks both has every
instance a run started under something that dispatched it:

| carrier | one record per | order |
|---|---|---|
| `TraceEntry.dispatches` | source item of a `map`'s fan-out (§5.2 is the one path that reports fewer, and it says which) | ascending `index` — never completion order |
| `TraceEntry.toolDispatches` | flow-as-tool call an agent's loop made (grammar §5.4, PRD 5.1) | the order the calls were made — see the hedge below, which is §7.2's for `models` |

PRD 5.6 makes a fan-out's cardinality and destination *data*, and PRD §9.20 makes
a model-invoked subflow findable as one shape rather than two; the record is that
shape, and the two keys are what keep the two dispatchers' `index` spaces apart
on the one entry that can hold both — a `map` whose target is an `agent.*` with a
`flow.*` in its `tools:`.

**`toolDispatches` carries §7.2's hedge**, and for the same reason `models`
does: the entry holds every record the node execution produced, and only some of
them are records the node can put in an order. On a plain `agent:` node the loop
made them one at a time and the array is that order, entire. On a `map` node the
records of the items that **landed** are concatenated in ascending source-item
index — and the records of an item the fan-out did not land, one
`on_item_error: skip` absorbed or an exhausted `retry:` attempt made, could not
be attributed to a position and come **ahead** of them in the order they were
made. With concurrent items that leading portion is in whatever order the
scheduler produced, so it is the one part of this array two runs of one
composition may spell differently. `idempotencyKey` is what identifies a record
there, as it is for the repeated `index` the row below describes.

A **`map` fan-out over an empty array** has no source items, and its entry
carries `dispatches: []` — the key present, the array empty. Grammar §8.6 rule 11
makes a zero-instance dispatch a completion, and the entry reads as one: the map
node ran, dispatched nothing, and its outgoing edge fired. The *absent* key is a
different statement, and §5.2 is where it is made. `toolDispatches` has no such
value: how many times a model calls a tool is the model's, and none is simply the
key's absence (§3).

| field | type | presence | meaning |
|---|---|---|---|
| `index` | integer | always | What identifies this dispatch within the array that holds it, and orders the array. On `dispatches` it is the **source-item index**, which is also what orders every write the item made (PRD 5.6, grammar §7.6.4 clause 2). On `toolDispatches` it is the **call ordinal** grammar §9.4 gives the call — how many times that tool had already been *invoked* in this agent execution, which is a count of the calls that got past the tool's contract rather than of the calls the model made (grammar D119) — so two records under one key can repeat an index where a `map` dispatched the agents that made them, and `idempotencyKey` is what tells those apart. |
| `route` | string | routed maps | The **route** the item was dispatched through: its variant tag, or `"$default"` for the `default:` catch-all (grammar §8.6 rule 4, Decision D30). Absent on the homogeneous form, which has one target and no tags, and on every `toolDispatches` record, which no `map` routed. |
| `variant` | string | routed maps | The **discriminator value the item carried** — the value at the map's `route_by:` field. On a named route it repeats `route`; on the catch-all it is the only record of which variant fell through, since `route` names the catch-all rather than the variant. It is always one of the union's declared variant tags, and that is also what makes the key present on **every** record a routed map files: the item was parsed against its producer's declared schema before any of this ran (PRD 5.2), so its discriminator is one of those tags. An item whose discriminator is not a string — which no artifact `build` accepted can produce — is left unrecorded rather than rendered, since a number written as a string would be a `variant` that is not a declared tag. Absent on every `toolDispatches` record, for `route`'s reason. |
| `target` | string | always | The component the dispatch went to, as a typed address — the item's target on `dispatches`, and the `flow.*` the model called on `toolDispatches`. |
| `outcome` | `"completed"` \| `"skipped"` \| `"failed"` \| `"detached"` | always | See §5.1. A `toolDispatches` record takes `"completed"` or `"failed"` and neither of the other two: there is no per-call error policy to skip a call, and no `detach:` on a tool attachment. |
| `attempts` | integer | always | How many attempts were **made** at this dispatch. On `dispatches` that is what the item's `on_item_error: { retry: … }` policy made (grammar §8.6 rule 10), and `0` for a detached dispatch, which has no observed outcome for a policy to have acted on. On `toolDispatches` it is always `1`: a tool attachment carries no per-call policy, so one call is one attempt — a *node*-level `retry:` re-runs the whole tool loop and files fresh records instead (§9.4 of the grammar, and §8 below). |
| `idempotencyKey` | string | always | The key this dispatch's effect site derives (grammar §9.4). See §8. |
| `inner` | array of [entries](#3-entries) | **joined** `flow.*` targets, and every `toolDispatches` record | The instance's own trace, whether it completed or failed — such a dispatch always has one, because the instance either answered with its trace or failed carrying it. Absent on every other `map` target, which ran no instance, and on a `"detached"` record whatever its target: see below. With `TraceEntry.inner` this is the **only** place an instance's trace appears; the node's own `inner` is for a `flow:` node's instance, and neither a `map` nor an agent's tool loop is one (§3, §8). |
| `error` | string | `"skipped"`, `"failed"` | Why the dispatch did not complete. On both, and in §3's `<error name>: <message>` shape: `on_item_error` decides which of the two outcomes a failed item takes (§5.1), not whether there was a failure to describe. |

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

  That last sentence is a rule about the **whole entry**, not only about this
  record: nothing a detached delivery goes on to do reaches the map node's
  entry. Its model calls are not in the entry's `models` and its store ops are
  not in the entry's `stores`, whether the sink answers before the join returns
  or an hour later. The alternative is not a fuller trace but a
  nondeterministic one — the entry is written when the join finishes, and what a
  delivery it does not wait for had managed by then is a matter of scheduling.
  So the account of a detached dispatch is exactly this record, and §7.2's "every
  model call that node execution made" is bounded by the same join that bounds
  the outcomes.

A `toolDispatches` record takes `"completed"` or `"failed"` and neither of the
other two, and the two read as they do everywhere else: the instance answered and
its `outputs:` went back to the model as the tool's result, or it did not and
nothing went back at all. A flow-as-tool call that failed **never** reaches the
model as a plausible result — the failure leaves the tool and the agent node's
own `on_error:` decides the run (grammar §9.2), which is the same shape a
`tool.*` whose request was refused has.

One failure is **not** the agent node's `on_error:`'s, and it is the record a
reader is likeliest to misread: a `human` node inside the flow the model called,
on a run with no way to answer it. A pause leaves ahead of every strategy
(grammar §8.7), so nothing decides it — but the record the call had already
filed stays on the entry, reading `outcome: "failed"` with a `HumanInterrupt`
`error` while the document's `status` is `"interrupted"`. Read that pair the way
§9 reads the aborting entry it sits on: the instance did not go wrong, it is
parked at a question nobody could answer, and the document's `status` — never
the record's outcome — is what says which. The `toolCalls` entry naming this
record is on the entry's `models` beside it and reads the same way (§7.3, §9).

The sibling carrier does not file one at that moment, and the asymmetry is an
invariant's rather than an accident: PRD §9.20 makes every instance a **model**
started findable as a dispatch record, so a flow-as-tool call files its record
whichever way it ended, while a `map`'s records are the outcomes that
**resolved** (§5.2) and a pause resolves none.

### 5.2 A map node that failed

A map node that failed still dispatched, and its entry still carries what it
dispatched — items that ran already had their effects. Two failure paths differ
in what survives:

* the map node **raised** (an item failed, or its own `on_error:` absorbed the
  fan-out): the records are complete, one per source item.
* the map node's own **`timeout:`** fired (grammar §9.2, §8.6 rule 9): the
  deadline is raced, so the records are those that had **resolved** — every
  detached delivery, which resolves at dispatch, and every joined instance that
  had settled. The entry's `error` names the budget that ended it.

  What the deadline caught **mid-flight** is where this format stops promising.
  Such an instance has no outcome at the moment the budget runs out, so there is
  nothing to record for it — but it is not stopped dead either: it unwinds
  against the aborted signal, and whether that unwinding lands a record before
  the entry is written is a matter of scheduling. So read a missing record as
  "no outcome resolved for this item", never as "this item never ran", and read
  a record for an item the deadline caught as an outcome that did resolve rather
  than as a contradiction. A deadline that caught every instance can therefore
  leave no records at all, and the entry then carries no `dispatches` key rather
  than an empty array — an absent key says "nothing resolved", where an empty
  array would say "nothing was dispatched".

### 5.3 An agent node a deadline caught mid-call

The same thing happens one carrier over, and it is worth stating separately
because the two keys have different vocabularies of absence. An `agent:` node's
own `timeout:` (grammar §9.2) is raced against its tool loop, so a call the
deadline catches **while the instance is still running** has no outcome at the
moment the budget runs out: `toolDispatches` carries the calls that had settled
and not that one, and where it was the only call the key is absent rather than
empty. `ModelCall.toolCalls` (§7.3) is absent on the same call for the same
reason, and where that call was the first its answer asked for, the model call's
key is absent too.

Read those absences as §5.2 says to read its own — "no outcome resolved for this
call", never "this call never ran". The abandoned instance is not stopped dead:
it unwinds against the aborted signal, and whether that unwinding lands a record
before the entry is written is a matter of scheduling. What is **not** a matter
of scheduling is the entry: it is complete when it is written, and an outcome
that resolves afterwards does not appear on it. So an absence here is a fact
about the entry rather than a value a reader might see change.

`toolDispatches` has no counterpart to `dispatches: []` here, because a tool loop
that dispatched nothing and a tool loop that resolved nothing are the same
absence: how many times a model calls a tool is the model's, and none of them is
the key's absence (§3).

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
| `key` | string | `get`, `set`, `delete`, `upsert`, `put` | The key the op addressed. Absent on the two ops of grammar §11.4's catalog that address no single key: `list`, which takes a `prefix`, and `search`, which takes a `query`. |
| `answer` | any | `"read"` | What the read answered — the history a replay consumes. On **every** read record: a read that found nothing still answered, and grammar §11.4's own output row is what it answered with — a `get` that missed carries `{ found: false }` and no `value` (Decision D110), which is an answer and not an absence. Its shape is the store's (grammar §11.4), not this format's. |
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
`retry:` policy made and **every instance a `map` joined**: an effect that
happened is an effect that happened, and a record that kept only the last
attempt's would describe a run the store did not see. That is the same bound
§7.2 puts on `models`, and it has the same one exception. A **detached** `map`
delivery is what "a node performs" does not reach, for the reason §5.1 gives:
the node never waited for it, so whether its ops had happened by the time the
entry was written is a matter of scheduling rather than a fact about the run.

"On *that* node's entry" is decided by where the op ran, and a fan-out has both
shapes. A joined instance dispatched to an `agent.*` or a `tool.*` has no entry
of its own, so its ops are the map node's. An instance dispatched to a `flow.*`
is a run of its own graph, whose nodes have entries: its ops are on those,
reached through `DispatchRecord.inner` (§5), and none of them is on the map
node's entry. A `flow:` node divides the same way, through `TraceEntry.inner`.

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
| `failovers` | array of [refusals](#71-refusals) | always, possibly empty | Every member that refused **and moved the ladder on**, in the order they were tried. Empty on every call that did not fail over — a direct binding's, and a route whose first member answered — which is most of them. |
| `refused` | [refusal](#71-refusals) | when no member answered | What ended the call. Such a record carries no `servedBy` and no `fallback`. |
| `toolCalls` | array of [tool calls](#73-tool-calls) | when this call's answer asked for at least one and the loop resolved it | What the model asked the agent's tools to do, in the order its answer asked, and what became of each (§7.3). Never empty: a call whose answer asked for none carries no key, and the pinned structured-output call that ends a loop is always one of those. A call that **ended the node** — a `"failed"` one — is the last entry rather than a missing one: the calls after it in the same answer never ran, and are absent because they did not happen. A `"refused"` one is not the last of anything: every call of an answer comes back to the model, refused or not, so the records after it exist — grammar D119 is that rule, and names the one wire shape that carries a refusal outside its own call's id. The one tool call with no entry is the one a node deadline caught **mid-flight** (§5.3), which resolved neither a result nor a failure; where it was the first the answer asked for, this key is absent rather than empty. |

Every record therefore carries exactly one of the two accounts of how it ended:
`servedBy` with `fallback`, or `refused`. `failovers` is beside whichever it is,
and describes the way there rather than the end of it.

A **direct** binding produces a record too, with `servedBy === model`,
`fallback: 0` and an **empty** `failovers` — the key present, holding nothing: a
trace that recorded only the interesting calls would leave a reader unable to
tell a call that did not fail over from a call nothing recorded.

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
every attempt its `retry:` policy made and every instance a `map` **joined** —
not only the calls of the attempt that answered. A node that succeeded on its
second attempt reports the first attempt's spent ladder too. A **detached**
delivery's calls are not here, for the reason §5.1 gives: the node never waited
for it, so what it had managed by the time the entry was written is a matter of
scheduling.

"That node execution made" divides a fan-out exactly as §6 divides its store
ops, and for the same reason — where the call was made. A joined instance
dispatched to an `agent.*` has no entry of its own, so its calls are the map
node's; a `tool.*` makes none. An instance dispatched to a `flow.*` is a run of
its own graph, whose nodes have entries: its calls are on those, reached through
`DispatchRecord.inner` (§5), and none of them is on the map node's entry. A
`flow:` node divides the same way, through `TraceEntry.inner`. So "joined" is
not the whole of the bound: a per-node roll-up summing `models` over a `map`
that fans out to `flow.*` targets finds nothing on the map node and every call
one level down.

Where the two sets differ, calls the node could **order** come in that order (a
`map`'s, which is source-item order) and the rest come ahead of them in the order
they were made. This format does **not** attribute a call to a source item: a map
node's `models` is the concatenation, and the item that made a given call is not
recorded.

### 7.3 Tool calls

`ToolCallRecord`, on `ModelCall.toolCalls`. An agent's tool loop is the one place
a compiled graph does work a *model* asked for, and without this the trace says a
call was made and nothing about what it set off. PRD §9.20 fixes the division:
the loop's story is complete inside the [model call](#7-model-calls) it belongs
to — what was asked for, what came of it, and the result the model saw — at the
cost of one indirection for the part that is a run of its own.

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The tool the model called, spelled as the request offered it — a `tool.*`'s local name, a `flow.*`'s (grammar §5.4), a synthesized store tool's (grammar §11.5), or a runtime built-in's (`bash`, `read_file`, `write_file`, `list` — grammar §5.5). |
| `target` | string | when the agent offers a tool of that name | The component behind the name, as a typed address (grammar §2.2). Absent on the one call that has none: a name the agent does not offer, which is a model answering with a tool that was never on the wire. That call is recorded `"refused"` and handed back to the model with the names it does have (grammar D119). |
| `outcome` | `"completed"` \| `"refused"` \| `"failed"` | always | What the loop did with the call. `"completed"` handed the model the tool's result. `"refused"` handed it the **refusal** instead: the tool's declared contract did not admit the call — arguments its schema refuses, on any of the three surfaces, or a name the agent never offered — and grammar D119 makes that a call the model is asked to make again, so the node did not end and records after it exist. `"failed"` is the tool's *execution* failing, which ended the node: the failure left the tool, the node's own `on_error:` decided the run (grammar §9.2), and the model saw nothing back. §5.1 names the one failure no `on_error:` decided: a `human` node inside the flow the call ran, on a run that could not answer it. |
| `instance` | string | flow-as-tool calls that started an instance | The **link**: the subflow instance this call ran, named exactly as the dispatch record carrying that instance's trace names itself in `idempotencyKey`, so the join between the two is string equality (§5, §8). Absent on every call that instantiated nothing — a `tool.*`, a store tool — and on a `"refused"` flow-as-tool call, which is arguments that failed the flow's own `inputs:` before an instance existed. A refused call spends **no** call ordinal (grammar §9.4, D119) — that ordinal counts invocations and this call reached no flow — so the instance path of the call that follows it is the one it would have had with no refusal ahead of it. |
| `result` | any | `"completed"` flow-as-tool calls, with `instance` | **The result the model saw**: the value the loop handed back, which for this tool is the instance's declared `outputs:` (grammar §5.4). PRD §9.20 asks the tool-call entry to record it, and it is the one tool result this format carries — §11 is where the rule it is carved out of is stated, and where the other two tool surfaces are left under it. Its calls are a **subset** of `instance`'s, not the same set: `instance` says an instance ran, `result` says the loop handed that instance's outputs back, so a call carrying `result` carries `instance` and not the other way round. Absent on a `"failed"` call and on a `"refused"` one, where the absence is the record — the first left the tool and the second never entered it, and in neither did the model see a result. A flow-as-tool call whose instance failed is exactly that call with an `instance` and no `result`. |
| `error` | string | `"failed"`, `"refused"` | What went wrong, in §3's `<error name>: <message>` shape. On a `"refused"` call the `<message>` half is **byte for byte the sentence the model was handed back**, and the `<error name>` half — `ToolCallRefused` — is this format's own envelope, which the model's copy does not carry: the two strings differ by that prefix and by nothing else, so a reader joining a record to a provider transcript compares the record's message half, never the whole string. That sentence names the tool, the field and the constraint the way a compiler diagnostic would (PRD G3) and may quote an excerpt of the arguments — see §11. |

**What is deliberately not here.** The **arguments** the model sent are absent
as a field of their own, and that is §11's rule rather than an omission: this
format carries no provider transcript, and what the call did to the run is
reachable through the instance `instance` links to. `result` is the one thing on
the other side of that rule, and it is here because PRD §9.20 put it here — "a
bare dispatch record alone would leave a tool call whose result came from
nowhere". It is carried for the one tool whose result is the composition's own
declared data; a `tool.*`'s answer and a store tool's stay out, the second of
them because §6 already carries it in `StoreRecord.answer`.

A **runtime built-in**'s answer stays out with them, and that is the whole of
what the built-ins changed here: a `bash`'s stdout and a `read_file`'s contents
are a tool's answer under §11's rule, so what this format records is the call —
`name`, `target: "builtin.bash"`, the outcome, and the error where there was
one. The full answer is in the durability journal, which is private recovery
data rather than a document a run hands out (`docs/durability.md` §3.2, §8, PRD
resolved q31).

A `"refused"` call's `error` is where an excerpt of those arguments can appear,
and it is not an exception to the rule above but the same one read where the
message goes: the refusal is written **for the model**, which chose the
arguments and is being asked to choose again, so quoting the offending value
back at it costs nothing (grammar D119, §11.1).

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
* `DispatchRecord.inner` — the instance something *dispatched*, whether it
  completed or failed: an item a `map` sent to a `flow.*` (grammar §8.6), on
  `TraceEntry.dispatches`, and a flow-as-tool call a model made (grammar §5.4),
  on `TraceEntry.toolDispatches`.

**One instance appears in one of those two places, never in both.** Neither a
`map` nor an agent is a `flow:` node, and neither carries `inner` on its own
entry however its dispatches went — including the item whose failure ended the
map node, and the tool call whose failure ended the agent node, each of whose
instances is under its own record like every other. So a reader walking a trace
for every subgraph run visits `TraceEntry.inner` and both carriers of
`DispatchRecord.inner`, and counts each instance once.

Nesting moves nobody's step numbers: an inner instance numbers its own supersteps
from `1`. An instance that **failed** carries the entry it aborted at last, the
way §3.1 says a failed run does — the ordering rule is the instance's, not only
the root's.

**Instance paths surface in exactly three fields**, and in none of them alone:
each is an idempotency key, of which the path is the **remainder** after the
execution id. Two are keys a delivery really carried —
`DispatchRecord.idempotencyKey`, on every dispatch record under either carrier,
and `StoreRecord.idempotencyKey`, which a store-op **node**'s write carries and a
write through a synthesized store tool does not (§6). The third is a **link**
rather than a key: `ToolCallRecord.instance` (§7.3) repeats the
`idempotencyKey` of the dispatch record its call filed, so a reader joins a tool
call to the instance that answered it by string equality. All three are

```
<execution.id> "/" <frame> { "/" <frame> }
```

where each frame is `<node id> "/" <traversal ordinal>` — plus `"/" <item index>`
for a `map` node — for every node crossed from the **root** flow instance down to
the effect site, outermost first, and where a **flow-as-tool call** contributes
one more frame of its own beneath the agent node's, `<tool name> "/" <call
ordinal>` (grammar §9.4, PRD resolved q19). The envelope's `execution_id` is that
first component, so a reader can strip it and read the remainder as the path.

```
exec_01/dispatch/0/7                 the detached dispatch of item 7 by map node `dispatch`
exec_01/outer/0/3/inner/0/0/save/0   store node `save`, under item 0 of `inner`, itself item 3 of `outer`
exec_01/ask/0/condense/1             the second call agent node `ask` made to its `condense` flow-tool
```

Grammar §9.4 is normative for the derivation; this document only fixes where it
appears. Two properties are what make it worth reading out of a trace: distinct
effects get distinct keys, and a repeated attempt at one effect reuses its key —
so `deduped: true` on a store write means an earlier attempt of *this* effect had
already been applied, rather than some other write colliding.

The second property is **positional** across a flow-tool frame, and grammar §9.4
says so openly: an agent-node `retry:` restarts the call ordinals, so the Nth
*invocation* of a retried attempt derives the Nth key of the failed one — while
what a nondeterministic model asks for the Nth time is not guaranteed to be the
same work. That is the at-least-once compromise the grammar accepts; nothing in
this format hides it, and the two attempts' records are both on the entry, in the
order they were made.

**Two policies re-run a tool loop, and both reuse its keys.** A node's own
`retry:` is the one above. The other is a `map`'s
`on_item_error: { retry: … }` (grammar §8.6 rule 10), which re-executes a
dispatched `agent.*` from its entry at the same source index — so the item's
frames are unchanged, the loop starts counting from zero again, and the Nth
invocation of the second attempt derives the first attempt's Nth key as a node
retry does. A reader meets it as repeated keys under one `toolDispatches` array,
`DispatchRecord.attempts` on the *item's* record being where the retry itself is
recorded (§5).

---

## 9. Failed runs

A run that produced no answer has a trace, and it is the trace a reader most
often wants. Both `status` values that mean "no answer" — `"failed"` and
`"interrupted"` — write the same document, and everything in this section holds
for either: an interrupted run is one whose aborting node is a `human` node that
was still waiting (§3.4), and there is nothing else different about it. What is
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

Everything else the node did is still there: `stores`, `models`, `dispatches`,
`toolDispatches` and `inner` reach the aborting entry like any other, because
those effects really happened and the failure alone says nothing about them.
Failing is not a sixth presence rule — each of the five is on this entry exactly
when §3's row for it says, so a node that made no model call still has no
`models`, a `map` whose deadline resolved nothing still has no `dispatches`
(§5.2), and an agent node whose deadline caught its only tool call still has no
`toolDispatches` (§5.3). The agent node a flow-as-tool call ended is the shape
that reads oddest and is the ordinary reading of the same rules: its
`toolDispatches` holds the instances the loop ran, the last of them `"failed"`,
and its `models` holds the call whose `toolCalls` names it (§7.3).

A run stopped by the **superstep ceiling** has no aborting node at all: the
ceiling is the compiler's safety net rather than one of the composition's own
bounds, and it stops the run between supersteps. Such a document has every entry
that landed and no final failed entry; the envelope's `error` is what names the
ceiling.

An **interrupted** run's aborting entry is the one place a `"failed"` entry is
not about something going wrong: the node did everything it was asked to and is
holding a question (grammar §8.7). It reads as every other aborting entry does —
no `writes`, no `routing`, no `fallback`, an `error` saying what stopped the run
— plus the `human` record whose missing `settledAt` is what says the wait was
never settled. The document's `status` is what a reader branches on; the message
text is not (§10.1).

---

## 10. Stability

`trace_version` is what a reader pins. This section is what pinning it buys.

### 10.1 What a reader may rely on

At a given `trace_version`, a reader MAY rely on:

* every field this document names, under the name and with the meaning given
  here;
* the presence rules stated in each table's *presence* column, read in the
  vocabulary the preamble fixes — **both ways round.** A column that names the
  cases a field appears in is also the statement that it does not appear in the
  others, and in some of them the absence *is* the record rather than a detail of
  it: §5.2's missing `dispatches`, where an absent key says "nothing resolved"
  and `[]` says "nothing was dispatched"; the missing `writes` and the
  mostly-missing `routing` of §9's aborting entry; `route` and `variant` on a
  homogeneous map, which is how a reader tells the two forms apart. Reading an
  absence this document states is using the format, not guessing at it — see the
  MUST NOT below for the absences that are not stated, which is a different
  thing;
* the **, possibly empty** qualifier, and its absence, as the same kind of
  statement. Where a cell carries it, an empty array or object is a value this
  format produces and means what the row says it means; where a cell does not,
  the field is present only with something in it, and a reader may treat an empty
  value there as impossible rather than as a case to handle;
* the vocabularies of the closed enumerations: `TraceDocument.status`,
  `TraceEntry.outcome`, `DispatchRecord.outcome`, `ToolCallRecord.outcome`,
  `HumanPause.settled`,
  `StoreRecord.op`,
  `StoreRecord.effect`, `StoreRecord.via`, `StoreRecord.scope`, a refusal's
  `condition`, and an edge decision's `reason` — the last being a closed
  vocabulary spelled as a sentence, which §4.1 enumerates and the MUST NOT below
  names as the one message-shaped field a reader may match on. `op` is the one
  whose type in `src/runtime.ts` is `string` rather than the union — the union is
  the emitted `src/stores.ts`'s `StoreOp`, and a record type declared under the
  runtime cannot name it without inverting that dependency — so §6's seven are
  its vocabulary, and the inventory checks them against `StoreOp` itself;
* the orders §3.1, §4.1, §5 and §7.3 fix — entries by `(step, node)`, edge
  decisions in declaration order, a `map`'s dispatch records in source-item index
  order, a tool loop's in call order, and a model call's `toolCalls` in the
  order its own answer asked. Two of those carry a hedge where the node could
  not order every record it has to report, and the hedge is part of what is
  fixed: §7.2's for `models` and §5's for `toolDispatches`;
* the nesting structure of §8, and the shape of the keys it describes.

A reader MUST NOT rely on:

* **the absence of a field this document does not name.** A later version may add
  one, and a reader that rejects unknown keys — or that treats "no such key
  today" as a fact about the format — will break on a compatible change. Ignore
  what you do not recognize. This is the complement of the presence rule above
  and not a retraction of it: an absence a *presence column* states is part of
  the contract, and widening one is a version bump (§10.3).
* **the text of any message field** — `TraceEntry.error`, `DispatchRecord.error`,
  `Refusal.detail`, and the envelope's `error`. These are diagnostics written for
  a person (PRD G3) and are improved between releases. §3's `<error name>:
  <message>` is the shape they are written in, not a parse: the class in front of
  the colon is the class of the error the runtime raised — which for an
  activity's own failure is the wrapper the runtime puts around it, with the
  activity's class appearing later in the text, and on the input-binding and
  routing paths is something else again — and neither the set of classes nor the
  text after it is fixed by this format. The `reason` field of an
  edge decision is the exception, and only because §4.1 enumerates its values: it
  is a closed vocabulary that happens to be spelled as a sentence.
* **anything printed by the human report.** See §11.
* **the shape of `StoreRecord.answer`**, which is the store's, not this
  format's, or of `ToolCallRecord.result`, which is the subflow's `outputs:`
  (grammar §5.4) and moves when the composition does.

### 10.2 What is a compatible change

Compatible, and made **without** a version bump:

* adding a field to an existing record type;
* adding a new record type reachable from an existing one;
* recording a field in a case this document's presence column **already** names
  and the runtime was not keeping — a bug in the implementation of this format
  rather than a change to it, and the reason §10.4's two tests exist;
* improving the text of a message field.

### 10.3 What requires a version bump

`trace_version` MUST be incremented for any change that would make a reader
written against the previous version wrong:

* removing a field, or renaming one;
* changing a field's type, or the meaning of its value;
* changing a presence rule in **either** direction — recording a field in fewer
  cases than this document promises, and equally in more. §10.1 makes a presence
  column readable both ways, so widening one moves an absence a reader was told
  to rely on: a reader who tells a timed-out fan-out from an empty one by §5.2's
  missing `dispatches` is broken by a release that starts writing `[]` there,
  exactly as one who reads `dispatches` is broken by a release that stops;
* adding or removing the **, possibly empty** qualifier on a presence cell. It is
  a presence rule in its own right — the preamble makes a cell without it a
  promise that the value has something in it — so a field that starts producing
  an empty array where this document said it produced none breaks the reader who
  was told not to handle one;
* adding a member to one of §10.1's closed enumerations, or removing one;
* changing one of the fixed orders;
* changing the derivation of an idempotency key, or where instance paths appear.

A bump changes the number on every surface at once — the file, `run --format
json`, and `serve` — because they carry one format. A release that bumps it says
so in its notes, and this document's *Trace version* header moves with it.

### 10.3.1 What version `2` changed

The `human` node runtime (grammar §8.7, PRD 5.5) added one record and moved two
promises a reader of version `1` had been given, which is what made it a bump
rather than an addition:

* **`TraceDocument.status` gained `"interrupted"`** — a member added to a closed
  enumeration. A run that ends holding a pause is not a run that failed, and a
  reader could not have been left to tell the two apart from `error`'s text,
  which §10.1 forbids parsing;
* **`TraceDocument.error` widened with it** — §2's envelope row now reads "on
  `"failed"` and `"interrupted"`", which is a presence rule recorded in more
  cases than version `1` promised. `TraceEntry.error` is a different field and
  version `2` did not touch it: §3's row still reads "on `"skipped"`,
  `"failed"`";
* **`TraceEntry.fallback` widened** — version `1` said the key appeared when
  `on_error: { fallback: … }` fired, and a wait that runs out its budget now
  writes it too, naming the `on_timeout:` route (§3, grammar §9.2). Its *meaning*
  is unchanged — the node id control went to instead of this node's own edges —
  but the cases it appears in are not, and a reader who read the key as "an
  `on_error:` fallback fired" is wrong under `2`. `TraceEntry.human` is what
  tells the two apart;
* **`TraceEntry.human` was added**, and with it the pause record §3.4
  specifies. That half is a §10.2 addition and would have needed no bump on its
  own.

Nothing was removed or renamed, and no order changed.

### 10.3.2 What version `3` changed

The flow-as-tool runtime (grammar §5.4, PRD 5.1, resolved q19 and q20) added two
record surfaces and moved one promise a reader of version `2` had been given.
Only the last of the three needed a bump; it is here rather than in §10.2 because
of what §10.3's last bullet names — *where instance paths appear*:

* **`TraceEntry.toolDispatches` was added**, holding the same dispatch record
  §5 already specified. That half is a §10.2 addition;
* **`ModelCall.toolCalls` was added**, and with it the tool-call record §7.3
  specifies — a new field, and a new record type reachable from an existing one,
  which §10.2 makes compatible twice over. `ToolCallRecord.result` is part of
  that record rather than a change to an older one: PRD §9.20 asks the entry to
  carry the result the model saw, and §11's rule about results is amended to say
  so at the one surface this record covers. A reader of version `2` meets it
  nowhere, because it is reached only through the key version `3` added;
* **an instance path surfaces in a third field.** Version `2`'s §8 opened with
  "instance paths surface in exactly two fields" and named both, which §10.1
  makes a reader entitled to rely on; `ToolCallRecord.instance` is a third, and
  §10.3 names widening that promise as a bump in its own right. The
  **derivation** did not change — grammar §9.4 gained a frame for a call site it
  had not had, and every path a version `2` composition produced is the path it
  produces now;
* two rows of §5 read wider with the second carrier, and both are the same field
  meaning one thing per carrier rather than a changed meaning: `index` is a
  source-item index on `dispatches` and a call ordinal on `toolDispatches`, and
  `attempts` is an item policy's attempts on the first and always `1` on the
  second. A reader of version `2` meets neither, because both are reached only
  through the key version `3` added.

Nothing was removed or renamed, and no order changed.

### 10.3.3 What version `4` changed

A tool call the model can fix by calling differently now goes **back to the
model** instead of ending the agent node (grammar
[D119](grammar.md#d119-a-refused-tool-call-returns-to-the-model-and-a-failed-one-ends-the-node),
PRD §9.22). That is a change to the *runtime*, but it reaches a reader of version
`3` through one field, and in the one way §10.3 does not forgive:

* **`ToolCallRecord.outcome` gained `"refused"`** — a member added to a closed
  enumeration, which §10.3 names outright. The alternative was to keep the
  two-member vocabulary and record a refused call as `"failed"`, and it is
  **worse rather than cheaper**: version `3` defines `"failed"` as a call that
  ended the node and whose answer the model never saw, and both halves are false
  of a refusal — the loop carried on, and the model was handed the sentence
  `error` spells under §3's envelope. Reusing the member would have changed the
  meaning of a value a reader was told to rely on, which §10.3 bumps for too,
  while leaving that reader unable to tell the two apart at all. A new member
  costs the same bump and leaves `"failed"` meaning exactly what it meant;
* **two presence rules widened with it**, and both are read off the row rather
  than added to it: `error` now appears on `"refused"` as well as `"failed"`, and
  `result`'s absence covers a third case. A reader of version `3` who treated
  every record carrying `error` as one that ended the node is wrong under `4`;
  `outcome` is what tells them apart;
* **`ToolCallRecord.target`'s row reads differently in its last sentence.** The
  one call with no `target` — a name the agent does not offer — is still the one
  call with no `target`, and what happens to it is no longer "ends the node".
  The *presence* rule is unchanged;
* **an instance path is derived from the same count as before.** Grammar §9.4's
  frame counts a flow-tool's *invocations*, as it always did, and a refused call
  invokes nothing — so it spends no ordinal and §10.3's "changing the derivation
  of an idempotency key" is not engaged. Every path a version `3` composition
  produced is the path it produces now, which is the strong form of the claim
  rather than a compatible reading of a weak one: under `3` a refusal ended the
  node, so no version-`3` run ever had a call standing behind one.

Nothing was removed or renamed, no field was added, and no order changed.

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
  project owns them. There is no envelope on this path and so no `trace_version`
  beside the entries: `src/runtime.ts` exports the constant `TRACE_VERSION`,
  which holds the same number §1's three surfaces publish, and it is what a
  caller that pins a version reads.
* **The trace file's path and name.** `<flow>-<execution id>.json` under the
  project's data directory is where a run puts it, and `trace_path` is how a
  caller learns where; the layout under `.agent-compose/` is the emitted
  project's, described in its own `README.md`.
* **Anything a store answered.** `StoreRecord.answer` is carried verbatim; its
  shape is the store's declared schema (grammar §11.4).
* **What a model sent a tool.** A tool call is recorded — that it was made, to
  what, and what became of it (§7.3) — and the **arguments** are not carried as a
  field, at any of the three tool surfaces. That half is §11's rule read where
  the *caller* is a model rather than the graph.

  The one place an argument value appears is inside a message: a `"refused"`
  call's `error`, which spells the sentence the model itself was handed back so
  that it could call again — under §3's `<error name>:` envelope, which is the
  one thing that sentence gains on its way here (grammar D119, §7.3). That is
  the same excerpt rule the row below states for every other contract failure,
  applied at the surface where the reader of the message is the party that
  composed the value — see §11.1's table, where the two sites are classified
  together.

  What a tool answered divides, and PRD §9.20 is what divides it. A
  flow-as-tool call's result **is** carried, on `ToolCallRecord.result`: the
  resolved question asks the tool-call entry to record the result the model saw,
  and what that result is is the child flow's declared `outputs:` — the
  composition's own data under a schema the composition wrote, which is the same
  standing `StoreRecord.answer` has two bullets below. The other two surfaces
  stay under the rule: a `tool.*`'s answer is an external system's, and a store
  tool's is already in `StoreRecord.answer`, so recording it twice would buy
  nothing.
* **The execution journal.** A compiled project keeps a second record beside
  this one, and the two are not the same artifact: the journal holds every
  effect's full payload — model completions, tool results, what a store
  answered, what a person answered — because a replay has to hand those back
  rather than re-issue them (`docs/durability.md`). It shares this format's
  keying (§8) and nothing else, it is private recovery data with the same
  sensitivity as the project's stores, and no field of this format is derived
  from it or carries any part of it. Everything §11 keeps out stays out; the
  journal is where it goes instead. A **resumed** execution writes a fresh trace
  document of its own, carrying the same `execution_id` and the same
  deterministic identities, in which replayed and live work are deliberately
  not distinguished — this format answers what the *execution* did, and that is
  the same answer whichever process did it (`docs/durability.md` §9).
* **What a human answered.** A `human` node's pause is recorded — that it began,
  how long it had, and how it ended (§3.4) — and the answer itself is not. It is
  the same rule as the one below for a model's completion and is stated
  separately because "never the completion" would not be read as covering it: a
  person's answer is a *result*, and every other node's result is absent from
  this format too. What the answer did to the run is on the entry, in the
  `writes` that name the channels it landed in by name — never their values —
  and the answer's *arrival* is `settledAt`.

  The one exception the row below grants a model's answer has no counterpart
  here, and the reason is worth stating: an answer that fails the contract its
  component declares is reported with an excerpt of the offending value, but a
  resume payload that fails the `human` node's `output:` is refused at the route
  before anything is delivered, does not consume the wait, and so never becomes
  part of a run's record at all (PRD 5.11). The refusal is an HTTP response to
  whoever sent it; the excerpt is in that body and in no trace.
* **Provider transcripts.** A trace records that a call was made and which member
  served it. It never carries a prompt, and it carries no completion as a field:
  §7's model-call record has no field for one. One fragment of a completion does
  reach a **message** field, and it is worth stating because "never the
  completion" would be read as covering it: an answer that failed the contract
  its component declares (PRD 5.2) is reported with an excerpt of the offending
  value at the failing path — at most 120 characters — and so are the arguments a
  model sent to a tool, on `ToolCallRecord.error` where its call was refused
  (§7.3) and on `TraceEntry.error` where the refusals ran the loop out of its
  budget. An answer that parsed is not recorded at all. §11.1's table is where
  the excerpt is classified, as text this process did not compose.

  `ToolCallRecord.result` is not an exception to that and is worth reading
  beside it. What it carries is a subflow's `outputs:`, which is the
  composition's own declared surface — but a composition whose subflow ends at
  an `agent:` node has *filled* that surface from a model's structured answer,
  so a parsed completion can reach this format that way. The difference from the
  sentence above is what the format promises: a completion is never carried
  because a model answered, and this value is carried because the composition
  declared an output and the tool call returned it.

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

**Class 2 — interpolable.** Grammar §4.3's class-2 row in full: an `http:` node's
and `http:` tool binding's `url` and `headers` values; the whole `exec:` surface —
`command`, every `args` entry, `cwd`, and `env` values, on the tool binding and
the inline node alike; a provider's `headers` values and its non-secret keys
(`region`, `location`, `project`, `organization`, `profile`, `api_version`); and
non-secret `storage_backends` and `event_sources` config values. Any of them may
hold a resolved value, and the promise above is over all of them rather than over
the ones a message happens to quote.

Two are what a message does quote, and they are the two an author is likeliest to
point at a secret: `url: "${SIGNED_ENDPOINT}/reports"` and
`command: "${TOOLBIN}/rg"` are compositions the grammar admits, and a failing
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
| `TraceEntry.error` | the failure the node's own activity raised — for an `http:` binding, the rejected response body truncated to 200 characters; for an `exec:` binding, the child's stderr; and on any surface parsed against a declared schema (PRD 5.2), an excerpt of the offending value at the failing path, truncated to 120 characters — a model's own answer, or a decoded `http:`/`exec:` payload that a non-2xx rule accepted and a schema did not. The arguments a **model** sent a tool reach this field one way only, since grammar D119: an agent node that spent `max_tool_iterations` says which refusal it was still holding, and that refusal is the excerpt |
| `DispatchRecord.error` | the same text, raised by one dispatched item (§5). Under `on_item_error: skip` this is the **only** field it reaches: the run survives, so no entry carries an `error` for it — and, on the other carrier, raised inside the subflow a model's tool call ran |
| `ToolCallRecord.error` | the same text, from the tool a model called (§7.3). On a `"failed"` call, the tool's execution failing: a `tool.*`'s refused response or child stderr, a store op's failure, or one raised inside a flow-as-tool call's instance. On a `"refused"` one it is this runtime's own sentence rather than the other side's — the tool, the field and the constraint, with an excerpt of the **arguments** the model sent, at whichever of the three surfaces refused them (grammar D119) |
| `TraceDocument.error` | the same text, when that failure is what stopped the run |
| `Refusal.detail` | what a provider answered: a status and a response body, truncated, or the socket failure that came back instead. Never the request, so the key it was signed with is not in it |

A target that echoes back what it was sent puts that echo in the trace — a 404
body naming the path it did not route, a command that prints its own arguments
on stderr. This format records what it was answered; it does not audit it.

Two fields are the composition's own to fill, and both are carried verbatim:
`StoreRecord.answer` is what a read answered (§6), and `ToolCallRecord.result`
is the `outputs:` a subflow a model called answered with (§7.3). A run that
reads a secret out of a store, or writes one into a flow's declared outputs, has
put it there itself; the trace records both like any other value. Neither is
derived from a resolved `${ENV}` reference by this runtime, which is what §11.1
opens by promising.

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

One more failure is restated and is **not** in this table, and the difference is
worth naming so the table reads as what it is. A child's standard input that
could not be written — the one non-`EPIPE` failure of an `exec:` binding's stdin
pipe — comes out as `` `${TOOLBIN}/rg`'s standard input could not be written
(<code>) ``, in place of the platform's own wording. The platform's message
there carries no resolved value, so §11.1's promise was never at stake; it is
restated for the *second* property below — one composition, one message, whatever
engine ran it — and because "could not be run" would be false of a command that
ran. This table is the leak sites; that one is a wording site.

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
