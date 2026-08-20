# trace

Every run records what it did: which node ran in which superstep, which edge
fired and the guard values that decided it, what each fan-out dispatched, every
store op, every model call, and every human pause. The format is versioned and
documented, so a consumer can pin `trace_version` and walk it.

This topic is orientation. `docs/trace.md` is normative and is what a reader
pins on.

## Where a trace is delivered

Three surfaces carry the same entries and differ in what surrounds them:

| surface | what carries it |
|---|---|
| `agent-compose run --format json` | one JSON object on **stdout**, whose `trace` is the array of entries |
| the trace **file** | one JSON object — the whole envelope — under the project's data directory |
| `agent-compose serve` status | `GET /executions/:id`, and the `callback:` webhook body |

Wherever a `trace` appears, the `trace_version` describing it appears beside it,
and wherever one is absent so is the other.

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

Entries are ordered by `(step, node)` — ascending superstep, then ascending node
id. Node id rather than completion order, so two runs of one composition produce
the entries in the same order. One entry is outside that ordering and is always
last: the entry of the node a **failed** run aborted at. Nested entries have
step numbers of their own instance, starting again at 1.

**Outcomes** are `"completed"`, `"skipped"` (the activity failed and
`on_error: skip` absorbed it — it carries an `error` too) and `"failed"`. A
failed entry has two shapes told apart by `fallback`: with it, the failure
routed to the named node and the run continues; without it, the failure ended
the run.

A flow whose `start` edges carry guards gets a synthetic entry node,
`node: "$start"`. The `$` sigil is outside the identifier grammar, so it
collides with nothing you can declare.

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

## What is not in a trace

Secrets are not, and neither is what a human answered at a pause. Those are
rules of the format rather than omissions.

Normative source: `docs/trace.md`
