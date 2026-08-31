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
// One precedence is here for a different reason. Deciding it takes a
// composition that declares `on_timeout:` beside an *explicit* `on_error:`, and
// no fixture a served app runs does: every composition that can reach an expiry
// resolves `on_error: fail`, where routing the `on_timeout:` fallback and
// absorbing the expiry look identical from outside. So a delivery failure, an
// expiry and an interrupt are each driven through one `human` node under one
// `on_error: skip` — the first absorbed, the other two not, because neither of
// them is a thing the activity did.
//
// Four sections are about one id being used twice. A retry ladder re-executes an
// instance at the site its predecessor ran at, so the pause a failed attempt left
// parked and the pause the next attempt opens are two waits under one id — and
// the shape that produces one needs a branch of an instance to fail while a
// sibling of it is parked, which is a scheduling no composition can ask for. All
// three halves of the rule are driven: each of the two ladders abandons what it
// left behind, the board refuses a pause that would displace an unsettled one at
// all, and a settlement reaches the pause it belongs to and no other.
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

/**
 * Wait until something has happened, rather than for a number of milliseconds.
 *
 * The one section that waits on a real expiry timer uses it: a fixed sleep long
 * enough to be safe on a loaded runner is a slow test, and one short enough to
 * be quick is a flaky one.
 */
async function until(ready, ms = 5_000) {
  const deadline = Date.now() + ms;
  while (!ready() && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

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
function park(execution, instancePath, fields = {}, effects = undefined) {
  const context = {
    execution: { id: execution, session_key: "" },
    node: "sign",
    ...(effects === undefined ? {} : { effects }),
  };
  return outcomeOf(
    runtime.runHuman(
      descriptor(fields),
      { question: "ship it?" },
      context,
      viewAt(instancePath, execution),
    ),
  );
}

/**
 * An effect recorder that **cannot write**, holding nothing for this key.
 *
 * A settled pause is journaled where the run is journaling — the answer is the
 * one payload `docs/trace.md` §11 keeps out of the trace, so the journal is the
 * only account of it — and this is the one seam where the write can refuse: a
 * disk with nothing left on it, or a lock another process is holding past
 * `busy_timeout`. Handed a slot that holds nothing, so the pause really parks
 * and the refusal happens at the settlement rather than at the claim.
 *
 * It is a stub rather than a real journal because what is under test is what
 * `runHuman` does with a throw, and a real one made to throw on command would be
 * the same stub with a database behind it.
 */
function refusing(error) {
  return {
    child: () => refusing(error),
    claim: () => ({
      key: "sign/0#human/0",
      site: "sign/0",
      kind: "human",
      ordinal: 0,
      held: undefined,
      keep: () => {
        throw error;
      },
      fail: () => {},
    }),
  };
}

/**
 * A `human` node under an explicit `on_error: skip`, with the activity swapped
 * in — the shape the three precedence sections share.
 *
 * Its own outgoing edge goes to `__end__`, so a section whose `on_timeout:`
 * names anything else can tell "the fallback was routed to" apart from "the
 * node's own edges were evaluated" by reading `goto` alone.
 */
function skipping(run) {
  return {
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
  };
}

/** The state a `runNode` call reads its execution from. */
function stateFor(execution) {
  return {
    $run: {
      ...runtime.emptyRun(),
      execution: { id: execution, session_key: "" },
    },
  };
}

/** The `human` activity of [`skipping`], as a node's `run`. */
function pausing(fields = {}) {
  return (input, context, view) =>
    runtime.runHuman(descriptor(fields), { question: "ship it?" }, context, view);
}

/** A `NodeView` over a `state.items` array, which is what a `map`'s `over:` reads. */
function mapView(items, execution) {
  const shape = { properties: { items: { items: "any" } } };
  const run = {
    ...runtime.emptyRun(),
    execution: { id: execution, session_key: "" },
    path: [],
    traversals: {},
  };
  return {
    state: { items },
    run,
    roots: {
      input: runtime.bind({}, { properties: {} }),
      state: runtime.bind({ items }, shape),
      execution: runtime.bind(run.execution, {
        properties: { id: "string", session_key: "string", item_index: "int" },
      }),
    },
  };
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

// A retry ladder is the other way one task stops waiting for a pause, and the
// sharper one: the next attempt re-executes the instance at the **same** site,
// so a pause the failed attempt left parked would share its id with the one the
// new attempt opens there. A `flow:` node's attempt is a whole instance and the
// failure that ends it is yielded as soon as it settles, which is exactly the
// shape that leaves a sibling branch parked — so every attempt abandons what it
// left behind before the next one starts, and each attempt begins with nothing
// held under its site.
{
  const execution = "exec_retrying";
  runtime.openHumanWaits(execution, true);
  const parked = [];
  const openAtEntry = [];
  let failed;
  try {
    await runtime.runActivity(
      "flow.probe",
      "wrap",
      "wrap/0",
      { retry: { max: 1, backoffMs: 1, multiplier: 1, jitter: false }, onError: "fail" },
      { id: execution, session_key: "" },
      async () => {
        openAtEntry.push(runtime.pausesUnder(execution, "wrap/0"));
        // A budget no attempt can outlive, so what settles these pauses is the
        // ladder rather than a timer racing it.
        parked.push(park(execution, ["wrap", "0"], { timeoutMs: 86_400_000, onTimeout: "escalate" }));
        await settle();
        throw new Error("one branch failed while another was still parked");
      },
    );
  } catch (error) {
    failed = error?.name ?? "Error";
  }
  await settle();

  observed.retrying = {
    open_at_each_attempt: openAtEntry,
    settled: parked.map((one) => one.state),
    published: runtime.humanWaits(execution).map((wait) => wait.id),
    node_failed: failed,
  };
  runtime.releaseHumanWaits(execution);
}

// `on_item_error: { retry: … }` is the same seam one construct further in: an
// item's attempts all re-execute the instance at one dispatch site. Driven
// through `runActivity` because a compiled `map` node is always inside one, and
// the pause the **last** attempt leaves is the node execution's to abandon.
{
  const execution = "exec_item_retrying";
  runtime.openHumanWaits(execution, true);
  const parked = [];
  const openAtEntry = [];
  const map = {
    node: "fan",
    as: "item",
    source: { path: "state.items", shape: "any" },
    maxConcurrency: 1,
    onItemError: { retry: { max: 1, backoffMs: 1, multiplier: 1, jitter: false } },
    routes: [
      {
        target: "flow.sign_off",
        maxConcurrency: 1,
        detach: false,
        itemShape: "any",
        input: () => ({}),
        writes: [],
        run: async (input, itemContext, site) => {
          const at = site.path.join("/");
          openAtEntry.push(runtime.pausesUnder(execution, at));
          parked.push(park(execution, site.path));
          await settle();
          throw new Error("one branch failed while another was still parked");
        },
      },
    ],
  };
  const view = mapView([{ at: 0 }], execution);
  let failed;
  try {
    await runtime.runActivity(
      "flow.probe",
      "fan",
      "fan/0",
      { onError: "fail" },
      { id: execution, session_key: "" },
      async (context) => await runtime.runMap(map, runtime.mapPlan(map, view), context),
    );
  } catch (error) {
    failed = error?.name ?? "Error";
  }
  await settle();

  observed.item_retrying = {
    open_at_each_attempt: openAtEntry,
    settled: parked.map((one) => one.state),
    published: runtime.humanWaits(execution).map((wait) => wait.id),
    node_failed: failed,
  };
  runtime.releaseHumanWaits(execution);
}

// …and the half of that which is not an ordering: a settlement reaches the pause
// it belongs to and no other. A wait id is an instance path and a path is re-run,
// so once a ladder has abandoned what it left parked the board holds a
// *successor* pause under an id a stale closure still remembers — and the stale
// closure with the longest reach is an expiry timer, which can already be in
// flight when its own entry is settled and replaced.
//
// The timer is fired by hand rather than by the clock, because "the callback runs
// after the entry it belongs to was settled and replaced" is an interleaving no
// sleep can schedule: `setTimeout` is swapped for the length of the first pause
// so the callback it was handed can be kept and run at the chosen moment. What
// the section decides is *what that callback does* — settle itself, or whatever
// now answers to its id.
{
  const execution = "exec_successor";
  runtime.openHumanWaits(execution, true);

  const budget = 20;
  let expire;
  const realSetTimeout = globalThis.setTimeout;
  globalThis.setTimeout = (fn, ms, ...rest) => {
    if (ms !== budget) return realSetTimeout(fn, ms, ...rest);
    expire = fn;
    // A handle `clearTimeout` accepts, attached to nothing: the callback is this
    // section's to run, and running it is the whole point.
    return realSetTimeout(() => {}, 0);
  };
  const stale = park(execution, ["wrap", "0"], { timeoutMs: budget, onTimeout: "escalate" });
  await settle();
  globalThis.setTimeout = realSetTimeout;

  // What every ladder does before re-executing an instance at a site — and what
  // the board now refuses to let one skip (see the section below).
  runtime.abandonPausesUnder(execution, "wrap/0");
  await settle();
  const live = park(execution, ["wrap", "0"]);
  await settle();

  // The stale budget, running out one settlement too late.
  expire?.();
  await settle();

  const open = runtime.pausesUnder(execution, "wrap/0");
  const published = runtime.humanWaits(execution).map((wait) => wait.id);
  const taken = runtime.deliverHumanAnswer(execution, "wrap/0/sign/0", { decision: "approve" });
  await settle();

  observed.successor = {
    stale: stale.state,
    open_after_the_stale_budget_ran_out: open,
    published,
    taken: { ok: taken.ok, wait: taken.wait?.id },
    live: live.state,
  };
  runtime.releaseHumanWaits(execution);
}

// …and the ordering itself, which is the board's own rule rather than a
// convention three call sites keep. A pause opened under an id the board is
// still holding an **unsettled** wait at would take that wait off the board with
// nothing left able to reach it: no resume can address it, no
// `abandonPausesUnder` or `releaseHumanWaits` can find it, and the task holding
// it is parked on a promise nothing can settle — a run that hangs, which is the
// one failure a test cannot tell from a slow machine. The board refuses instead,
// at the moment the invariant breaks, and the standing pause is untouched: still
// published, still open, and still what an answer reaches.
{
  const execution = "exec_displacing";
  runtime.openHumanWaits(execution, true);
  const standing = park(execution, ["wrap", "0"]);
  await settle();
  const displacing = park(execution, ["wrap", "0"]);
  await settle();

  // Read before the answer: what the refusal must have left behind is a board
  // still holding the standing pause, not one the delivery below put back.
  const published = runtime.humanWaits(execution).map((wait) => wait.id);
  const open = runtime.pausesUnder(execution, "wrap/0");
  const taken = runtime.deliverHumanAnswer(execution, "wrap/0/sign/0", { decision: "approve" });
  await settle();

  observed.displacing = {
    refused: displacing.state,
    said: displacing.value,
    published,
    open,
    taken: { ok: taken.ok, wait: taken.wait?.id },
    standing: standing.state,
    output: standing.value?.output,
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

// …and a journal that **refuses the record** fails the node rather than leaving
// the pause parked for ever.
//
// The wait is marked settled before its record is written — it has to be, or an
// answer and an expiry could both land — so a write that threw out of the
// settlement would leave a wait nothing may settle again holding a promise
// nothing ever settles: `closeHumanWaits` and `releaseHumanWaits` both skip a
// settled entry and a later delivery refuses it. The `human` node's `await`
// would never return, the graph would never advance, and the run would neither
// fail nor park nor end. So the write's error leaves through the parked promise:
// the node fails with it, and the answer is not offered again, because the turn
// was spent.
{
  const execution = "exec_unwritable";
  runtime.openHumanWaits(execution, true);
  const held = park(
    execution,
    ["review", "0"],
    {},
    refusing(new Error("the journal refused this record")),
  );
  await settle();

  let delivery = null;
  let threw = null;
  try {
    delivery = runtime.deliverHumanAnswer(execution, undefined, { decision: "approve" });
  } catch (error) {
    threw = error?.message ?? String(error);
  }
  await settle();

  observed.unwritable_answer = {
    delivery: { ok: delivery?.ok ?? null, threw },
    settled: held.state,
    reported: held.value,
    still_published: runtime.humanWaits(execution).map((wait) => wait.id),
    again: runtime.deliverHumanAnswer(execution, "review/0/sign/0", { decision: "approve" })
      .reason,
  };
  runtime.releaseHumanWaits(execution);
}

// The same on the settlement nobody is waiting to be told about. This arm is
// reached from a `setTimeout` callback, where a throw is an uncaught exception
// rather than something a caller could report — so the run would die of the
// write rather than fail of it, and a `serve` process would take every other
// execution with it. The expiry is not routed either: a run that took
// `on_timeout:` past a wait whose expiry the journal does not hold would re-park
// on the resume and spend the budget a second time.
{
  const execution = "exec_unwritable_expiry";
  runtime.openHumanWaits(execution, true);
  const held = park(
    execution,
    ["review", "0"],
    { timeoutMs: 5, onTimeout: "escalate" },
    refusing(new Error("the journal refused this record")),
  );
  await until(() => held.state !== "pending");

  observed.unwritable_expiry = {
    settled: held.state,
    reported: held.value,
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

  // The control, and it is what makes every assertion below about a guard
  // rather than about a policy that was never applied: the same node and the
  // same `skip`, over an ordinary failure, really does absorb it.
  const absorbed = await runtime.runNode(
    skipping(async () => {
      throw new Error("the delivery failed");
    }),
    stateFor(execution),
  );
  const entry = absorbed.update?.$run?.trace?.[0];

  const parked = outcomeOf(runtime.runNode(skipping(pausing()), stateFor(execution)));
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

// …and neither is an **expiry**. A budget that ran out is the composition's own
// control flow — `on_timeout:` transfers control to the route it names *instead
// of* evaluating the node's outgoing edges (grammar 8.7, 9.2) — so a node
// declaring both keys must route to `on_timeout:`'s target rather than let
// `on_error: skip` mark it skipped and take its own edge onward. The two keys
// are legal together (`retry:`/`timeout:` are the pair a `human` node refuses,
// D102), and no served composition can decide this: the acceptance fixture's
// expiring flow resolves `on_error: fail`, so the ordering `runNode` reads them
// in is only visible from here. The node's edge goes to `__end__` and the
// `on_timeout:` route does not, so `goto` alone says which one fired.
{
  const execution = "exec_expiry_over_skip";
  runtime.openHumanWaits(execution, true);
  const ended = outcomeOf(
    runtime.runNode(
      skipping(pausing({ timeoutMs: 20, onTimeout: "note" })),
      stateFor(execution),
    ),
  );
  await until(() => ended.state !== "pending");
  const entry = ended.value?.update?.$run?.trace?.[0];

  observed.expiry_over_skip = {
    settled: ended.state,
    goto: ended.value?.goto,
    outcome: entry?.outcome,
    fallback: entry?.fallback,
    pause_settled: entry?.human?.settled,
    published: runtime.humanWaits(execution).map((wait) => wait.id),
  };
  runtime.releaseHumanWaits(execution);
}

// …and neither is an **interrupt**. A run with no resume surface is the run's
// own shape rather than something the activity did, so it leaves `runNode` as a
// throw ahead of the policy: `skip` absorbing it would carry the graph past a
// `human` node whose answer the composition declared it needed — which is what
// `agent-compose run` reports as its own exit path instead. Under the same
// explicit `on_error: skip` as the control above.
{
  const execution = "exec_interrupt_over_skip";
  runtime.openHumanWaits(execution, false);
  let refused;
  try {
    const skipped = await runtime.runNode(skipping(pausing()), stateFor(execution));
    refused = {
      threw: false,
      outcome: skipped.update?.$run?.trace?.[0]?.outcome,
      goto: skipped.goto,
    };
  } catch (error) {
    refused = {
      threw: true,
      name: error?.name ?? "Error",
      interrupt: runtime.interruptOf(error)?.name ?? null,
      node: runtime.interruptOf(error)?.node ?? null,
    };
  }
  await settle();

  observed.interrupt_over_skip = refused;
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

// ---------------------------------------------------------------------------
// A pause a **worker** opened (`docs/distributed.md` §3.4, PRD resolved q46)
// ---------------------------------------------------------------------------
//
// The parity bar q46 sets is "a placed `human:` node must mean what the same node
// unplaced means", and this is where that is decidable: the sections above drove
// a local pause through this board, and these drive a remote one through the
// *same* board and read the same observations back. What a served hub can show
// is the status shape and the resume; what only this can show is that the wait
// is the board's own entry — counted by `pausesUnder`, so the dispatching node's
// budget is held still (D102); refused by a mismatched payload without consuming
// the turn; abandoned when the run ends; and settled into exactly the record a
// local pause writes through `slot.keep`, which is what the redispatch replays.

/** A recorder that holds nothing, so a claim lands at the frontier. */
function claiming(key, site) {
  return {
    child: () => claiming(key, site),
    claim: () => ({
      key,
      site,
      kind: "human",
      ordinal: 0,
      request: '{"node":"sign"}',
      held: undefined,
      keep: (value) => value,
      fail: () => {},
    }),
  };
}

/** The pause a worker settles its dispatch with, as §3.4 carries one home. */
function remote(fields = {}) {
  const node = fields.node ?? "sign";
  return {
    wait: `escalate/0/${node}/0`,
    flow: "flow.sign_off",
    node,
    shown: { question: "ship it?" },
    pausedAt: "2026-08-31T09:14:02.113Z",
    effect: {
      key: `escalate/0/${node}/0#human/0`,
      site: `escalate/0/${node}/0`,
      ordinal: 0,
      request: `{"node":"${node}"}`,
    },
    ...fields,
  };
}

/**
 * The hub's writer, as `./mesh.ts`'s `answered` hands one in: what a local
 * pause's `slot.keep` is, for a wait a worker opened.
 *
 * `wrote` is appended to inside the settlement and the promise's continuation
 * appends to it after, so the order of the two is the reading that says the
 * record was written **before** the answer was acknowledged
 * (`docs/durability.md` §3.4).
 */
function writer(wrote) {
  return (record) => {
    wrote.push({ kept: record });
  };
}

// The hub reads the contract off its own copy of the descriptor rather than off
// anything that travelled (§4.3), which is what this registration is. Three
// nodes, because the budget is read off the descriptor too: `sign` declares no
// `timeout:` and waits, `decide` declares a short one, and `confirm` declares a
// long one — which is what makes "the wire's `expires_at` is not the timer"
// decidable below.
runtime.registerHumanNodes({
  "flow.sign_off.sign": descriptor(),
  "flow.sign_off.decide": descriptor({ node: "decide", timeoutMs: 30, onTimeout: "__end__" }),
  "flow.sign_off.confirm": descriptor({
    node: "confirm",
    timeoutMs: 60_000,
    onTimeout: "__end__",
  }),
});

{
  const execution = "exec_remote_answered";
  runtime.openHumanWaits(execution, true);
  const pause = remote();
  // The two events whose **order** is the durability rule: the record reaching
  // the hub's writer, and the promise the resume route's `202` is answered off
  // resolving. A record written in a later turn of the loop is one a process
  // killed in between never wrote, and the person is asked again on the restart.
  const wrote = [];
  const promise = runtime.holdRemotePause(execution, pause, writer(wrote));
  const held = outcomeOf(promise);
  void promise.then(
    () => wrote.push({ resolved: true }),
    () => wrote.push({ rejected: true }),
  );
  await settle();
  const published = runtime.humanWaits(execution);
  // A payload the node's `output:` refuses does **not** consume the wait.
  const refused = runtime.deliverHumanAnswer(execution, pause.wait, { decision: "maybe" });
  const plantedAt = published[0]?.pausedAt;
  const seen = {
    published: published.map((wait) => ({
      id: wait.id,
      flow: wait.flow,
      node: wait.node,
      shown: wait.shown,
      schema: wait.schema,
    })),
    // …dated where it was planted rather than where it was asked: the wire's
    // instant is another machine's clock, and the pair a reader is shown has to
    // be one clock's (`docs/distributed.md` §3.4).
    published_paused_at_is_an_instant: typeof plantedAt === "string",
    published_paused_at_is_the_wires: plantedAt === pause.pausedAt,
    // The reading `runActivity` holds a dispatching node's deadline still by.
    held_under: runtime.pausesUnder(execution, "escalate/0"),
    held_elsewhere: runtime.pausesUnder(execution, "stamp/0"),
    refused: refused.ok === false ? refused.reason : "taken",
    waiting_after_a_mismatch: runtime.humanWaits(execution).length,
  };
  runtime.deliverHumanAnswer(execution, pause.wait, { decision: "approve" });
  await settle();
  seen.settled = held.state;
  seen.record = held.value;
  // The journal keeps the instant the board published, so the answered pause's
  // trace entry is the entry an unplaced pause writes.
  seen.record_carries_the_published_pause = held.value?.pausedAt === plantedAt;
  seen.waiting_after_the_answer = runtime.humanWaits(execution).length;
  // What the writer was handed, and when: the record itself, and before the
  // promise the answer is acknowledged off resolved.
  seen.wrote = wrote.map((event) => (event.kept === undefined ? "resolved" : "kept"));
  seen.written_record = wrote.find((event) => event.kept !== undefined)?.kept;
  // …and a **second** answer is refused, exactly as a local pause's is: a wait
  // is settled once, and a delivery that re-settled one would journal a second
  // `human` record over an answer somebody already gave.
  const twice = runtime.deliverHumanAnswer(execution, pause.wait, { decision: "reject" });
  await settle();
  seen.twice = twice.ok === false ? twice.reason : "taken";
  seen.record_after_the_second_answer = held.value;
  observed.remote_answered = seen;
  runtime.releaseHumanWaits(execution);
}

{
  // The budget is the **composition's**, spent from the moment this hub plants
  // the wait: `descriptor.timeoutMs`, not the wire's `expiresAt`.
  const execution = "exec_remote_expired";
  runtime.openHumanWaits(execution, true);
  const wrote = [];
  const held = outcomeOf(
    runtime.holdRemotePause(
      execution,
      remote({ node: "decide", expiresAt: new Date(Date.now() + 30).toISOString() }),
      writer(wrote),
    ),
  );
  await until(() => held.state !== "pending");
  observed.remote_expired = {
    settled: held.state,
    record: held.value,
    // An expiry is journaled through the same writer, and it has to be: a run
    // that took `on_timeout:` past a wait whose expiry the journal does not hold
    // would re-park on the resume and spend the budget again.
    written_record: wrote.find((event) => event.kept !== undefined)?.kept,
  };
  runtime.releaseHumanWaits(execution);
}

{
  // **A worker's clock is not this hub's**, and a wait's budget may not depend
  // on the difference. `expiresAt` here is an hour in this process's past — what
  // a worker an hour behind would stamp on a pause it opened a moment ago — and
  // the node's own budget is a minute, so the wait is still open. Armed off the
  // wire it would have expired on the next tick, `on_timeout:` would have routed,
  // and nobody could ever have answered a question the composition gave a minute.
  const execution = "exec_remote_skewed";
  runtime.openHumanWaits(execution, true);
  const pause = remote({
    node: "confirm",
    expiresAt: new Date(Date.now() - 3_600_000).toISOString(),
  });
  const held = outcomeOf(runtime.holdRemotePause(execution, pause, () => {}));
  await settle();
  const seen = { settled_while_the_budget_runs: held.state };
  // …and the deadline a reader is shown is the one this hub will fire, derived
  // beside the arming rather than taken off the wire. The wire's instant is an
  // hour past, so publishing it would show the question as expired for the whole
  // minute the resume surface still takes its answer.
  const shown = runtime.humanWaits(execution)[0]?.expiresAt;
  seen.published_expires_at_is_the_wires = shown === pause.expiresAt;
  seen.published_expires_at_is_ahead =
    typeof shown === "string" && shown > new Date().toISOString();
  runtime.deliverHumanAnswer(execution, pause.wait, { decision: "approve" });
  await settle();
  seen.settled = held.state;
  seen.record = held.value;
  // The record holds the deadline the board published, which is the one the
  // timer was armed for: a replayed wait shows what this execution was under.
  seen.record_dates_the_deadline_it_published = held.value?.expiresAt === shown;
  observed.remote_skewed = seen;
  runtime.releaseHumanWaits(execution);
}

{
  // **The pair a reader is shown is one clock's**, which is PRD resolved q46's
  // parity bar for status visibility: the same node unplaced dates both members
  // off one `Date.now()` reading, so a placed one must too. This worker's clock
  // runs an hour *ahead* of this process's — it dates the question an hour from
  // here and stamps the deadline its own minute of budget gives it — and neither
  // instant reaches the board. What the planting publishes is its own now and
  // its own now plus the descriptor's minute, so `expiresAt − pausedAt` is
  // exactly the `timeout:` the composition declares. A board that had kept the
  // wire's `pausedAt` would publish an *inverted* pair here: a deadline a minute
  // from now beside a question asked an hour from now.
  const execution = "exec_remote_planting";
  runtime.openHumanWaits(execution, true);
  const asked = new Date(Date.now() + 3_600_000).toISOString();
  const pause = remote({
    node: "confirm",
    pausedAt: asked,
    expiresAt: new Date(Date.parse(asked) + 60_000).toISOString(),
  });
  const planted = Date.now();
  const held = outcomeOf(runtime.holdRemotePause(execution, pause, () => {}));
  await settle();
  const shown = runtime.humanWaits(execution)[0];
  const seen = {
    // Open, because the budget armed is the descriptor's and is spent from here.
    settled_while_the_budget_runs: held.state,
    published_paused_at_is_the_wires: shown?.pausedAt === pause.pausedAt,
    published_expires_at_is_the_wires: shown?.expiresAt === pause.expiresAt,
    // What the composition's minute is worth from the instant this hub planted
    // the wait.
    budget_from_the_planting_ms:
      typeof shown?.expiresAt === "string" ? Date.parse(shown.expiresAt) - planted : null,
    // …and how far the *dating* is from that same instant, which is what says
    // the question was dated here rather than an hour from here.
    dated_from_the_planting_ms:
      typeof shown?.pausedAt === "string" ? Date.parse(shown.pausedAt) - planted : null,
    // …and what subtracting one published member from the other says, which is
    // the node's `timeout:` and nothing else.
    published_gap_ms:
      typeof shown?.expiresAt === "string"
        ? Date.parse(shown.expiresAt) - Date.parse(shown.pausedAt)
        : null,
  };
  runtime.deliverHumanAnswer(execution, pause.wait, { decision: "approve" });
  await settle();
  seen.settled = held.state;
  // The journal keeps the pair the board published, so the answered pause's own
  // trace entry is the entry an unplaced pause writes (`docs/trace.md` §3.4).
  seen.record_carries_the_published_pair =
    held.value?.pausedAt === shown?.pausedAt && held.value?.expiresAt === shown?.expiresAt;
  observed.remote_planting = seen;
  runtime.releaseHumanWaits(execution);
}

{
  // …and a node the artifact gives no `timeout:` publishes no deadline at all,
  // however the wire dated the pause. Grammar 8.7 makes that wait unbounded, so
  // there is no timer — and an instant nothing will ever fire is not one a
  // status route may show. `sign` declares none; the pause carries an hour.
  const execution = "exec_remote_unbounded";
  runtime.openHumanWaits(execution, true);
  const dated = remote({ expiresAt: new Date(Date.now() + 3_600_000).toISOString() });
  const held = outcomeOf(runtime.holdRemotePause(execution, dated, () => {}));
  await settle();
  observed.remote_unbounded = {
    settled: held.state,
    published_expires_at: runtime.humanWaits(execution)[0]?.expiresAt ?? null,
  };
  runtime.releaseHumanWaits(execution);
}

{
  // **A composition that has stopped declaring the node the pause names.**
  // Unreachable while one generation holds the pause — `./mesh.ts`'s `pauseOf`
  // refuses it before the dispatch is settled — but a hub restarted on a rebuilt
  // artifact re-derives an unanswered pause straight off the settled row, where
  // no route reads the body again. That is resolved q29's disagreement exactly,
  // so it is that class, named at the record the answer would have been written
  // under, and travels past every policy rather than being absorbed as a node
  // failure.
  const execution = "exec_remote_unregistered";
  runtime.openHumanWaits(execution, true);
  const gone = outcomeOf(
    runtime.holdRemotePause(execution, remote({ node: "withdrawn" }), () => {}),
  );
  await settle();
  observed.remote_unregistered = {
    settled: gone.state,
    names_the_record: (gone.value ?? "").includes("escalate/0/withdrawn/0#human/0"),
    published: runtime.humanWaits(execution).length,
  };
  runtime.releaseHumanWaits(execution);
}

{
  // **Planting the wait is what arms it, every time it is planted.** A hub that
  // re-derives an unanswered pause after a restart calls this function again,
  // and what it gets is the node's whole `timeout:` — the same thing a resumed
  // generation gives a local wait it re-parks (`docs/durability.md` §5), which
  // is what PRD resolved q46's parity bar asks for. There is no instant to pass:
  // the signature carries no elapsed time, so no caller can spend a
  // predecessor's. Driven here as the second planting of one wait identity: the
  // first runs out its short budget, the second is given the whole of it again.
  const execution = "exec_remote_replanted";
  runtime.openHumanWaits(execution, true);
  const pause = remote({ node: "decide" });
  const first = outcomeOf(runtime.holdRemotePause(execution, pause, () => {}));
  await until(() => first.state !== "pending");
  const replanted = Date.now();
  const again = outcomeOf(runtime.holdRemotePause(execution, pause, () => {}));
  await until(() => again.state !== "pending");
  observed.remote_replanted = {
    first: first.state,
    replanted: again.state,
    // How long the **second** planting lasted, in whole milliseconds. A budget
    // that carried its predecessor's spending would have run out on the next
    // tick; the node declares thirty milliseconds and the second wait gets them.
    lasted: Date.now() - replanted,
  };
  runtime.releaseHumanWaits(execution);
}

{
  // The two settlements that are the run's own shape, not the composition's.
  const abandoned = "exec_remote_abandoned";
  runtime.openHumanWaits(abandoned, true);
  const dropped = outcomeOf(runtime.holdRemotePause(abandoned, remote(), () => {}));
  await settle();
  runtime.releaseHumanWaits(abandoned);
  await settle();

  const withdrawn = "exec_remote_withdrawn";
  runtime.openHumanWaits(withdrawn, true);
  const closed = outcomeOf(runtime.holdRemotePause(withdrawn, remote(), () => {}));
  await settle();
  runtime.closeHumanWaits(withdrawn);
  await settle();

  const unanswerable = "exec_remote_unanswerable";
  runtime.openHumanWaits(unanswerable, false);
  const raised = outcomeOf(runtime.holdRemotePause(unanswerable, remote(), () => {}));
  await settle();

  observed.remote_unsettled = {
    abandoned: dropped.state,
    withdrawn: closed.state,
    unanswerable: raised.state,
  };
  runtime.releaseHumanWaits(withdrawn);
  runtime.releaseHumanWaits(unanswerable);
}

// …and the other end of the same wire, **last**, because the switch it turns on
// is the process's and is never turned off: a worker runs one dispatch and exits
// (`./worker-node.ts`). What it proves is the half no served hub can: the wait
// identity a worker sends home is derived by the *same* `runHuman` at the *same*
// view a local pause is derived by, so a placed node's question is addressed by
// the id an unplaced one would have opened.
{
  const execution = "exec_travelling";
  runtime.openHumanWaits(execution, true);
  park(execution, ["escalate", "0"]);
  await settle();
  const opened = runtime.humanWaits(execution).map((wait) => wait.id);
  runtime.releaseHumanWaits(execution);
  await settle();

  runtime.dispatchPausesHome();
  const away = "exec_travelled";
  const recorder = claiming("escalate/0/sign/0#human/0", "escalate/0/sign/0");
  let carried = { name: "no throw" };
  try {
    // No board at all, which is what a worker is: nothing here opens one.
    await runtime.runHuman(
      descriptor(),
      { question: "ship it?" },
      { execution: { id: away, session_key: "" }, node: "sign", effects: recorder },
      viewAt(["escalate", "0"], away),
    );
  } catch (error) {
    carried = {
      name: error?.name,
      wait: error?.remote?.wait,
      flow: error?.remote?.flow,
      node: error?.remote?.node,
      shown: error?.remote?.shown,
      effect: error?.remote?.effect,
      // Every ladder between a `human` node and this wire lets an interrupt
      // through untouched, and a pause is one — which is why `on_error: skip`
      // cannot absorb it on its way to the result line.
      travels_as_an_interrupt: runtime.interruptOf(error) !== undefined,
    };
  }
  observed.travelling = {
    opened,
    carried,
    published: runtime.humanWaits(away).map((wait) => wait.id),
  };
}

process.stdout.write(JSON.stringify(observed));
