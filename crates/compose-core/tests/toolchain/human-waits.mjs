// Drives a generated project's `human` wait board directly, and reports what it
// did (grammar 8.7, 9.2, PRD 5.11).
//
// The acceptance suite answers, expires and addresses pauses through a served
// app, which is where "the composition behaves" is decided. This is the other
// half: the rules that are only *observable* from inside, because the case that
// breaks them is a task nobody is awaiting any more. A pause left on the board
// after the node holding it stopped waiting is published by the status route as
// a question a person can still answer and taken by the resume route as an
// answer that goes nowhere — and neither end of that is reachable from a
// composition on purpose, because it needs a dispatch to be abandoned at a
// moment a test cannot schedule. The same goes for what an abandonment must
// *not* be — an activity outcome `on_error: skip` can absorb — for the expiry
// timer a released wait leaves behind, and for the budget reading an activity
// divides: nothing a served app answers says whether the timer was cleared or
// what `context.deadline` said while the wait was open.
//
// `src/runtime.ts` is a compiler constant, byte-identical in every project, so
// driving it directly is driving what every project runs.
//
// The timer section replaces the global `setTimeout`/`clearTimeout` for the
// duration of one pause, counting only the calls made with that pause's own
// budget. That is the one way the question "was the timer cleared" has an answer
// at all: an `unref`ed timer keeps nothing alive, so it is invisible to
// `process.getActiveResourcesInfo()` and to the process exiting promptly.
//
// Usage: node human-waits.mjs <generated project directory>
// Output: one JSON object of observations; the expectations live in the Rust
// test that reads it (`generated_code_gates.rs`).

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node human-waits.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

/** The published schema of a sign-off's `output:`, and the parse that holds it. */
const SCHEMA = {
  type: "object",
  properties: { decision: { enum: ["approve", "reject"] } },
  required: ["decision"],
  additionalProperties: false,
};

function parse(payload) {
  if (
    typeof payload !== "object" ||
    payload === null ||
    !["approve", "reject"].includes(payload.decision)
  ) {
    throw new Error("`decision` is `approve` or `reject`");
  }
  return { decision: payload.decision };
}

/** A `human` node's descriptor, as a compiled `graph.ts` emits one. */
function descriptor(fields = {}) {
  return { flow: "flow.sign_off", node: "sign", schema: SCHEMA, parse, ...fields };
}

/** A `NodeView` at `instancePath`, which is what a wait id is derived from. */
function viewAt(instancePath, id) {
  const run = {
    ...runtime.emptyRun(),
    execution: { id, session_key: "" },
    path: instancePath,
  };
  return { state: {}, run, roots: { input: runtime.bind({}, { properties: {} }) } };
}

/** What a parked task ended as: `"pending"`, `"resolved"`, or the error's name. */
function outcomeOf(promise) {
  const held = { state: "pending", value: undefined };
  promise.then(
    (value) => {
      held.state = "resolved";
      held.value = value;
    },
    (error) => {
      held.state = error?.name ?? "Error";
      held.value = error?.message ?? "";
    },
  );
  return held;
}

/** Park one pause and answer the handle its outcome is read through. */
function park(execution, instancePath, fields = {}) {
  const context = { execution: { id: execution, session_key: "" }, node: "sign" };
  return outcomeOf(
    runtime.runHuman(
      descriptor(fields),
      { question: "ship it?" },
      context,
      viewAt(instancePath, execution),
    ),
  );
}

const observed = {};

// A pause is addressed by its instance path, and a node holds the pauses its own
// path is a prefix of — the whole of what links a wait to the node that
// dispatched the instance it happened in.
{
  const execution = "exec_addressing";
  runtime.openHumanWaits(execution, true);
  const first = park(execution, ["fan", "0", "0"]);
  const second = park(execution, ["fan", "0", "1"]);
  await settle();

  observed.addressing = {
    ids: runtime.humanWaits(execution).map((wait) => wait.id),
    under_the_map: runtime.pausesUnder(execution, "fan/0"),
    under_one_instance: runtime.pausesUnder(execution, "fan/0/1"),
    under_the_wait_itself: runtime.pausesUnder(execution, "fan/0/1/sign/0"),
    under_another_node: runtime.pausesUnder(execution, "other/0"),
    both_pending: [first.state, second.state],
  };

  // …and abandoning the node abandons both, which is what keeps an orphan off
  // the status route and out of the resume route.
  runtime.abandonPausesUnder(execution, "fan/0");
  await settle();
  const refused = runtime.deliverHumanAnswer(execution, "fan/0/0/sign/0", {
    decision: "approve",
  });
  observed.abandoning = {
    both_settled: [first.state, second.state],
    still_published: runtime.humanWaits(execution).map((wait) => wait.id),
    still_open: runtime.pausesUnder(execution, "fan/0"),
    refusal: { ok: refused.ok, reason: refused.reason, detail: refused.detail },
  };
  runtime.releaseHumanWaits(execution);
}

// The wiring, rather than the function: one node execution ending is what
// abandons the pauses left under it. The reachable shape is the node's own
// **deadline** racing a parked instance — `runActivity` abandons on every way a
// node execution ends, and a budget that ran out in the tick a pause was opening
// is the one that leaves a pause behind. It is driven here through a throwing
// activity rather than through a real deadline, because what is under test is
// the `finally` rather than which error reached it: the node is over, and the
// parked instance's answer has nobody left to read it.
{
  const execution = "exec_orphan";
  runtime.openHumanWaits(execution, true);
  const parked = park(execution, ["fan", "0", "1"]);
  await settle();
  const before = runtime.pausesUnder(execution, "fan/0");

  let failed;
  try {
    await runtime.runActivity(
      "flow.probe",
      "fan",
      "fan/0",
      {},
      { id: execution, session_key: "" },
      async () => {
        throw new Error("one instance failed while another was still parked");
      },
    );
  } catch (error) {
    failed = error?.name ?? "Error";
  }
  await settle();

  observed.orphans = {
    before,
    after: runtime.pausesUnder(execution, "fan/0"),
    published: runtime.humanWaits(execution).map((wait) => wait.id),
    settled: parked.state,
    node_failed: failed,
    refusal: runtime.deliverHumanAnswer(execution, "fan/0/1/sign/0", { decision: "approve" })
      .reason,
  };
  runtime.releaseHumanWaits(execution);
}

// A wait that is released rather than answered clears its expiry timer, whatever
// the budget was.
{
  const execution = "exec_timer";
  const budget = 86_400_000;
  const armed = [];
  let cleared = 0;
  const realSetTimeout = globalThis.setTimeout;
  const realClearTimeout = globalThis.clearTimeout;
  globalThis.setTimeout = (fn, ms, ...rest) => {
    const timer = realSetTimeout(fn, ms, ...rest);
    if (ms === budget) armed.push(timer);
    return timer;
  };
  globalThis.clearTimeout = (timer) => {
    if (armed.includes(timer)) cleared += 1;
    return realClearTimeout(timer);
  };

  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"], { timeoutMs: budget, onTimeout: "escalate" });
  await settle();
  const armedWhilePending = armed.length;
  const clearedWhilePending = cleared;

  runtime.releaseHumanWaits(execution);
  await settle();

  globalThis.setTimeout = realSetTimeout;
  globalThis.clearTimeout = realClearTimeout;

  observed.timer = {
    armed: armedWhilePending,
    cleared_while_pending: clearedWhilePending,
    cleared_after_release: cleared,
    settled: held.state,
    // The board is gone with the run, so a resume finds nothing to deliver to.
    after_release: runtime.deliverHumanAnswer(execution, undefined, { decision: "approve" })
      .reason,
  };
}

// The ordinary path still works, and settles exactly once.
{
  const execution = "exec_answering";
  runtime.openHumanWaits(execution, true);
  const held = park(execution, ["review", "0"]);
  await settle();

  const published = runtime.humanWaits(execution)[0];
  const taken = runtime.deliverHumanAnswer(execution, undefined, { decision: "reject" });
  await settle();
  const again = runtime.deliverHumanAnswer(execution, "review/0/sign/0", {
    decision: "approve",
  });
  const mismatched = runtime.deliverHumanAnswer(execution, "review/0/sign/0", { decision: 7 });

  observed.answering = {
    published: { id: published?.id, shown: published?.shown, schema: published?.schema },
    taken: { ok: taken.ok, wait: taken.wait?.id },
    settled: held.state,
    output: held.value?.output,
    pause: {
      settled: held.value?.human?.settled,
      paused_at_is_an_instant: typeof held.value?.human?.pausedAt === "string",
      settled_at_is_an_instant: typeof held.value?.human?.settledAt === "string",
      expires_at: held.value?.human?.expiresAt ?? null,
    },
    twice: { ok: again.ok, reason: again.reason },
    // A payload that does not fit a *settled* wait is refused as the settlement
    // rather than as a mismatch: which pause comes before what is in the body.
    mismatched: { ok: mismatched.ok, reason: mismatched.reason },
    still_published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  runtime.releaseHumanWaits(execution);
}

// `on_error:` is a policy over what an activity *did*, and an abandonment is not
// that: it is the run saying nobody is listening for this answer any more. A
// `skip` that absorbed one would route the graph past a `human` node whose
// answer the composition declared it needed — inside an instance whose result
// was already discarded — so it leaves through `runNode` as a throw instead.
{
  const execution = "exec_absorbing";
  runtime.openHumanWaits(execution, true);

  /** A `human` node under `on_error: skip`, with the activity swapped in. */
  const node = (run) => ({
    flow: "flow.sign_off",
    node: "sign",
    policy: { onError: "skip" },
    // A `human` node takes no `timeout:` and no `retry:` at any level (D102).
    exempt: true,
    shapes: {
      input: { properties: {} },
      state: { properties: {} },
      output: { properties: {} },
    },
    input: () => ({}),
    run,
    writes: [],
    edges: [{ to: "__end__" }],
  });
  const stateOf = () => ({
    $run: {
      ...runtime.emptyRun(),
      execution: { id: execution, session_key: "" },
    },
  });

  // The control, and it is what makes the assertion below about the guard
  // rather than about a policy that was never applied: the same node and the
  // same `skip`, over an ordinary failure, really does absorb it.
  const absorbed = await runtime.runNode(
    node(async () => {
      throw new Error("the delivery failed");
    }),
    stateOf(),
  );
  const entry = absorbed.update?.$run?.trace?.[0];

  const parked = outcomeOf(
    runtime.runNode(
      node((input, context, view) =>
        runtime.runHuman(descriptor(), { question: "ship it?" }, context, view),
      ),
      stateOf(),
    ),
  );
  await settle();
  const pending = runtime.humanWaits(execution).map((wait) => wait.id);
  runtime.abandonPausesUnder(execution, "sign/0");
  await settle();

  observed.absorbing = {
    delivery_failure: { outcome: entry?.outcome, goto: absorbed.goto },
    pending,
    abandoned: parked.state,
  };
  runtime.releaseHumanWaits(execution);
}

// The budget of the node *above* a pause is held still while the pause is open
// — and `context.deadline` is that same budget seen from the side an activity
// divides it from, so it moves with the hold. Read as an instant: while the
// budget runs it is the moment the armed timer will fire and does not move,
// and while it is held it is `remaining` from *now* and slides with the clock.
{
  const execution = "exec_budget";
  runtime.openHumanWaits(execution, true);
  const seen = {};
  const tick = () => new Promise((resolve) => setTimeout(resolve, 100));

  await runtime.runActivity(
    "flow.patient",
    "wrap",
    "wrap/0",
    { timeoutMs: 60_000, onError: "fail" },
    { id: execution, session_key: "" },
    async (context) => {
      const armed = context.deadline;
      await tick();
      seen.armed_moved_by = context.deadline - armed;

      const parked = park(execution, ["wrap", "0"]);
      await settle();
      const held = context.deadline;
      await tick();
      seen.held_moved_by = context.deadline - held;

      runtime.deliverHumanAnswer(execution, "wrap/0/sign/0", { decision: "approve" });
      await settle();
      const rearmed = context.deadline;
      await tick();
      seen.rearmed_moved_by = context.deadline - rearmed;
      seen.settled = parked.state;
      return { output: {} };
    },
  );

  observed.budget = seen;
  runtime.releaseHumanWaits(execution);
}

// A run with no resume surface never registers a pause at all: there is nothing
// that could answer it, so it raises instead of parking (grammar 8.7).
{
  const execution = "exec_unanswerable";
  runtime.openHumanWaits(execution, false);
  const held = park(execution, ["review", "0"]);
  await settle();
  observed.unanswerable = {
    settled: held.state,
    published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  runtime.releaseHumanWaits(execution);
}

process.stdout.write(JSON.stringify(observed));
