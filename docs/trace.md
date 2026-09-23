# agent-compose — Trace Format

**Trace version:** `5`
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
12. [The OTLP export](#12-the-otlp-export)

---

## 1. Delivery surfaces

A compiled project delivers a trace on four surfaces. All four carry the same
entries; they differ in what surrounds them, and one of them may re-encode them.

| surface | what carries the trace | version key |
|---|---|---|
| `run --format json` | one JSON object on **stdout**, whose `trace` is the array of entries | `trace_version`, beside `trace` |
| the trace **file** | one JSON object — the whole [envelope](#2-the-envelope) — under the project's data directory | `trace_version`, at the head of the envelope |
| `serve` status | `GET /executions/:id` and the `callback:` webhook body, whose `trace` is the array of entries | `trace_version`, beside `trace` |
| the **trace sink** | one POST per settled execution — a **child execution** a detached `flow.*` dispatch started included, under its own id (§1.4) — to the address `trace_sink:` names (grammar §14.5). Each is the whole envelope, or the OTLP/JSON §12 maps it to | `trace_version`, at the head of the envelope — and, under `format: otlp`, as the `agentcompose.trace_version` attribute of the root span |

The rule that spans them: **on the first three surfaces above, wherever a
`trace` appears, the `trace_version` that describes it appears beside it — and
wherever one is absent, so is the other.** Both halves are load-bearing, because
one of the three carries reports with no trace on them: a `serve` report for a
run that is still going carries neither, and so does one for a run whose failure
carried no trace at all — a request the graph refused before it ran (§1.3). `run
--format json` always carries both, `trace` empty where the run recorded
nothing. In process there is no document to put a version in, so the constant
`TRACE_VERSION` is where an in-process caller of `runFlow` reads the same number
(§11).

The sink is the one surface that does not always carry a `trace` key, and only
because under `format: otlp` it carries no envelope at all — §12 is what it
carries instead, and the version travels as an attribute so that a reader on a
collector can still pin it. Under the default `format: envelope` it is an
envelope like the file's, version included.

### 1.4 The trace sink

`trace_sink:` in a deploy file names one address, and **every execution that
settles under that target** ships its trace there — a `serve` request's, a
recovered execution's, and an `agent-compose run` alike (grammar §14.5, PRD
resolved q50). It is the surface a collector can rely on: the other three are
somebody asking for one trace.

The POST is a **delivery** on the journal's ledger, which is what the guarantees
are: journaled before it is attempted, retried on a bounded schedule, ordered
per execution by `X-AgentCompose-Ordinal`, deduped by `X-AgentCompose-Delivery`,
and signed with the outbound headers grammar §13.3 defines when the sink
declares `auth:` (`docs/durability.md` §3.7). Two consequences a receiver is
entitled to: **an export can arrive twice**, and it is one export — the bytes are
serialized once, at the intent, so a retry is the same delivery rather than a
second view of one run. And a sink that is down costs deliveries a retry and
never an execution: nothing about the export can fail, delay or change a run.

One export per settled execution and no more. An execution the journal already
holds an export for is not exported again — which is what a `serve` that died
between journaling the row and closing the lifecycle row leaves behind. "An
export" is the execution's **own** row on its own ledger; the one other kind of
sink row a journal can hold is the envelope a build before version `5` shipped
onto a *parent's* ledger for a detached delivery (§10.3.4), which is not the
parent's export and does not stand in for it.

**Settled** means the journal's lifecycle row closed, which is the moment the
export is journaled in. Two runs therefore export nothing, and both are runs with
nothing to export: one that ends holding a `human` pause leaves its row *open* on
purpose (`docs/durability.md` §3.6) and has not settled yet, and one that failed
before it was journaled at all — a payload the flow's `inputs:` refused, a
journal that could not be opened — never had a row or a trace.

**Child executions** (PRD resolved q64, q65). A **detached** `flow.*` dispatch
(grammar §8.6 rule 7) starts a **child execution**: an execution of its own,
with an id derived from its parent's and the dispatch's grammar §9.4 key, its
own lifecycle row, journal and recovery (`docs/durability.md` §3.2) — and so its
own export, under that id, by exactly the contract above. There is **one event
class**: every POST is a settled execution's export. What a child's carries
beside the ordinary envelope is its **cause**, as three head fields:

| export of | its head |
|---|---|
| an execution a trigger or a command started | no `detached` key |
| a child execution | `detached: true`, `parent_execution`, `idempotency_key` — and, under `format: otlp`, the same three as attributes of the root span (§12.5) |

Every execution has a cause — a trigger, or a detached dispatch from another
execution's node — and the head is where a child's is written down.
`idempotency_key` is byte for byte the `idempotencyKey` of the parent's stub
`"detached"` record (§5.1), so the join between the two envelopes is string
equality (§8). The parent's account of the dispatch does not move: its map
node's entry carries that stub record and nothing the child did, so the parent's
export is byte for byte what it would have been had the child never run. The
child's envelope is where its entries are, with their model calls, store
records, harness runs and the child's own joined dispatches on them.

* **A child that failed ships too**, `status: "failed"` and `error` on its
  envelope and the aborting entry last (§9) — the debugging story is strongest
  exactly there. One that met a replay **divergence** stays open and ships
  nothing, as every execution does (`docs/durability.md` §7).
* **Nothing about it can delay or fail its parent** (grammar §8.6 rule 7). The
  parent never waits for its child: a parent may settle, and export, before its
  child has run a node, and a child that fails fails on its own.
* **A child never parks.** A detached dispatch that could reach a `human` node
  is refused at build time (Decision D118), so a child's envelope takes
  `status: "completed"` or `"failed"` and never `"interrupted"`.
* **A detached `agent.*` or `tool.*` delivery ships nothing of its own.** It
  runs no node and starts no child; the stub record stays its whole account.
* **The wire does not move.** A child's export is a `settled` row on the
  child's own ledger, and its POST carries `X-AgentCompose-Event: settled` —
  grammar §13.3's vocabulary is unchanged.

**Once per child, and deduped on the pair.** A child's export is an execution's
export, so the once-per-execution rule above is its rule: a child recovered after
it exported is not exported again, whichever process recovers it, and a retried
POST is one delivery under one `X-AgentCompose-Delivery` as ever. A receiver
joining children to parents keys a child on
**`(parent_execution, idempotency_key)`**, and may dedupe on the pair: one
dispatch names one child in every generation (`docs/durability.md` §3.2), so the
pair and the child's `execution_id` are one-to-one — which is grammar §9.4's
receiver-side rule read one more time, and also what folds the envelope a build
before version `5` shipped for the same dispatch (§10.3.4).

**Where children exist.** `detach: true` is legal under `--target local` and
under no other target (grammar §8.6 rule 7, Decision D59), so a child execution
exists exactly where detach does. Under `serve` a child is tracked on the status
route by its own id (§1.3), recovered like any open execution — including one
whose parent settled first — and exported when it settles
(`docs/durability.md` §6.1). Under `agent-compose run` the command reports its
own run first and then **waits for every child execution it started**, children
of children included, shipping each one's export before it exits
(`docs/durability.md` §6.2).

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
  "trace_version": 5,
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

A run that made **no** entries writes no file, and names none. The file an
`agent-compose resume` of a **child execution** writes carries the child's
lineage head (§2), as its export does.

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
  "trace_version": 5,
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

A **child execution** (§1.4) is reported here by its **own** id, exactly as
every execution is — running, settled, or recovered by a later `serve` — and
its `trigger` is its parent's: the trigger's `auth:` is what guards the route,
and whoever may read the execution that dispatched a child may read the child,
while a child reachable with no credential beside a parent that demands one
would be a route around the parent's `auth:`. The report carries no lineage
field; the envelope's head (§2) and the journal's lineage row
(`docs/durability.md` §3.5) are where a child's cause is written.

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
| `trace_version` | integer | always | The format the `entries` are written in. `5` is this document, and a compiled project spells it `TRACE_VERSION` (exported from its `src/runtime.ts`). See [Stability](#10-stability). |
| `detached` | `true` | a child execution's envelope | Marks the envelope of a **child execution** — one a detached `flow.*` dispatch started (§1.4, PRD resolved q65). Never `false`: an execution a trigger or a command started omits the key. Carried by the envelopes a child's settlement writes — its trace-sink export, and the trace file an `agent-compose resume` of it writes; `run --format json` and a `serve` report carry no envelope head and so never do. |
| `parent_execution` | string | a child execution's envelope | The execution whose detached dispatch started this one — its **parent**, and never this envelope's own `execution_id`: a child is an execution of its own. Half of the pair a reader joins the parent's stub record to this envelope on, and the pair a receiver may dedupe on (§1.4); also half of what the child's id is derived from (`docs/durability.md` §3.2). |
| `idempotency_key` | string | a child execution's envelope | The grammar §9.4 key of the dispatch that started this child: byte for byte the `idempotencyKey` of the parent's stub `"detached"` dispatch record (§5.1), so the join between the parent's entry and this envelope is string equality (§8). The other half of the pair (§1.4). |
| `flow` | string | always | The flow that was run, as its typed address (grammar §2.2). On a child execution's envelope, the `flow.*` the dispatch targeted. |
| `execution_id` | string | always | The execution the entries belong to — grammar §4.1's `execution.id`, and the prefix of every idempotency key in the document (grammar §9.4). On a child execution's envelope it is the **child's** own id — derived from `parent_execution` and `idempotency_key` (`docs/durability.md` §3.2) — under which its effects are journaled and every key in its entries is derived. |
| `status` | `"completed"` \| `"failed"` \| `"interrupted"` | always | How the run ended: with an answer, without one, or holding a `human` pause it had no way to answer (grammar §8.7, §9). The third is told apart from the second because the two ask different things of whoever is reading — one is a run to look into, the other a question to answer — and because a reader may not decide it from the message text (§10.1). A child execution's envelope takes the first two only: a detached dispatch that could reach a `human` node is refused at build time (Decision D118), so a child never parks. |
| `error` | string | on `"failed"` and `"interrupted"` | What stopped the run. Present on every document that carries neither answer, because such a run's last entry does not always say: a run stopped by the superstep ceiling has no aborting node to carry one. On `"interrupted"` it names the node that is waiting and where an answer would come from. |
| `entries` | array of [entries](#3-entries) | always, possibly empty | Every entry the run recorded, in the order §3.1 fixes. Empty only on a run that recorded none at all — one that failed before any node produced an entry, which takes a failure the node a run aborts at cannot account for, since that node contributes one (§9). The trace **file** is not written for such a run (§1.2), so an empty array reaches a reader only as `run --format json`'s `trace` or a `serve` report's — or as a child execution's export, which ships whatever its run recorded. |

**A child execution's envelope** is this record with three more keys at its
head, in this order after `trace_version`: `detached`, `parent_execution`,
`idempotency_key` — the child's **cause**, where an execution a trigger started
has none to write. Everything else about it is an execution's envelope, under
the child's own `execution_id` (§1.4). Version `4` carried the same three keys on
a different document — the envelope a detached `flow.*` *delivery* shipped of
its own, whose `execution_id` was its **parent's** — and PRD resolved q65 made
the delivery an execution, which changed what `execution_id` and
`parent_execution` mean on it. That is §10.3's "changing the meaning of a
field's value", and it is what version `5` is (§10.3.4).

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
| `harness` | array of [harness runs](#76-a-harness-run) | `coder` nodes that started a run | Every coding-harness run this node execution made (grammar §8.9, PRD resolved q57). A coder node makes **one run per attempt**, so what an entry carries across a `retry:` ladder is one record per attempt, in the order they were made — the same set rule `models` is under, and for the same reason: a run that failed on the first attempt really ran. Never empty: a node that started none carries no key. A node whose *input* could not be built is one such entry — `attempts: 0`, exactly as that leaves `inner` off a `flow:` node — but it is not the only one: an attempt that failed *before* the run began starts nothing either, which is what a bound harness this process has no driver for, an `${ENV}` the environment did not hold, and a `workspace:` the node's input scope could not answer — an expression that read an absent value, evaluated to something that is not a string, or resolved to an empty path — all produce. Those report `attempts` of one or more with no record to show for them. **The absence is a statement about runs started, not about attempts made**; read `attempts` for that. A node that is not a `coder` node never carries it. See §7.6. |
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
| `pausedAt` | string | always | When the wait began, as an ISO 8601 instant. It is the runtime's own clock reading rather than anything derived from the trace's step numbers, because a wait is the one thing in a run whose duration is not the graph's to decide. On a pause a **worker** opened it is the hub's reading of when it planted the wait, not the worker's of when it reached the node: the process holding a question is the one that dates it, so that this pair and the next are one clock's (`docs/distributed.md` §3.4). |
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
makes reliable. The instance's trace is not lost, only elsewhere: a detached
`flow.*` dispatch starts a **child execution**, which exports its own trace
under its own id when it settles (§1.4), headed with an `idempotency_key` equal
to this record's `idempotencyKey`.

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
  So the account of a detached dispatch *on this entry* is exactly this record,
  and §7.2's "every model call that node execution made" is bounded by the same
  join that bounds the outcomes.

  **What a detached `flow.*` did is written somewhere else, and this record
  points at it.** The rationale above is about the parent's entry and nothing
  more: that entry is written when the join finishes, so a fuller one would be
  nondeterministic. A detached `flow.*` dispatch starts a **child execution**
  (§1.4, PRD resolved q65) — an execution with its own id, journal and export —
  and the child's envelope holds its entries in full, a failed child's included,
  headed `detached: true`, `parent_execution` and an `idempotency_key` that is
  **this record's `idempotencyKey`**, so a reader joins the stub to the child's
  account by string equality (§2, §8). Nothing of that envelope reaches this
  entry, and nothing about this record changes because the child exists.

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
entry was written is a matter of scheduling rather than a fact about the run. A
detached `flow.*` dispatch's ops are a **child execution's**, on the entries of
the envelope it exports under its own id (§1.4).

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
| `outputMechanism` | `OutputMechanism` | on an answered call that asked for structured output | Which of the answering wire's two ways of asking for an object under a schema produced this answer: `"native"` — the wire's own structured-output parameter (`output_config`'s `format` on the Messages wire, `response_format` on Chat Completions, `text.format` on Responses), whose object arrives as the assistant's text — or `"forced_tool"` — the synthetic output tool pinned by name, whose object arrives as that call's arguments. See [§7.5](#75-which-mechanism-answered). |
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
scheduling. A detached `flow.*` dispatch's calls are a **child execution's**, on
the entries of the envelope it exports under its own id when it settles (§1.4) —
a detached coder review's harness runs, taped per PRD resolved q57, land there
the same way, and their transcripts on the child's journal record.

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
| `name` | string | always | The tool the model called, spelled as the request offered it — a `tool.*`'s local name, a `flow.*`'s (grammar §5.4), a synthesized store tool's (grammar §11.5), or a built-in's — `bash` and `str_replace_based_edit_tool`, which are the names the provider-defined tool types dictate rather than the definition keys (grammar §5.5, §6.1). |
| `target` | string | when the agent offers a tool of that name | The component behind the name, as a typed address (grammar §2.2). Absent on the one call that has none: a name the agent does not offer, which is a model answering with a tool that was never on the wire. That call is recorded `"refused"` and handed back to the model with the names it does have (grammar D119). |
| `outcome` | `"completed"` \| `"refused"` \| `"failed"` | always | What the loop did with the call. `"completed"` handed the model the tool's result. `"refused"` handed it the **refusal** instead: the tool's declared contract did not admit the call — arguments its schema refuses, on any of the three surfaces, or a name the agent never offered — and grammar D119 makes that a call the model is asked to make again, so the node did not end and records after it exist. `"failed"` is the tool's *execution* failing, which ended the node: the failure left the tool, the node's own `on_error:` decided the run (grammar §9.2), and the model saw nothing back. §5.1 names the one failure no `on_error:` decided: a `human` node inside the flow the call ran, on a run that could not answer it. |
| `instance` | string | flow-as-tool calls that started an instance | The **link**: the subflow instance this call ran, named exactly as the dispatch record carrying that instance's trace names itself in `idempotencyKey`, so the join between the two is string equality (§5, §8). Absent on every call that instantiated nothing — a `tool.*`, a store tool — and on a `"refused"` flow-as-tool call, which is arguments that failed the flow's own `inputs:` before an instance existed. A refused call spends **no** call ordinal (grammar §9.4, D119) — that ordinal counts invocations and this call reached no flow — so the instance path of the call that follows it is the one it would have had with no refusal ahead of it. |
| `result` | any | `"completed"` flow-as-tool calls, with `instance` | **The result the model saw**: the value the loop handed back, which for this tool is the instance's declared `outputs:` (grammar §5.4). PRD §9.20 asks the tool-call entry to record it, and it is the one tool result this format carries — §11 is where the rule it is carved out of is stated, and where the other two tool surfaces are left under it. Its calls are a **subset** of `instance`'s, not the same set: `instance` says an instance ran, `result` says the loop handed that instance's outputs back, so a call carrying `result` carries `instance` and not the other way round. Absent on a `"failed"` call and on a `"refused"` one, where the absence is the record — the first left the tool and the second never entered it, and in neither did the model see a result. A flow-as-tool call whose instance failed is exactly that call with an `instance` and no `result`. |
| `error` | string | `"failed"`, `"refused"` | What went wrong, in §3's `<error name>: <message>` shape. On a `"refused"` call the `<message>` half is **byte for byte the sentence the model was handed back**, and the `<error name>` half — `ToolCallRefused` — is this format's own envelope, which the model's copy does not carry: the two strings differ by that prefix and by nothing else, so a reader joining a record to a provider transcript compares the record's message half, never the whole string. That sentence names the tool, the field and the constraint the way a compiler diagnostic would (PRD G3) and may quote an excerpt of the arguments — see §11. |
| `program` | [built-in program](#74-what-a-built-in-ran) | calls that reached a **built-in** tool | **What the model ran** (§7.4): the `bash` command and its exit status, or the file operation and the path. The one carve-out from §11's rule that this format carries no tool arguments, and PRD resolved q54 ruling c is what carves it: for every other binding the program is the composition's and a reader already has it, while a built-in runs what the model wrote at run time. Present on every call that reached one — `"completed"`, `"failed"`, and the `"refused"` calls the *tool* refused (a path that leaves the workspace) — and absent on a built-in call refused before it, which never reached the tool, as well as on every call to any other tool. |

**What is deliberately not here.** The **arguments** the model sent are absent
as a field of their own, and that is §11's rule rather than an omission: this
format carries no provider transcript, and what the call did to the run is
reachable through the instance `instance` links to. `result` is the one thing on
the other side of that rule, and it is here because PRD §9.20 put it here — "a
bare dispatch record alone would leave a tool call whose result came from
nowhere". It is carried for the one tool whose result is the composition's own
declared data; a `tool.*`'s answer and a store tool's stay out, the second of
them because §6 already carries it in `StoreRecord.answer`.

A **built-in tool**'s answer stays out with them: a `bash`'s stdout and a file
view's contents are a tool's answer under §11's rule, so what this format records
of one is the call — `name`, the address it was attached by as the `target`
(`builtin.bash` for a shorthand, `tool.sandbox` for a configured one), the
outcome, the error where there was one, and — this and no more of it — the
**program** the model wrote, which §7.4 specifies. The answer itself is in the
durability journal, which is private recovery data rather than a document a run
hands out (`docs/durability.md` §3.2, §8, PRD resolved q54).

A `"refused"` call's `error` is where an excerpt of those arguments can appear,
and it is not an exception to the rule above but the same one read where the
message goes: the refusal is written **for the model**, which chose the
arguments and is being asked to choose again, so quoting the offending value
back at it costs nothing (grammar D119, §11.1).

### 7.4 What a built-in ran

`BuiltinProgram`, on `ToolCallRecord.program`. The named carve-out §11 gives the
two built-in tools (grammar §5.5, §6.1), and PRD resolved q54 ruling c is the
decision behind it: "the trace includes what the model ran … for these tools the
model's program *is* the interesting record".

The reason it is only these two. Every other implementation binding fixes **what
runs** at build time — an `exec:`'s command, an `http:`'s URL, a `module:`'s file
— and lets the model fill parameters a schema constrains, so a reader who wants
to know what a tool call ran opens the composition. A built-in inverts that: the
model authors the program at run time, so a trace that recorded only "`bash` was
called, and it completed" would say an agent ran *something* on a machine and
nothing whatever about what. The trust level is the one grammar D135 states out
loud, and this is the record that makes it auditable.

| field | type | presence | meaning |
|---|---|---|---|
| `tool` | `"bash"` \| `"files"` | always | Which built-in ran it. It is the binding's `builtin:` keyword rather than the name the model called — `"files"` for the tool the wire calls `str_replace_based_edit_tool` — so a reader joins it to the composition rather than to the transcript. |
| `command` | string | `"bash"` calls | The shell command **as the model wrote it**, capped at 1000 characters. A call that asked only to restart the session records the word `restart`, which is the whole of what such a call is; one that asked for a restart *and* a command records the command, which is what ran. |
| `operation` | `"view"` \| `"create"` \| `"str_replace"` \| `"insert"` | `"files"` calls naming one of them | Which file operation was asked for. Absent where the model named something else, which is a call the tool refused: the record then carries the `path` and the refusal in `error`. |
| `path` | string | `"files"` calls | The path **as the model wrote it**, capped at 1000 characters — never the resolved one, which would carry the directory a `${WORKSPACE}` resolved to on this machine (§11.1). |
| `exitCode` | integer | `"bash"` calls whose command completed | `$?`. Absent where no command completed — a command killed at its bound, a shell that exited under it, a call that only restarted the session — which is the difference between "the command failed" and "the command did not finish" (and, for the restart, never began). |
| `timedOut` | boolean | `"bash"` calls the command bound killed | `true` where the command outran the binding's `timeout:` and its process group was killed. The call is still a `"completed"` one: PRD resolved q54 makes a spent deadline a **tool result** the model is handed rather than a node failure, so the loop went on with what the command had printed by then. |
| `change` | string | `"files"` calls that changed a file | What the edit did, as a sentence this runtime composes: `wrote 412 bytes`, `replaced one occurrence at line 12`, `inserted 3 line(s) after line 40`. The **bound** §11 asks for, and it is a bound of kind rather than of length: an excerpt of what was written would be a file's contents in a trace, so what is carried is the shape of the edit and never its bytes. |

**This is an addition rather than a change.** A new record type reachable from an
existing one, and a new field on §7.3's record, are both compatible under
§10.2 — a reader written against `trace_version` 4 sees a key it does not
recognize and ignores it, which is what §10.1 already requires of it. No version
bump, and nothing that was recorded before is recorded differently.

**What is still not here**, so the carve-out's edge is where this document says
it is: the command's **output**, the file's **contents**, the arguments of any
other tool, and the resolved workspace. `stdout`, `stderr` and a file view are
the tool's *answer* and stay under §11's rule with every other tool's; the
journal holds them, and §11 says why that is a different artifact.

### 7.5 Which mechanism answered

`outputMechanism`, on the §7 record of a **structured-output** call — the pinned
call that ends an agent node, and no other. PRD resolved q53 ruling b is
the decision behind it: how an agent's `output:` contract is asked for on the
wire is handled inside the provider integration and is **never** a consumer
surface — no YAML key, no capability flag, no shipped model table — and the
trace is the one place it surfaces at all.

Every wire this runtime speaks has two ways of asking for an object under a
schema, and the runtime prefers the wire's own native parameter and falls back to
the synthetic output tool when the endpoint refuses the first with a
capability-shaped 400 (or the reverse, for the model generation that has removed
forced tool use). Which one answered is therefore a property of the **endpoint**
rather than of the composition: one spec deployed against a gateway a generation
behind and against the newest model generation answers through different
mechanisms, with the same YAML on both. Without this field a reader comparing two
traces of one spec — or reading the extra round trip a laddered first call cost —
has nothing to read it off.

The field is on the calls that asked for an object and on nothing else. A tool
loop's calls pin nothing, so there is no mechanism to name, and their records
carry no key at all. A call **replayed from a journal** carries what that journal
recorded, so one written before this field existed carries none — the same
compatible reading `docs/durability.md` §11.2 gives every field added to a record
type.

It reaches a **collector** too, as `agentcompose.model.output_mechanism` on the
model call's span (§12.5). The comparison the field exists for is between two
deployments, and a collector is where an operator has both; an export that
carried every other field of the record and not this one would drop the fact on
the one delivery surface built for reading it.

**This is an addition rather than a change**, under §10.2's first bullet: a field
added to an existing record type. A reader written against `trace_version` 4 sees
a key it does not recognize and ignores it, which is what §10.1 already requires
of it. The span attribute is the same addition on the other surface, and §12.7
says so from the reader's side: a reader MUST NOT rely on the absence of an
attribute the document does not name, so a collector meeting this one for the
first time is meeting a case it was already told to expect. No version bump,
nothing recorded before is recorded differently, and §11's exclusions are
untouched — the mechanism is a fact about the *request's* shape, not about the
prompt, the answer, or anything a provider was told.

### 7.6 A harness run

`HarnessRecord`, on `TraceEntry.harness`. A `coder:` node (grammar §8.9) runs a
whole coding-agent harness as one node, and this is what that run leaves behind.

**A record type of its own rather than a model call with more fields on it**, and
that is half the reason it exists: [§7.3](#73-tool-calls)'s claim that an agent's
tool loop is *the one place* a compiled graph does work a model asked for
survives untouched, because a harness run is not an agent's tool loop and does
not pretend to be one. The other half is that the two harnesses report
differently — one an estimated cost in money, the other tokens per turn — and one
shape should not lie about which.

**The envelope carries top-level turns and tool events only.** A harness that
runs subagents of its own yields their turns and their tool calls too, and those
stay in the durability journal's private payload with every other tool's answer
(`docs/durability.md` §3.9, §8, PRD resolved q50). What a reader gets here is the
shape of the run, not its transcript.

**Which mechanism asked for the answer is the harness's own**, and that is why
[§7.5](#75-which-mechanism-answered)'s field is not repeated here. PRD resolved
q57 ruling d puts a coder node's structured output on a third mechanism family —
a JSON-Schema output format on `cc`, an output schema on `codex` — on resolved
q53's terms, and `harness` and `sdk` below name that mechanism between them: a
harness's is fixed by the SDK release this compiler pins, and grammar §8.9
states the mapping. What does not transfer is §7.5's own argument for carrying
the field: two deployments of one composition reach different *endpoints* and so
answer through different wire mechanisms, and there is no such variation inside
a run.

**A run a resume replayed reads exactly like one this process performed.** That
is `docs/durability.md` §9's decision, taken for every record type at once
rather than an omission here: a resumed generation writes a fresh trace document
whole, answering what the *execution* did rather than what this process did, and
a reader who needs the other question reads the journal. The record is journaled
(`docs/durability.md` §3.9) so that the resumed document can carry it — every
attempt's run, the ones that failed included — not so that the document can mark
it.

| field | type | presence | meaning |
|---|---|---|---|
| `harness` | `HarnessName` — `"cc"` \| `"codex"` | always | Which harness ran it: the `harness:` keyword the composition wrote (grammar §8.9). A closed vocabulary, and the two reserved names of grammar §15 are **not** members — `validate` refuses a composition that binds one, so no run under one exists to record. |
| `sdk` | string | always | The SDK package this compiler release reached it through and the version it pinned, as `<package>@<version>`. It is what joins a trace to the `package.json` of the project that produced it. |
| `model` | string | always | The `model.*` the composition named (grammar §12.2). |
| `modelId` | string | always | The provider-native model id that address resolved to, which is what the harness was actually handed (grammar §8.9, Decision D141). |
| `workspace` | string | always | **The directory this run was contained by, resolved** (grammar §8.9, Decision D147, PRD resolved q61). A `coder:` node's `workspace:` is an expression evaluated at each dispatch, so the composition's own text no longer answers which directory a run held — and for a `map` over a coder node, where the whole point is that every dispatch holds a different one, that is the first question a reader of this record has. A `workspace: fresh` run reads the directory the runtime provisioned for it, under `.agent-compose/workspaces/<execution id>/<instance path>/`. This is the **one** field of this format derived from a resolved value rather than from what the composition wrote; [§11.1](#111-secrets) names it as the exception it is. |
| `outcome` | `"completed"` \| `"failed"` | always | Whether the run produced an answer this node went on with. `"completed"` means the answer also passed the node's `output:` gate; a run whose answer the gate refused is `"failed"`, because what the node got was not an answer it could use. There is no third member: a harness's own refusals are inside its loop and are `toolCalls` entries, not outcomes of the run. |
| `turns` | array of [turns](#761-a-turn), possibly empty | always | The run's **top-level** turns, in the order the harness took them. Empty on a run that failed before its first turn — a harness that could not start. |
| `toolCalls` | array of [tool events](#763-a-tool-event) | when the run made any | Its top-level tool events, in the order it made them. Never empty: a run that called nothing carries no key. |
| `cost` | [a rollup](#764-what-a-run-cost) | always | What the run cost, as far as this harness reports it. |
| `extra` | object | when this harness reports something the other has no shape for | Harness-native extras, read **under `harness`**: its keys are that harness's own vocabulary and this format fixes none of them. It is the deliberate escape hatch PRD resolved q57 ruling a asks for — the alternative was a fixed row that would have to invent a value for whichever harness did not supply one. A reader that does not know a key ignores it, which is what §10.1 already requires. |
| `error` | string | `"failed"` | What ended the run, in §3's `<error name>: <message>` shape: the harness reported a fatal error, it produced no structured output at all, or its answer failed the node's `output:` gate. Written for a person — §10.1 makes the text something a reader must not parse. |

#### 7.6.1 A turn

`HarnessTurn`, on `HarnessRecord.turns`.

| field | type | presence | meaning |
|---|---|---|---|
| `index` | integer | always | Its ordinal in the run, counting from `0`. |
| `usage` | [usage](#762-what-a-turn-used) | when the harness reports usage per turn | What this turn used. A harness that reports usage for the **run** rather than per turn carries no key here, and its totals are in `cost` instead. |

#### 7.6.2 What a turn used

`HarnessUsage`, on `HarnessTurn.usage`.

Every field is optional, and that is the shape rather than an oversight: the two
harnesses count different things, and a row that promised all four would have to
report zero for a counter its harness never kept.

| field | type | presence | meaning |
|---|---|---|---|
| `inputTokens` | integer | when the harness reports it | Input tokens this turn consumed. |
| `cachedInputTokens` | integer | when the harness reports it | How many of those were served from its prompt cache. |
| `outputTokens` | integer | when the harness reports it | Output tokens this turn produced. |
| `reasoningTokens` | integer | when the harness reports it | How many of the output tokens were reasoning. |

#### 7.6.3 A tool event

`HarnessToolCall`, on `HarnessRecord.toolCalls`. [§7.3](#73-tool-calls)'s record,
narrowed to what a harness reports.

The **same three-member vocabulary**, meaning the same three things. What is not
here is §7.3's other four fields, and each absence is [§11](#11-what-is-not-part-of-this-format)'s
rule rather than an omission: no `target`, because the tool is the harness's
rather than a component of this composition; no `instance`, because nothing a
harness calls is a flow of this graph; no `result`, because a tool's answer is
the one thing §11 keeps out at every surface; and no `program`, because
[§7.4](#74-what-a-built-in-ran)'s carve-out is for the two built-ins *this*
runtime implements, whose bounds this compiler states — a harness's commands are
in the journal payload with the rest of its stream.

| field | type | presence | meaning |
|---|---|---|---|
| `name` | string | always | The tool the harness called, spelled as the harness spells it. It is that vendor's vocabulary, not this grammar's, so a reader joins it to the harness's own documentation rather than to the composition. |
| `outcome` | `"completed"` \| `"refused"` \| `"failed"` | always | What the harness's loop did with the call. `"completed"` handed its model the tool's result; `"refused"` is the harness's own permission surface declining the call, which is what a node's `allow_tools:` produces on the harness that enforces it in-loop (grammar §8.9); `"failed"` is the call's execution failing. A refusal did **not** end the run — the harness's loop went on, exactly as an agent's does after a refusal (§7.3). |
| `error` | string | `"refused"`, `"failed"` | What went wrong, written for a person. On a `"refused"` call it is the sentence the harness's model was handed back. |

#### 7.6.4 What a run cost

`HarnessCost`, on `HarnessRecord.cost`.

A rollup with optional members rather than a fixed row, because the harnesses
report differently and PRD resolved q57 ruling a asks the record not to lie about
it: one reports an estimated total in money and run-level token usage, the other
reports tokens per turn and no money at all.

| field | type | presence | meaning |
|---|---|---|---|
| `turns` | integer | always | How many top-level turns the run took — `turns.length`, carried here so a reader summing cost across a trace need not walk the array. |
| `inputTokens` | integer | when the harness reports tokens | Input tokens over the whole run. |
| `outputTokens` | integer | when the harness reports tokens | Output tokens over the whole run. |
| `usd` | number | when the harness estimates a cost | What the harness itself estimated the run cost, in US dollars. An **estimate the harness made**, not a billing statement and not a figure this runtime computed. |

**This is an addition rather than a change.** A new record type reachable from an
existing one, and a new field on §3's entry, are both compatible under
[§10.2](#102-what-is-a-compatible-change) — a reader written against the current
`trace_version` sees a key it does not recognize and ignores it, which is what
§10.1 already requires of it. No version bump, and nothing that was recorded
before is recorded differently: a composition with no `coder:` node produces
byte-identical traces.

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

**The key has more readers, at the head of an envelope rather than inside one**
(PRD resolved q64, q65). A detached `flow.*` dispatch starts a child execution,
and three things read its key:

* **the child's identity.** The child's execution id is derived from its
  parent's id and this key (`docs/durability.md` §3.2), so the key's two
  properties below are what make one dispatch name one child in every
  generation — a recovered parent that re-issues the dispatch, or a map node's
  own `retry:` that re-issues it at the same path, derives the child it already
  started rather than a second one;
* **the join.** The child's envelope carries `idempotency_key` beside
  `parent_execution` (§2): the key of the stub `"detached"` record the parent's
  entry holds, **repeated rather than derived afresh**, so it names no instance
  path the parent's trace does not already carry and grammar §9.4's list of
  carriers is unchanged — the key gains a reader, not a carrier. The parent's
  stub record and the child's envelope are the same dispatch exactly when the
  two strings are equal;
* **the dedupe.** A receiver that keys a child on
  `(parent_execution, idempotency_key)` folds every account of one dispatch
  into one (§1.4).

Every key *inside* a child's envelope — its entries' store writes and dispatch
records — is derived from the child's own id, beginning with its
`execution_id`, because a child is an execution of its own.

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
  `condition`, a model call's `outputMechanism`, and an edge decision's
  `reason` — the last being a closed
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

### 10.3.4 What version `5` changed

A detached `flow.*` dispatch now starts a **child execution** (PRD resolved
q65): an execution with its own id, journal, recovery and export, where version
`4` ran the same work as a *delivery* under its parent's id and shipped an
envelope of its own for it — the trace sink's second event class, which PRD
resolved q64 had added one release before. The ruling collapsed that class into
the first, and it reaches a reader of version `4` through the same three head
fields q64 defined, in the one way §10.3 does not forgive:

* **`execution_id` changed meaning on the envelope that carries `detached`.**
  Under `4` it was the **parent's** id — the delivery ran under it — and it is
  now the **child's** own, derived from the parent's id and the dispatch's key
  (`docs/durability.md` §3.2). A reader of `4` that filed a `detached` envelope
  under its `execution_id` filed it under the parent; the same reader now files
  it under the child, which is the correct place and a different one. That is
  "changing the meaning of a field's value", and it is the bump;
* **`parent_execution` changed with it.** Under `4` it was equal to
  `execution_id` on the same envelope, by construction; under `5` it never is.
  Its meaning — the execution whose dispatch this was — is unchanged, and it is
  the half of the pair that says so;
* **the envelope moved ledgers, and its entries' keys moved with it.** It rides
  the child's own ledger rather than its parent's (`docs/durability.md` §3.7),
  and every idempotency key inside it — derived from the execution id, as every
  key is (§8) — now begins with the child's id rather than with the parent's
  dispatch path. §10.3 bumps for "changing the derivation of an idempotency key,
  or where instance paths appear", and this is both, on this one envelope;
* **the OTLP root is keyed by the child's id.** Under `4` the root of a
  detached export was keyed by the envelope's `idempotency_key`, to keep it off
  the parent root's span id; a child's id is its own, so the ordinary rule
  applies and the special one is gone (§12.2). The trace and the root's parent
  are what they were: the trace of the execution at the head of the lineage —
  under `4` every nested delivery ran under that head's id, so it is the trace
  a grandchild's export always landed in — under the parent's root span;
* **a child that is still running when its parent settles is recovered, and
  exported once.** Under `4` a delivery still in flight when its parent's row
  closed was lost, and a recovered delivery re-shipped under a new delivery id;
  under `5` a child is an open execution that `serve` resumes, and its export is
  guarded once per execution like any other (§1.4).

What did **not** move: the three head fields' names, types, order and presence
rule (on the envelope of a child, and of nothing else); the parent's envelope,
byte for byte, with its stub `"detached"` record (§5.1); the wire, one event
under `X-AgentCompose-Event: settled`; and every envelope an execution a trigger
or a command started ships, which changed only its `trace_version`. A journal a
version-`4` build wrote may still hold a detached envelope on a parent's ledger;
this build delivers such a row as it was written — its `trace_version` says `4`
— and never counts it as the parent's export (`docs/durability.md` §11.2).

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

  **The named carve-out: a built-in tool's program.** The rule above is about
  what a model *sent a tool*, and it has exactly one exception, which PRD
  resolved q54 ruling c states and §7.4 specifies: a call to `builtin.bash` or
  `builtin.files` carries `ToolCallRecord.program` — the command and its exit
  status, or the file operation and the path, and for an edit a sentence about
  what changed. Every other binding fixes what runs at build time, so a reader
  who wants the program opens the composition; a built-in has the **model author
  it at run time**, and a format that kept it out would record that an agent ran
  something on a machine and nothing about what. It is an exception in one
  direction only: the built-ins' *answers* — a command's stdout and stderr, a
  file's contents, an edit's bytes — stay out with every other tool's, and the
  bound on what `program` carries is written into §7.4 rather than left to
  whatever a model happened to write.
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

**No resolved `${ENV}` value appears in this format, with one named exception
below.** Grammar §4.3 classifies every string surface of a composition, and the
two classes that can hold one are kept out for different reasons.

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
answered** — and, for the two built-in tools, what the **model** asked them to
run. A reader should treat all of it as untrusted:

| field | what it can carry from outside |
|---|---|
| `TraceEntry.error` | the failure the node's own activity raised — for an `http:` binding, the rejected response body truncated to 200 characters; for an `exec:` binding, the child's stderr; and on any surface parsed against a declared schema (PRD 5.2), an excerpt of the offending value at the failing path, truncated to 120 characters — a model's own answer, or a decoded `http:`/`exec:` payload that a non-2xx rule accepted and a schema did not. The arguments a **model** sent a tool reach this field one way only, since grammar D119: an agent node that spent `max_tool_iterations` says which refusal it was still holding, and that refusal is the excerpt |
| `DispatchRecord.error` | the same text, raised by one dispatched item (§5). Under `on_item_error: skip` this is the **only** field it reaches: the run survives, so no entry carries an `error` for it — and, on the other carrier, raised inside the subflow a model's tool call ran |
| `ToolCallRecord.error` | the same text, from the tool a model called (§7.3). On a `"failed"` call, the tool's execution failing: a `tool.*`'s refused response or child stderr, a store op's failure, or one raised inside a flow-as-tool call's instance. On a `"refused"` one it is this runtime's own sentence rather than the other side's — the tool, the field and the constraint, with an excerpt of the **arguments** the model sent, at whichever of the three surfaces refused them (grammar D119) |
| `HarnessRecord.error` | what ended a coding-harness run (§7.6): the harness's own fatal-error text, or — when the run's answer failed the node's `output:` gate — this runtime's schema refusal with an excerpt of the offending value, under the same 120-character cap every other parse refusal is under. The harness's text is the *vendor's* rather than this process's, and a harness that quotes what a command printed puts that echo here |
| `HarnessToolCall.error` | what one tool event of a harness run reported (§7.6.3). On a `"failed"` event it is the harness's own account of the failure; on a `"refused"` one it is the sentence the harness's permission surface handed its model, which for a node's `allow_tools:` is composed by this runtime and names the list |
| `TraceDocument.error` | the same text, when that failure is what stopped the run |
| `Refusal.detail` | what a provider answered: a status and a response body, truncated, or the socket failure that came back instead. Never the request, so the key it was signed with is not in it |
| `BuiltinProgram.command`, `BuiltinProgram.path` | the **program the model wrote** for a built-in tool, carried verbatim and capped at 1000 characters each: the `bash` command's text, and the `files` path as the model asked for it (§7.4). Not what a system answered but what the model chose, so it is the one text here that is adversarial by construction wherever a model read something it should not have — a `command` is shell source and a `path` is arbitrary bytes, and neither is a name this runtime composed. `BuiltinProgram.change` beside them is this runtime's own sentence about an edit (`replaced one occurrence at line 12`) and carries nothing of the file |

A target that echoes back what it was sent puts that echo in the trace — a 404
body naming the path it did not route, a command that prints its own arguments
on stderr. This format records what it was answered; it does not audit it. The
built-in row is the same posture read one step earlier: it records what an agent
was told to run, and does not vouch for it.

Two fields are the composition's own to fill, and both are carried verbatim:
`StoreRecord.answer` is what a read answered (§6), and `ToolCallRecord.result`
is the `outputs:` a subflow a model called answered with (§7.3). A run that
reads a secret out of a store, or writes one into a flow's declared outputs, has
put it there itself; the trace records both like any other value. Neither is
derived from a resolved `${ENV}` reference by this runtime, which is what §11.1
opens by promising. `ToolCallRecord.program` is a third field carried verbatim
and is in the table above rather than here, because the party that fills it is
the **model** rather than the composition — the reason it is untrusted text and
these two are not.

**The exception, named rather than left to be found.**
[`HarnessRecord.workspace`](#76-a-harness-run) carries a **resolved** path —
the one field of this format that does. PRD resolved q61 made a coder node's
`workspace:` an expression evaluated at each dispatch (grammar §8.9, Decision
D147), and three things follow that make this the right field to bend the
promise for and the only one:

* **the composition's text stopped answering the question.** Before q61 a
  reader with the spec in front of them knew which directory a run held; after
  it, a map over a coder node holds a different one per dispatch, and which
  directory a harness was contained by is the first thing a reader of a coder
  run needs. `asWritten` would print the expression, which is in the journal's
  replay identity already and answers something else;
* **it is a containment bound, not a connection.** What §11.1 keeps out is where
  traffic goes and how it authenticates — the class-1 fields, whose whole value
  is one reference. A workspace is a directory on the machine that ran the
  graph; an `${ENV}` inside the expression that names it is a machine root, not
  a credential, and the secret-bearing fields of grammar §4.3's class-1 table
  stay out of this format entirely;
* **nothing else moved.** The journal's effect request still records the
  workspace **as written** (`docs/durability.md` §3.9), the two restated
  failures of §11.2 still quote the expression rather than its answer, and every
  other field of this record is the composition's own text.

A reader who ships traces somewhere a directory layout should not go has one
field to redact, and it is named here.

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

---

## 12. The OTLP export

`trace_sink: { url: …, format: otlp }` ships the same settled trace as
OTLP/JSON spans instead of as the envelope (grammar §14.5, PRD resolved q51).
This section is normative for that mapping.

**No SDK.** The exporter is hand-written — `src/otlp.ts` in the emitted project,
whose only import is `node:crypto` — and held by a conformance corpus rather than
by a vendored dependency: `crates/compose-core/tests/fixtures/otlp-conformance/`
pairs an envelope and an export context with the exact bytes this mapping must
produce, and `crates/compose-core/tests/otlp_conformance.rs` runs every fixture
through the emitted module itself.

**Protobuf.** The wire is OTLP/HTTP with `Content-Type: application/json`, which
is the encoding this project emits and the only one it emits. A backend that
speaks protobuf and not JSON is served by **pointing an OpenTelemetry Collector
at this endpoint**: it accepts OTLP/JSON on its `otlphttp` receiver and re-encodes
to whatever the backend wants. That is the answer, and it is deliberate — the
alternative is a protobuf encoder in a project whose whole point is that it has
no telemetry dependency.

**Encoding.** Proto3's JSON mapping, as the OTLP specification applies it. Two
consequences a reader will notice: every 64-bit integer — a timestamp, an
`intValue` attribute — travels as a **string**, and `traceId`/`spanId` travel as
lowercase **hex** rather than base64. Enum fields (`kind`, `status.code`) travel
as numbers.

The body is one `ExportTraceServiceRequest`: a single `resourceSpans` entry, a
single `scopeSpans` entry inside it, and every span of the execution in that one
array, in the walk order §12.1 fixes.

### 12.1 The span tree

| trace record | span |
|---|---|
| the **execution** | the root span. Named for the flow (`flow.review_loop`), and parented by the caller's span where §12.3 gives it one. A **child execution's** root (§1.4) is parented by its **parent** execution's root span instead (§12.2) |
| each **entry**, at every depth | a span under whatever ran it: the root for a top-level entry, the enclosing entry's span for one under `inner`, the dispatch's span for one under a dispatch record. Named for the node id |
| each **dispatch record**, on `dispatches` and on `toolDispatches` alike | a span under its entry's, named for the record's `target` |
| each **model call** | a span under its entry's, named for the `model.*` the agent asked for. `kind` is `3` (client) — the call leaves the process for another service |
| each **harness run** on a `coder:` node (§7.6) | a span under its entry's, named for the harness (`cc`, `codex`). `kind` is `3` (client) for the model call's reason read one loop out: the run *is* somebody else's agent loop, and every model call inside it was made by them. It is a span of its own rather than a model span with more attributes, because a collector that summed the two would be summing calls this graph made with calls it did not |
| each **store record**, and a `human` node's **pause** | a span **event** on the entry's span, not a span |
| each **tool call** on a model call, and each **failover** or **refusal** | a span event on the model call's span |
| each **top-level turn** of a harness run, and each of its **tool events** | a span event on the harness run's span. The depth is the envelope's (§7.6): a harness's subagent transcripts are journal payload and reach no delivery surface |
| a **flow-as-tool** join | a span **link** from the model-call span to the dispatch span that answered it (§12.2) |

Those two rows are the only `kind: 3` spans this exporter emits, and they are the
two things a node does that leave the process. **Every other span is `1`
(internal).**

Nothing else becomes a span. `attempts` is a **count** in this format rather than
a record per attempt (§3), so it travels as the `agentcompose.attempts` attribute
on the span whose record carries it; there is no per-attempt span, because there
is no per-attempt record to make one from. A `coder:` node under `retry:` is the
case a reader will reach for: its `harness` array carries one record per attempt
that started a run (§7.6), so that node's entry span has one child span per
attempt — which is a span per *record*, exactly as this table says, and not a
span per attempt.

### 12.2 Identity: trace ids, span ids and links

Ids are derived, never random and never clocked, so **two exports of one trace
are the same bytes** — which is what makes an at-least-once delivery safe to
retry and what lets the conformance corpus pin exact output.

* **`traceId`** — 16 bytes, 32 lowercase hex: SHA-256 over `agent-compose/trace/v1`,
  a newline, and the execution id, truncated to the first 16 bytes. Replaced
  outright by the caller's trace id where §12.3 applies.
* **`spanId`** — 8 bytes, 16 lowercase hex: SHA-256 over `agent-compose/span/v1`,
  the execution id, the span's **kind word** (`execution`, `entry`, `dispatch`,
  `model`, `harness`) and its **key**, newline-separated, truncated to the first
  8 bytes. The kind word is in the hash so that an entry and a dispatch at one
  instance path cannot collide — and so that a model call and a harness run at
  one position on one entry cannot either.
* The **key** is a *path through the document*, not the instance path: each step
  names the record and appends its **position** in the array that held it, and
  every step is built on its parent's key rather than on the instance path. So an
  entry's key is `<key of whatever ran it>/<node>/<traversal>#<index among its
  siblings>`, a dispatch record's is `<entry key>/<map|tool>/<idempotency
  key>#<index on that carrier>`, a model call's is `<entry key>/model/<index>`,
  a harness run's is `<entry key>/harness/<index>`, and the root's is the
  execution id. The position is load-bearing rather than
  decorative — §8's derivation deliberately gives two attempts of a retried
  `flow:` node the same instance path, and two spans of one id would be one span
  to a collector — and it is **inherited** for the same reason: two same-path
  siblings whose keys differed only at their own step would still hand identical
  keys to everything beneath them.
* The instance path itself is reported rather than hashed: it is the
  `agentcompose.instance_path` attribute of §12.5, carried exactly as the
  envelope wrote it.
* Neither id is ever **all zero**, which the W3C trace context forbids.

**A child execution's export is filed under its parent** (§1.4, PRD resolved
q64, q65). A child is an execution with an id of its own, so every span id its
export derives is its own by the rules above — nothing it exports can take an id
of its parent's. Two things place it:

* its **trace** is the one its parent's export lands in, since a collector files
  a child under the run that dispatched it rather than beside it. A parent that
  is itself a child landed in *its* parent's, so down any chain of children that
  is one trace: the one the id of the execution at the **head** of the lineage —
  the one a trigger or a command started — derives, or the caller's where a
  valid `traceparent` started that head (§12.3). A grandchild's trace is
  therefore its grandparent's, never the one its own parent's id would derive,
  which is a trace nothing else is exported into. The envelope names only the
  immediate parent, so the exporter is told the head beside it, walked up the
  lifecycle rows (`docs/durability.md` §3.5);
* its root's **parent is the parent execution's root span**, whose id is a
  function of the parent's execution id alone (the root key above) and so is
  known without the parent's export in hand. Not a caller's span: the caller
  started the parent, and the parent's dispatch started the child.

A **link** is how a flow-as-tool call reaches the instance that answered it. Its
`ToolCallRecord.instance` is a dispatch record's `idempotencyKey` (§7.3), so the
link points at the span of the **first** record on that entry's `toolDispatches`
carrying that key — first, because §8's map-item retry can file one key twice —
and carries `agentcompose.instance_path` as its own attribute. A call whose
record is not on the entry contributes no link.

### 12.3 An inbound `traceparent`

An `http` trigger's request may carry a W3C `traceparent` header. Where it is
**syntactically valid**, the export adopts it: the root span takes the caller's
`trace-id` as its own `traceId`, and the caller's `parent-id` as its
`parentSpanId`. An embedded graph therefore appears inside its caller's trace
rather than beside it.

Version `00` is read as the specification writes it, and a later version is read
for its first four fields — the forward compatibility the specification asks of a
parser. A header that is **not** valid is **ignored silently**, which is the W3C
behaviour: the execution costs nothing for a caller's malformed header, and the
export gets a trace id of its own. A header is refused when

* there is none;
* it has fewer than four `-`-separated fields;
* its version is not two lowercase hex digits, or is the reserved `ff`;
* its version is `00` and it does not have exactly four fields;
* its `trace-id` is not 32 lowercase hex characters, or is all zero;
* its `parent-id` is not 16 lowercase hex characters, or is all zero;
* its `trace-flags` is not two lowercase hex digits.

Surrounding whitespace is trimmed rather than refused, and `trace-flags` is
carried as written rather than interpreted: an unsampled caller still names the
trace this export joins. Each of these arms has a case in
`crates/compose-core/tests/fixtures/traceparent-conformance.json`, run through the
emitted parser — an ignored header and a refused one produce the same export, so
nothing downstream could tell a lapsed guard from a working one.

The header is **not a field of this format.** It is a property of the request
that started the execution rather than of the run the envelope records, so it
travels on the journal's lifecycle row beside the `callback:` URL that came from
the same place (`docs/durability.md` §3.5) and reaches the exporter as part of
its context. That is deliberate: putting it in the envelope would publish it on
three surfaces this feature says nothing about and put every later release under
§10.3's presence discipline for it. `trace_version` is untouched by this
section, and §10.2's compatible-change list is untouched with it.

### 12.4 Timestamps and status

**This format records no per-node timing**, and the export does not invent any. A
trace entry carries a superstep number and a traversal ordinal, not a clock. So:

* the **root span** covers the execution's window — its journaled `startedAt`
  (`docs/durability.md` §3.5) to the instant the export was journaled. A child
  execution's is its own window, from its own row, like any execution's;
* **every other span** covers the same window, except an entry whose `human`
  pause gives it one of its own: that span runs from `pausedAt` to `settledAt`,
  or to the execution's end where nothing settled it (§3.4);
* a span **event** is stamped at its span's start, except the two a pause
  contributes, which carry the pause's own instants.

The tree is therefore a **structure, not a latency waterfall**, and this
document says so rather than papering over it: a collector rendering invented
durations as measurements is worse than one rendering honest structure. Adding
per-entry timing to this format is a change to §3, not to this section.

Every instant above is written as nanoseconds since the epoch, in a JSON string,
because proto3's JSON mapping writes every 64-bit integer as one. An instant the
exporter **cannot read** — one this runtime never writes, since the envelope's
instants are its own `toISOString()` — falls back to a point of the execution's
window already in hand rather than to the epoch: a span whose edge is
approximate is still a reading, and a wait rendered in 1970 is not. A pause's two
events fall back with the edges they sit on, so the placement above holds either
way.

`status.code` is `1` (ok), `2` (error) or `0` (unset):

| span | ok | error | unset |
|---|---|---|---|
| root | `status: "completed"` | `"failed"`, with the envelope's `error` as the status message | `"interrupted"` — a run holding a question is not an outcome, and `agentcompose.status` is where it is said |
| entry | `outcome: "completed"` | `"failed"`, with the entry's `error` as the message | `"skipped"` — its `error` says what was absorbed, and it is on the attributes |
| dispatch | `outcome: "completed"` | `"failed"`, with the record's `error` as the message | `"skipped"` and `"detached"` |
| model call | any call a member answered | a call `refused` ended, with `<member>: <detail>` as the message | — |
| harness run | `outcome: "completed"` | `"failed"`, with the record's `error` as the message where it has one | — |

### 12.5 Attributes

Every attribute this export sets is under the `agentcompose.` prefix, except the
resource attributes of §12.6. The values come from the fields §2 to §7 specify
and add nothing to them.

This table is the whole set, and that is checked rather than asserted:
`crates/compose-core/src/codegen/otlp.rs`'s
`the_span_attributes_are_the_documented_ones` reads every `agentcompose.` key the
emitted module sets and every one this table names, and fails on either
direction — an attribute a collector receives that no reader was told to expect,
or a row promising one nothing sets.

| attribute | on | from |
|---|---|---|
| `agentcompose.execution.id` | root | the envelope's `execution_id` |
| `agentcompose.flow` | root, entry | the envelope's `flow`; an entry's own `flow` |
| `agentcompose.status` | root | the envelope's `status` |
| `agentcompose.trace_version` | root | `TRACE_VERSION` — how a reader on a collector pins the format (§1) |
| `agentcompose.detached`, `agentcompose.parent_execution`, `agentcompose.idempotency_key` | root of a child execution's export | the envelope's lineage head — `detached` as a boolean, `parent_execution` and `idempotency_key` as strings (§2). How a collector tells a child execution from one a trigger started, and the pair it joins the child to its parent's stub record on (§1.4); on the root of no other export |
| `agentcompose.error` | root, entry, dispatch, tool-call event | that record's `error` |
| `agentcompose.instance_path` | entry, dispatch, store event, tool-call event, link | §8's path — an entry's derived, a dispatch's read off its `idempotencyKey` |
| `agentcompose.node`, `agentcompose.step`, `agentcompose.traversal`, `agentcompose.outcome`, `agentcompose.attempts` | entry | the entry's own fields |
| `agentcompose.writes` | entry | the entry's `writes`, as a string array |
| `agentcompose.fallback` | entry | the entry's `fallback` |
| `agentcompose.routing` | entry | the **whole** routing decision (§4), serialized as one JSON string. An OTLP attribute is a scalar or an array of scalars and a routing decision is neither, and PRD 5.3 makes routing data — so it travels losslessly rather than partially |
| `agentcompose.routing.targets` | entry | the same decision's `targets`, repeated as a string array, because "which way did it go" is the question a collector filters on |
| `agentcompose.dispatch.carrier` | dispatch | `map` for a record on `dispatches`, `tool` for one on `toolDispatches` |
| `agentcompose.dispatch.index`, `agentcompose.dispatch.target`, `agentcompose.dispatch.outcome`, `agentcompose.dispatch.route`, `agentcompose.dispatch.variant`, `agentcompose.attempts` | dispatch | the record's own fields |
| `agentcompose.model` | model call, harness run | the `model.*` the agent asked for; on a harness run, the one its adapter mapped down (§7.6) |
| `agentcompose.model.served_by`, `agentcompose.model.fallback`, `agentcompose.model.failovers` | model call | which member answered, its ordinal in the route, and how many refused on the way |
| `agentcompose.model.output_mechanism` | model call | §7.5's `outputMechanism` — which of the wire's two ways of asking for an object answered. On the one call per agent node that asked for one, and on no other, so most model spans carry no such attribute |
| `agentcompose.model.member`, `agentcompose.model.condition`, `agentcompose.model.detail` | failover and refusal events | the refusing member, the `route_on:` condition, and what it said |
| `agentcompose.tool.name`, `agentcompose.tool.target`, `agentcompose.tool.outcome` | tool-call event | the call's own fields. A **harness** run's tool events are the same three attributes, less `target`: the tool is the harness's rather than a component of this composition (§7.6.3) |
| `agentcompose.harness` | harness run | which harness ran it (§7.6) |
| `agentcompose.harness.sdk`, `agentcompose.harness.model_id`, `agentcompose.harness.outcome` | harness run | the record's own fields — the SDK and version this release pinned, the provider-native id the harness was handed, and how the run ended |
| `agentcompose.harness.turns`, `agentcompose.harness.input_tokens`, `agentcompose.harness.output_tokens`, `agentcompose.harness.cost_usd` | harness run | the run's cost rollup (§7.6.4). The token attributes are present only where that harness reports tokens and the money one only where it estimates a cost, which is the rollup's own presence rule read onto the span |
| `agentcompose.harness.turn`, `agentcompose.harness.input_tokens`, `agentcompose.harness.cached_input_tokens`, `agentcompose.harness.output_tokens`, `agentcompose.harness.reasoning_tokens` | turn event | the turn's ordinal, and whatever usage that harness reported for it (§7.6.2) |
| `agentcompose.store`, `agentcompose.store.op`, `agentcompose.store.effect`, `agentcompose.store.via`, `agentcompose.store.scope`, `agentcompose.store.key`, `agentcompose.store.deduped` | store event | the store record's own fields |
| `agentcompose.human.expires_at`, `agentcompose.human.settled` | the two pause events | the pause's budget and how it ended |

**What is deliberately not exported.** §11's exclusions hold automatically —
the mapping's input is the envelope, which never held them — and the mapping adds
no attribute that reaches around it. A harness run's **subagent transcripts** are
the sharpest case and cost this mapping nothing: they are journal payload and
were never in the envelope to export (§7.6, `docs/durability.md` §3.9). Two
fields the envelope *does* carry are
still left out, and the reason is the same one §11 gives: `StoreRecord.answer`
and `ToolCallRecord.result` are payloads, and shipping them to a third-party
collector is a wider disclosure than a trace file read by the deployment's own
operator. A reader who wants them reads the trace.

### 12.6 Resource attributes

Stable, and this table is the promise. PRD resolved q51's second amendment holds
the metrics door open by requiring them: a later, standalone metrics exporter
that resources itself this way correlates with these spans on any collector
without touching this one. `crates/compose-core/src/codegen/otlp.rs`'s
`the_resource_attributes_are_the_documented_ones` reads the list out of the
emitted module and out of this table, so a rename that touched one and not the
other fails the build.

| attribute | value |
|---|---|
| `service.name` | the flow's typed address (`flow.review_loop`). The flow rather than the project: it is the identity every trace carries, and an operator watching one graph wants one service |
| `service.namespace` | the deploy target the artifact was built for; `local` under the built-in target |
| `service.version` | the artifact hash (`sha256:…`). PRD 5.12 makes the generated tree the deployable and `docs/distributed.md` §4.1 makes its hash the thing two processes agree on, so it is the version of the *service*; the compiler's version is the SDK's, below |
| `deployment.environment.name` | the deploy target, again — under the OpenTelemetry semantic convention a backend groups environments by |
| `telemetry.sdk.name` | `agent-compose` |
| `telemetry.sdk.language` | `nodejs` |
| `telemetry.sdk.version` | the agent-compose release that emitted the exporter |

The instrumentation **scope** is `{ name: "agent-compose", version: <the same
release> }`, on the single `scopeSpans` entry.

### 12.7 What a reader may rely on

Everything §10 says about the trace format, read through this mapping: the export
is a function of the envelope and the context, so a field §10.3 would need a
version bump to move is a field this mapping would need one to move too. Beyond
that, at a given `trace_version` a reader of the export MAY rely on the span
tree of §12.1, the id derivation of §12.2, the status table of §12.4, the
attribute names of §12.5, and the resource attributes of §12.6 — and MUST NOT
rely on the *order* of attributes within a span, or on the absence of an
attribute this document does not name.
