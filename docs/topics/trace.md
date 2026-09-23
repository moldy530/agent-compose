# trace

Every run records what it did: which node ran in which superstep, which edge
fired and the guard values that decided it, what each fan-out dispatched, every
store op, every model call, and every human pause. The format is versioned and
documented, so a consumer can pin `trace_version` and walk it.

This topic is orientation. `docs/trace.md` is normative and is what a reader
pins on.

## Where a trace is delivered

Four surfaces carry the same entries and differ in what surrounds them:

| surface | what carries it |
|---|---|
| `agent-compose run --format json` | one JSON object on **stdout**, whose `trace` is the array of entries |
| the trace **file** | one JSON object — the whole envelope — under the project's data directory |
| `agent-compose serve` status | `GET /executions/:id`, and the `callback:` webhook body |
| the **trace sink** | one POST per settled execution — a child execution a detached `flow.*` dispatch started included — to the address a deploy file's `trace_sink:` names |

On the first three, wherever a `trace` appears the `trace_version` describing it
appears beside it, and wherever one is absent so is the other.

The first three are somebody asking for **one** trace. The fourth is the
deployment saying once where all of them go: `trace_sink:` in a deploy file
(`agent-compose docs targets`), and every execution that settles under that
target ships its trace there — a served request's, a recovered execution's and a
one-shot `run` alike. It is a journaled delivery like a `callback:` webhook:
bounded retry, ordered per execution, signed with the same headers when the sink
declares `auth:`, and **never** able to block or fail the run it describes.

A `map` route with `detach: true` to a `flow.*` starts a **child execution**:
an execution of its own, with an id derived from its parent's and the dispatch's
idempotency key, its own journal, its own recovery — a child still running when
its parent settled is resumed by the next `serve` — and its own export. Its
parent's trace holds only a stub `"detached"` dispatch record for it; the
child's envelope, shipped when it settles, completed or failed, holds its
entries in full, model calls, store ops and harness runs included, headed
`detached: true`, `parent_execution` and an `idempotency_key` equal to the stub
record's `idempotencyKey`, which is how a receiver joins the two — and the pair
a receiver may dedupe on, since one dispatch names one child. Its `execution_id`
is the child's own. Detach is legal only under `--target local`, so a child
exists only where a `deploy/local.yml` allows it. `docs/trace.md` §1.4 is
normative.

Its body is the envelope, or — under `format: otlp` — an OTLP/JSON
`ExportTraceServiceRequest` mapped from that same envelope: the execution as the
root span, every entry a span under whatever ran it, model calls and dispatches
as child spans, routing decisions as attributes, and a flow-as-tool join as a
span link. It is hand-emitted, with no OpenTelemetry dependency in the generated
project, and a backend that speaks only protobuf is served by pointing an
OpenTelemetry Collector at the sink. An inbound W3C `traceparent` on an `http`
trigger's request is honored, so an embedded graph exports into its caller's
trace. `docs/trace.md` §12 is the normative mapping.

Every `run` also writes `.agent-compose/traces/<flow>-<execution id>.json` under
the project's data directory, and names the path on stderr under
`--format human`. `AGENT_COMPOSE_DATA_DIR` moves that directory. A run that made
no entries writes no file.

The two are not the same document. This is `run --format json`'s, whose array of
entries is `trace` and which carries the run's `outputs` and the path of the
file:

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

## The envelope

The **file**'s document, and the one to write a consumer against:
`trace_version`, `flow`, `execution_id`, `status`, `entries` — always — plus
`error` on a run that carried no answer. `status` is `"completed"`, `"failed"`,
or `"interrupted"`; the third is told apart from the second because they ask
different things of whoever is reading. One is a run to look into, the other a
question to answer.

Envelope keys are `snake_case`; an entry's are `camelCase`. The seam is
deliberate: the envelope's are document keys, an entry's are runtime record
keys.

## Entries

**One entry per node execution.** A node that runs twice — a second traversal of
a bounded cycle, a node reached again through a `fallback:` — produces two
entries. A node's own `retry:` attempts do **not**; `attempts` records those.

Always present: `step` (the superstep, from 1), `flow`, `node`, `traversal`,
`outcome`, `attempts`. Then, per case: `writes` (channels this node wrote),
`routing`, `dispatches` (a `map`'s fan-out, one record per source item in index
order), `toolDispatches` (what a *model* dispatched, one per flow-as-tool call),
`inner` (a `flow:` node's subflow trace), `stores`, `models`, `human`, `error`,
`fallback`.

**The trace numbers supersteps from 1, where routing counts them from 0.** The
nodes the taken `start` edges reach are routing's step 0
(`agent-compose docs routing`) and carry `step: 1` here; a trace entry's step is
one more than the step of the topology it belongs to, throughout.

Entries are ordered by `(step, node)` — ascending superstep, then ascending node
id. Node id rather than completion order, so two runs of one composition produce
the entries in the same order. One entry is outside that ordering and is always
last: the entry of the node a **failed** run aborted at. Nested entries have
step numbers of their own instance, starting again at 1.

**An entry's `outcome`** is `"completed"`, `"skipped"` (the activity failed and
`on_error: skip` absorbed it — it carries an `error` too) or `"failed"`. A
failed entry has two shapes told apart by `fallback`: with it, the failure
routed to the named node and the run continues; without it, the failure ended
the run.

Three record types carry an `outcome` and each closes over its own set, so read
the enumeration for the field you are holding: an **entry**'s is the three
above, a **dispatch record**'s adds `"detached"` (a `detach: true` delivery,
resolved at issue — no outcome was ever observed, which is why its `attempts` is
`0`), and a **tool call**'s is `"completed"`, `"refused"` or `"failed"`.
`"refused"` is the one to know when a tool loop misbehaves: the tool's declared
contract did not admit the call — arguments its schema refuses, or a name the
agent never offered — so the model was handed the refusal and asked again, and
the node did not end.

A flow whose `start` edges carry guards gets a synthetic entry node,
`node: "$start"`. The `$` sigil is outside the identifier grammar, so it
collides with nothing you can declare.

**`models`** holds one record per model call: the `model.*` the node named, the
route member that answered (`servedBy`, `fallback`), and every member that
refused on the way (`failovers`). The last call of an agent node — the one that
asks for its `output:` — also carries `outputMechanism`, `"native"` or
`"forced_tool"`: which of the wire's two ways of asking for an object under a
schema the endpoint actually answered. It is not a setting and there is nothing
to configure; `agent-compose docs models` says what the two are and when the
runtime moves between them.

## Routing decisions

`routing` is the record that makes a change reviewable: `edges` (one decision
per outgoing edge, **in declaration order, including the untaken ones**),
`targets` (what was scheduled next), and `counters` (the `max_iterations`
budgets this step spent).

An edge decision carries `to`, `taken`, the `when:` source and the `value` it
answered on a guarded edge, `else: true` on the catch-all, a `budget` on a
budgeted edge whose guard answered true, and sometimes a `reason`. `reason` has
exactly three values — `"unconditional"`, the budget-spent sentence, and
`"a guarded sibling was taken"` — and a decision the vocabulary does not cover
carries none. So a guarded edge that answered false has `value: false`,
`taken: false` and no reason: the guard value is the whole explanation.

**What terminated a cycle** is readable from the same array. A counting bound
that ran out shows the back-edge with `value: true`, `taken: false`, the
budget-spent reason, and a `budget` naming the counter and the declared `max`. A
CEL exit condition that went false shows `value: false`, `taken: false`, and no
reason.

## Nesting

A `flow:` node's instance is under `inner`. A `map`'s items report under their
own dispatch records, never under `inner`. A flow-as-tool call appears **twice**
by role: its instance is a canonical dispatch record on `toolDispatches`, keyed
by its instance path, and the tool-call entry inside the agent's model call
records the call, the result the model saw, and that instance path as a **link**.
Both invariants survive — every subflow instance in a trace is findable as a
dispatch record, and the tool loop's story is complete inside the model call.

A `"refused"` flow-as-tool call is the exception that proves the pairing: its
arguments failed the flow's own `inputs:` before an instance existed, so the
record carries neither `instance` nor `result` and there is no dispatch record
to link to. It spends no call ordinal either — the call after it gets the
instance path it would have had with no refusal ahead of it.

Dispatch records and store records carry `idempotencyKey`, which is the one
place the flattened instance path of a nested fan-out can be read back at all —
`execution.item_index` exposes only the innermost index.

## Stability

At a given `trace_version` you may rely on every field the document names, the
presence rules **both ways round**, the closed enumerations, and the orders it
fixes. You may **not** rely on the absence of a field the document does not name
— ignore what you do not recognize — nor on **the text of any message field**.
`error` fields are diagnostics written for a person and are improved between
releases; the one message-shaped field you may match on is an edge decision's
`reason`, because its values are enumerated.

Adding a field, adding a record type, and improving message text are compatible
changes made without a version bump.

## A harness run

A `coder:` node runs somebody else's agent loop (`agent-compose docs agents`),
and what the trace carries of one is a record of its own on the entry's
`harness`: which harness and which pinned SDK, the run's **top-level** turns with
whatever usage that harness reports, its top-level tool events in the same
three-outcome vocabulary a tool call uses, and a cost rollup — money where the
harness estimates one, tokens where it counts them.

Depth is the rule worth knowing: a harness that runs subagents of its own keeps
their transcripts in the run's journal record and out of the trace. The other
one is the count: a `retry:` ladder leaves **one record per attempt**, in the
order the runs were made, and a resumed execution's trace holds the same set —
nothing on a record says which generation ran it, because the trace answers what
the *execution* did.

## What is not in a trace

Secrets are not, and neither is what a human answered at a pause, nor a
harness's own transcript. Those are rules of the format rather than omissions.

Normative source: `docs/trace.md`
