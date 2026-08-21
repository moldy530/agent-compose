// Drives a generated project's fan-out machinery directly, and reports what it
// did (grammar 8.6, 7.6.4, 9.4).
//
// The acceptance suite runs a fan-out through a compiled graph against scripted
// model answers, which is where "the composition behaves" is decided. This is
// the other half: the rules that are only *observable* from inside — how many
// instances were in flight at once, which item a failure was reported about when
// two of them failed, whether the join returned before a detached delivery
// finished, what that delivery put on the wire, how many attempts an item made
// when the node's deadline cut a backoff short — and the ones a composition
// cannot make adversarial enough on purpose. `src/runtime.ts` is a compiler
// constant, byte-identical in every project, so driving it directly is driving
// what every project runs.
//
// Two sections reach past the runtime on purpose. The channel section builds a
// real `StateGraph` the way `codegen::state` builds one, because a reduce policy
// is half reducer and half channel and the half that swallowed a fan-out's batch
// was the channel. The delivery section intercepts `fetch`, runs a real
// subprocess, and registers a real host function — one per binding kind grammar
// 9.4 fixes a surface for — because a key that is derived and recorded but never
// sent is indistinguishable from a delivered one anywhere else.
//
// Four sections drive a map through **`runtime.runNode`** rather than calling
// `runMap` directly, because a compiled map node is always inside one: grammar
// 9.3 level 3 (`defaults:`) puts a `timeout:` and a `retry:` on every node a
// composition declares, maps included, and the emitted `examples/triage-fanout`
// carries both on its `dispatch` node. `runMap` on its own never sees them —
// its `context.signal` never aborts and its call is never repeated — so the
// rules that only exist at that seam (a queued delivery when the budget runs
// out, the record a node that returned no answer still owes, a bound that has to
// span two executions of one node) are decided from `runNode` downwards.
//
// One consequence worth stating: `max_concurrency` is admitted against a table
// keyed by **node identity**, which is process-wide and outlives a call. Every
// section therefore lets its detached deliveries finish before the next one
// starts, and the sections that cannot are given an execution id of their own.
//
// Every completion order here is deliberately the reverse of source order: an
// ordering rule that holds only when the instances happen to finish in order is
// not a rule.
//
// Usage: node map-dispatch.mjs <generated project directory>
// Output: one JSON object of observations; the expectations live in the Rust
// test that reads it (`generated_code_gates.rs`).

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

import { Annotation, StateGraph } from "@langchain/langgraph";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node map-dispatch.mjs <generated project directory>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

/** A sleep that a node's deadline can cut short, the way a real activity's does. */
const naps = (ms, signal) =>
  new Promise((resolve, reject) => {
    const timer = setTimeout(resolve, ms);
    signal?.addEventListener(
      "abort",
      () => {
        clearTimeout(timer);
        reject(signal.reason instanceof Error ? signal.reason : new Error("aborted"));
      },
      { once: true },
    );
  });

/** A `NodeView` over a `state.items` array, which is what `over` reads here. */
function viewOf(items, { path: instancePath = [], traversals = {}, id = "exec_gate" } = {}) {
  const shape = { properties: { items: { items: "any" } } };
  const run = {
    ...runtime.emptyRun(),
    execution: { id, session_key: "" },
    path: instancePath,
    traversals,
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

const context = { execution: { id: "exec_gate", session_key: "" }, signal: new AbortController().signal, node: "fan" };

/** One route, with a `run` the caller supplies. */
function route(fields) {
  return {
    target: "agent.worker",
    maxConcurrency: 8,
    detach: false,
    itemShape: "any",
    input: (roots) => runtime.toJson(runtime.evaluate("item", roots)),
    writes: [{ field: "result", channel: "results", reduce: "append" }],
    ...fields,
  };
}

/** A map descriptor over `state.items`, with the routes the caller supplies. */
function descriptor(fields) {
  return {
    node: "fan",
    as: "item",
    source: { path: "state.items", shape: "any" },
    maxConcurrency: 8,
    onItemError: "fail",
    routes: [route({})],
    ...fields,
  };
}

/**
 * A map node as `runtime.runNode` sees one: the descriptor a compiled `graph.ts`
 * emits for a `map:`, and the graph state it is called against.
 *
 * The input phase and the activity are exactly what `codegen::graph` writes —
 * `runtime.mapPlan(map, view)` and `runtime.runMap(map, plan, context)` — so
 * what runs here is the seam a real node runs through, policy and all.
 */
function mapNode(map, items, { policy = {}, id = "exec_probe" } = {}) {
  const state = {
    items,
    $run: { ...runtime.emptyRun(), execution: { id, session_key: "" } },
  };
  const descriptor = {
    flow: "flow.probe",
    node: map.node,
    policy,
    shapes: {
      input: { properties: {} },
      state: { properties: { items: { items: "any" } } },
      output: "any",
    },
    input: (_roots, view) => runtime.mapPlan(map, view),
    run: async (input, context) => runtime.runMap(map, input, context),
    writes: [],
    edges: [{ to: "next" }],
  };
  return { descriptor, state };
}

/** The one trace entry a `runNode` answer carries, as a plain object. */
function entryOf(command) {
  return command.update.$run.trace[0];
}

const observed = {};

// --- Ordering, under a completion order that is the reverse of the source's ---
{
  const map = descriptor({
    routes: [
      route({
        run: async (input) => {
          // Item 0 waits longest, item 2 not at all.
          await sleep((3 - Number(input.at)) * 60);
          return { output: { result: `done-${input.at}` } };
        },
      }),
    ],
  });
  const view = viewOf([{ at: 0 }, { at: 1 }, { at: 2 }]);
  const answer = await runtime.runMap(map, runtime.mapPlan(map, view), context);
  observed.orderedByIndex = answer.channels;
  observed.orderedDispatches = answer.dispatches.map((record) => record.index);
}

// --- The bound: the node's own, and a route that tightens it ----------------
{
  let live = 0;
  let peak = 0;
  const watched = (fields) =>
    route({
      ...fields,
      run: async (input) => {
        live += 1;
        peak = Math.max(peak, live);
        await sleep(25);
        live -= 1;
        return { output: { result: String(input.at) } };
      },
    });

  const wide = descriptor({ maxConcurrency: 2, routes: [watched({})] });
  const items = [0, 1, 2, 3, 4, 5].map((at) => ({ at }));
  await runtime.runMap(wide, runtime.mapPlan(wide, viewOf(items)), context);
  observed.nodeBound = peak;

  live = 0;
  peak = 0;
  let tight = 0;
  let tightPeak = 0;
  const routed = descriptor({
    maxConcurrency: 4,
    routeBy: "kind",
    routes: [
      watched({ tag: "wide", itemShape: "any" }),
      route({
        tag: "narrow",
        maxConcurrency: 1,
        run: async (input) => {
          tight += 1;
          tightPeak = Math.max(tightPeak, tight);
          live += 1;
          peak = Math.max(peak, live);
          await sleep(25);
          live -= 1;
          tight -= 1;
          return { output: { result: String(input.at) } };
        },
      }),
    ],
  });
  const mixed = [0, 1, 2, 3, 4, 5].map((at) => ({ at, kind: at % 2 === 0 ? "wide" : "narrow" }));
  await runtime.runMap(routed, runtime.mapPlan(routed, viewOf(mixed)), context);
  observed.routedNodeBound = peak;
  observed.tightenedRouteBound = tightPeak;
}

// --- `on_item_error`: skip, a bounded retry, and which item a `fail` names ---
{
  const failing = (at) => at === 1 || at === 3;
  const map = descriptor({
    onItemError: "skip",
    routes: [
      route({
        run: async (input) => {
          if (failing(input.at)) throw new Error(`item ${input.at} refused`);
          return { output: { result: `done-${input.at}` } };
        },
      }),
    ],
  });
  const items = [0, 1, 2, 3].map((at) => ({ at }));
  const answer = await runtime.runMap(map, runtime.mapPlan(map, viewOf(items)), context);
  observed.skipped = answer.dispatches.map((record) => [record.index, record.outcome]);
  observed.skippedChannels = answer.channels;

  // The same items under `fail`, where the *lowest-indexed* failure is the one
  // reported — item 3 fails first in time, and item 1 is what the error names.
  const strict = descriptor({
    routes: [
      route({
        run: async (input) => {
          if (failing(input.at)) {
            await sleep(input.at === 1 ? 60 : 0);
            throw new Error(`item ${input.at} refused`);
          }
          return { output: { result: `done-${input.at}` } };
        },
      }),
    ],
  });
  try {
    await runtime.runMap(strict, runtime.mapPlan(strict, viewOf(items)), context);
    observed.failed = null;
  } catch (error) {
    observed.failed = {
      name: error.name,
      index: error.index,
      attempts: error.attempts,
      // What the fan-out did, carried out on the failure — the map node is about
      // to be resolved by its own `on_error:` and this is the only account of the
      // items that already ran (PRD 5.3, 5.6). The two that failed say `failed`
      // and not `skipped`: nothing skipped them.
      dispatches: error.dispatches.map((record) => [record.index, record.outcome]),
    };
  }

  // A parameterized retry: two attempts fail, the third answers.
  let tries = 0;
  const retried = descriptor({
    onItemError: { retry: { max: 2, backoffMs: 1, multiplier: 1, jitter: false } },
    routes: [
      route({
        run: async (input) => {
          tries += 1;
          if (tries < 3) throw new Error("not yet");
          return { output: { result: `done-${input.at}` } };
        },
      }),
    ],
  });
  const once = await runtime.runMap(retried, runtime.mapPlan(retried, viewOf([{ at: 0 }])), context);
  observed.retried = { attempts: once.dispatches[0].attempts, tries };

  // …and one that never answers: an exhausted retry resolves as `fail` does.
  const exhausted = descriptor({
    onItemError: { retry: { max: 1, backoffMs: 1, multiplier: 1, jitter: false } },
    routes: [route({ run: async () => { throw new Error("never"); } })],
  });
  try {
    await runtime.runMap(exhausted, runtime.mapPlan(exhausted, viewOf([{ at: 0 }])), context);
    observed.exhausted = null;
  } catch (error) {
    observed.exhausted = { name: error.name, attempts: error.attempts };
  }

  // …and the count when the node's **deadline** ends the loop mid-backoff: what
  // the item did, not what its policy allowed. One real attempt was made against
  // the target here, and a trace saying six would report a run that never
  // happened (grammar 9.2 bounds the node, and `context.signal` is its clock).
  const controller = new AbortController();
  const aborting = descriptor({
    onItemError: { retry: { max: 5, backoffMs: 200, multiplier: 1, jitter: false } },
    routes: [route({ run: async () => { throw new Error("never"); } })],
  });
  setTimeout(() => controller.abort(new Error("the node's budget ran out")), 50);
  try {
    await runtime.runMap(aborting, runtime.mapPlan(aborting, viewOf([{ at: 0 }])), {
      ...context,
      signal: controller.signal,
    });
    observed.abortedMidBackoff = null;
  } catch (error) {
    observed.abortedMidBackoff = {
      attempts: error.attempts,
      recorded: error.dispatches[0].attempts,
    };
  }
}

// --- `detach: true`: resolved at dispatch, and keyed (D94, grammar 9.4) -----
{
  const order = [];
  let release;
  const held = new Promise((resolve) => {
    release = resolve;
  });
  const map = descriptor({
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        run: async (input) => {
          order.push(`joined-${input.at}`);
          return { output: { result: String(input.at) } };
        },
      }),
    ],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: async () => {
        await held;
        order.push("detached");
        return { output: {} };
      },
    }),
  });
  // The map node is two frames deep: an outer `flow:` node, then this map.
  const view = viewOf([{ at: 0, kind: "joined" }, { at: 1, kind: "away" }], {
    path: ["outer/0"],
    traversals: { fan: 1 },
  });
  const answer = await runtime.runMap(map, runtime.mapPlan(map, view), context);
  order.push("joined");
  release();
  await sleep(20);
  observed.detachOrder = order;
  observed.detachRecords = answer.dispatches.map((record) => ({
    index: record.index,
    outcome: record.outcome,
    attempts: record.attempts,
    key: record.idempotencyKey,
  }));
  observed.detachChannels = answer.channels;
}

// --- Zero instances (grammar 8.6 rule 6) -----------------------------------
{
  const map = descriptor({ routes: [route({ run: async () => ({ output: { result: "x" } }) })] });
  const answer = await runtime.runMap(map, runtime.mapPlan(map, viewOf([])), context);
  observed.empty = { channels: answer.channels, dispatches: answer.dispatches };

  // …and a producer that was skipped, which is the other way rule 11 gets there.
  const fromProducer = descriptor({
    source: { path: "plan.output.tasks", producer: "plan", shape: { properties: { tasks: { items: "any" } } } },
  });
  const view = viewOf([]);
  const answer2 = await runtime.runMap(
    fromProducer,
    runtime.mapPlan(fromProducer, view),
    context,
  );
  observed.skippedProducer = { dispatches: answer2.dispatches.length };
}

// --- The reducers a map's batch goes through (grammar 7.6.4, 10.2) ---------
{
  const batch = new runtime.OrderedWrites(["b", "c"]);
  observed.reducers = {
    appendOne: runtime.appendReduce(["a"], "b"),
    appendBatch: runtime.appendReduce(["a"], batch),
    mergeOne: runtime.mergeReduce({ a: 1 }, { b: 2 }),
    mergeBatch: runtime.mergeReduce({ a: 1 }, new runtime.OrderedWrites([{ b: 2 }, { b: 3, c: 4 }])),
    setOne: runtime.setReduce("a", "b"),
    setBatch: runtime.setReduce("a", batch),
  };
}

// --- …and the channel they are called by, which is not always the reducer ---
//
// A reducer that folds a batch correctly is only half of it: LangGraph keeps the
// first update to an **empty** channel verbatim rather than calling the reducer
// with it, so a channel with no initial value never gets the chance. Every
// policy is driven here through a real `StateGraph` built exactly the way
// `codegen::state` builds one — `append` and `merge` at their identity elements,
// `last_wins` with a default and without one — over a batch of three.
{
  const channels = {
    notes: Annotation({
      reducer: (left, right) => runtime.appendReduce(left, right),
      default: () => [],
    }),
    totals: Annotation({
      reducer: (left, right) => runtime.mergeReduce(left, right),
      default: () => ({}),
    }),
    latest: Annotation({
      reducer: (left, right) => runtime.setReduce(left, right),
      default: () => "",
    }),
    // The shape with no `default:` (grammar 10.1, Decision D78) — the one a
    // batch cannot be handed to.
    winner: Annotation({ reducer: (left, right) => runtime.setReduce(left, right) }),
  };
  const write = (channel, reduce, values) =>
    runtime.orderedUpdate({ channel, reduce, values });
  const graph = new StateGraph(Annotation.Root(channels))
    .addNode("fan", () => ({
      notes: write("notes", "append", ["n-A", "n-B", "n-C"]),
      totals: write("totals", "merge", [{ who: "A" }, { who: "B", note: "b" }, { who: "C" }]),
      latest: write("latest", "set", ["l-A", "l-B", "l-C"]),
      winner: write("winner", "set", ["w-A", "w-B", "w-C"]),
    }))
    .addEdge("__start__", "fan")
    .addEdge("fan", "__end__")
    .compile();
  observed.batched = await graph.invoke({});
}

// --- What a detached delivery carries to its sink (grammar 9.4, PRD 5.6) ----
//
// The key is derived for every dispatch and recorded in the trace either way, so
// nothing about a run says whether it was ever *delivered*. These are the three
// surfaces grammar 9.4 fixes, one per binding kind a `tool.*` can be bound by.
{
  const site = { execution: { id: "exec_gate", session_key: "" }, path: ["fan/0/1"], idempotencyKey: "exec_gate/fan/0/1" };
  const keyed = runtime.delivering(context, site);

  const sent = [];
  const realFetch = globalThis.fetch;
  globalThis.fetch = async (url, init) => {
    sent.push({ url: String(url), headers: init.headers, body: init.body });
    return new Response("{}", { status: 200, headers: { "content-type": "application/json" } });
  };
  try {
    const binding = {
      method: "POST",
      url: ["https://sink.invalid/v1/enqueue"],
      headers: [],
      expectStatus: "2xx",
      decoding: { envelope: [], decoded: [], empty: true },
    };
    await runtime.runHttp(binding, { body: { text: "a1" } }, keyed);
    // …and the same request from a dispatch that is *not* detached.
    await runtime.runHttp(binding, { body: { text: "a1" } }, context);
    // A header the binding declares itself is the author's statement about this
    // wire, so it wins — the rule the emitted `content-type` already follows.
    await runtime.runHttp(
      { ...binding, headers: [{ name: "Idempotency-Key", value: ["mine"] }] },
      { body: { text: "a1" } },
      keyed,
    );
  } finally {
    globalThis.fetch = realFetch;
  }
  // The name reaches the wire as grammar 9.4 spells it, not merely as some
  // case-fold of it, so the read is by the exact key.
  observed.deliveredHeaders = sent.map((one) => one.headers["Idempotency-Key"] ?? null);

  // The subprocess form: the environment, because `args:` is the author's own
  // command line. `IDEMPOTENCY_KEY` is the name grammar 9.4 fixes, and the shell
  // reads it by that name and no other.
  const exec = {
    command: ["sh"],
    args: [["-c"], ["printf '%s' \"${IDEMPOTENCY_KEY-}\""]],
    env: [],
    expectExit: [0],
    decoding: { envelope: [], decoded: [], raw: "out", empty: false },
  };
  observed.deliveredEnv = {
    detached: (await runtime.runExec(exec, {}, keyed)).out,
    joined: (await runtime.runExec(exec, {}, context)).out,
    declared: (
      await runtime.runExec(
        { ...exec, env: [{ name: runtime.IDEMPOTENCY_ENV, value: ["mine"] }] },
        {},
        keyed,
      )
    ).out,
    // `environmentName` spells an input field by upper-casing it, so a target
    // declaring a field called `idempotency_key` names this same variable. No
    // composition gets here — the validator refuses a detached dispatch to a
    // sink declaring the slot (`check::maps`, grammar 9.4, Decision D66) — so
    // this is the order a direct call resolves it in, and the delivery wins:
    // a sink with nothing to dedupe on is the failure the key exists to
    // prevent.
    collided: (await runtime.runExec(exec, { idempotency_key: "an input field" }, keyed)).out,
    // …and the name is a plain one, so a variable this process happened to be
    // started with is not a key: a target reads one when the runtime delivered
    // it, and a joined dispatch beside it still reads nothing.
    ambient: await (async () => {
      process.env.IDEMPOTENCY_KEY = "from the process environment";
      try {
        return (await runtime.runExec(exec, {}, context)).out;
      } finally {
        delete process.env.IDEMPOTENCY_KEY;
      }
    })(),
  };

  // The host-function form: the `idempotency_key` field of the invocation
  // context, which is the surface grammar 9.4 fixes for a `function:` binding —
  // and the whole of what a host implementation has to dedupe on.
  const seen = [];
  runtime.registerFunction("sink", async (args, invocation) => {
    seen.push(invocation.idempotency_key ?? null);
    return { receipt: "ok" };
  });
  await runtime.callFunction("sink", { text: "a1" }, keyed);
  await runtime.callFunction("sink", { text: "a1" }, context);
  observed.deliveredContext = seen;
}

// --- The node-wide bound covers a detached delivery too ---------------------
//
// `max_concurrency` is an **admission** bound over every in-flight dispatch,
// detached included (grammar 8.6's key table, D28): a detached delivery waits
// for a node permit to start, so a map declaring 2 beside a detached route
// bounded at 2 still has at most 2 calls in flight rather than 4. What D94 keeps
// is the other half — the join never waits on the outcome — which the section
// above pins by other means.
//
// Every dispatch here is *watched*, so the peak is over the joined and the
// detached together, and half of the six items take the detached route.
{
  let live = 0;
  let peak = 0;
  const watched = async () => {
    live += 1;
    peak = Math.max(peak, live);
    await sleep(40);
    live -= 1;
    return { output: {} };
  };
  const map = descriptor({
    maxConcurrency: 2,
    routeBy: "kind",
    routes: [route({ tag: "joined", run: watched })],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      maxConcurrency: 2,
      writes: [],
      run: watched,
    }),
  });
  const items = [0, 1, 2, 3, 4, 5].map((at) => ({ at, kind: at % 2 === 0 ? "joined" : "away" }));
  await runtime.runMap(map, runtime.mapPlan(map, viewOf(items)), context);
  // The detached deliveries can still be in flight when the join returns — that
  // is what the section above pins — so the peak is read after they have all
  // finished, because every one of them is what this bound is over.
  await sleep(400);

  // The hazard admission introduces, and the answer to it: at a bound of 1 a
  // detached delivery cannot start until the joined instance ahead of it has
  // released the permit, which is after the map node has already returned. It
  // still runs. A delivery that a bound merely *delayed* past the join would be
  // a lost message if it were dropped there.
  const order = [];
  const queued = descriptor({
    maxConcurrency: 1,
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        run: async () => {
          await sleep(40);
          order.push("joined");
          return { output: { result: "0" } };
        },
      }),
    ],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: async () => {
        // Long enough that the map node's own return is not racing it inside one
        // microtask queue: the join is finished, and this has not started.
        await sleep(40);
        order.push("delivered");
        return { output: {} };
      },
    }),
  });
  const pair = [{ at: 0, kind: "joined" }, { at: 1, kind: "away" }];
  await runtime.runMap(queued, runtime.mapPlan(queued, viewOf(pair)), context);
  order.push("returned");
  await sleep(100);

  // …and the same two items the other way round, which is the direction the
  // bound and rule 7 can actually contradict each other in. Grammar 8.6 rule 7
  // says nothing a detached dispatch does can **delay** the enclosing flow
  // instance; a delivery that took the node's only permit at index 0 and held it
  // until it settled would make the joined instance at index 1 — and so the map
  // node's own join — wait out the sink. So a detached dispatch is not issued
  // until this call's joined instances are admitted, and the join still returns
  // before the delivery lands whichever order the items are in. Reverse this
  // fixture's indices with the permit taken eagerly and `["delivered", "joined",
  // "returned"]` is what comes back.
  const reversed = [];
  const ahead = descriptor({
    maxConcurrency: 1,
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        run: async () => {
          reversed.push("joined");
          return { output: { result: "0" } };
        },
      }),
    ],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: async () => {
        // Long enough that a delivery which *had* got in front of the join would
        // be unmistakable rather than a microtask-ordering coin flip.
        await sleep(40);
        reversed.push("delivered");
        return { output: {} };
      },
    }),
  });
  const detachedFirst = [{ at: 0, kind: "away" }, { at: 1, kind: "joined" }];
  await runtime.runMap(
    ahead,
    runtime.mapPlan(ahead, viewOf(detachedFirst, { id: "exec_ahead" })),
    context,
  );
  reversed.push("returned");
  await sleep(200);

  // The same direction taken to its end: a sink that never answers. Holding a
  // permit until the delivery settled did not merely delay the join here, it
  // ended it — `Promise.all` never resolved, the map node never completed, and a
  // map with no `timeout:` (grammar 9.3 level 4's built-in, and what
  // `every-schema-form` emits) blocked its flow instance for ever on a dispatch
  // rule 7 defines the join as not waiting for. Raced against a budget many
  // times the join's real cost, so a pass means the join did not wait for the
  // sink rather than that the sink was quick.
  const never = descriptor({
    node: "never",
    maxConcurrency: 1,
    routeBy: "kind",
    routes: [route({ tag: "joined", run: async () => ({ output: { result: "0" } }) })],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: () => new Promise(() => {}),
    }),
  });
  // The budget's timer is cleared rather than left to expire: this fixture ends
  // by draining the event loop, and a pending 2s timer would hold it open. The
  // delivery itself never settles and holds no handle, so it does not.
  let overdue;
  const budget = new Promise((resolve) => {
    overdue = setTimeout(() => resolve("the join is still waiting on the delivery"), 2000);
  });
  const joinedBehind = await Promise.race([
    runtime
      .runMap(never, runtime.mapPlan(never, viewOf(detachedFirst, { id: "exec_never" })), context)
      .then(() => "joined-returned"),
    budget,
  ]);
  clearTimeout(overdue);

  observed.detachedBound = {
    declared: 2,
    peak,
    queuedThenDelivered: order,
    aheadOfJoined: reversed,
    joinedBehindAHangingSink: joinedBehind,
  };
}

// --- A map node under its own `timeout:` (grammar 9.2, 8.6 rules 7, 9) ------
//
// A `map` takes a node policy like any other node, and grammar 9.3 level 3 puts
// one on every node a composition declares. Two of the guarantees above only
// mean anything once that policy is in play, and neither is reachable from a
// bare `runMap` call:
//
//   * a detached delivery still **queued for a permit** when the budget runs out
//     is delivered anyway. Its signal is its own, not the node's, so it does not
//     start against an already-aborted one and vanish into the catch that keeps
//     it from failing the flow — a message that is recorded as `detached` and
//     never sent is the lost message the key exists to prevent (rule 7, 5.6);
//   * the node still says **what it dispatched**. A deadline is raced, so the
//     map's promise is abandoned and neither its answer nor an `ItemFailure`
//     arrives; the record is the only account of effects that already happened.
{
  const attempted = [];
  const delivered = [];
  /** A sink that reports the difference between being started and being sent. */
  const sink = (fields) =>
    route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: async (_input, context, site) => {
        attempted.push(site.idempotencyKey);
        // What `fetch` and `spawn` do with a signal that has already aborted,
        // and therefore what `runHttp` and `runExec` would do here.
        if (context.signal.aborted) throw new Error("the delivery started aborted");
        delivered.push(site.idempotencyKey);
        return { output: {} };
      },
      ...fields,
    });

  // One joined instance holds the only permit and honours the node's signal; the
  // detached delivery behind it is still in the queue when the budget expires.
  const queued = descriptor({
    node: "queued",
    maxConcurrency: 1,
    onItemError: "skip",
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        writes: [],
        run: async (_input, context) => {
          await naps(400, context.signal);
          return { output: {} };
        },
      }),
    ],
    fallback: sink({}),
  });
  const held = mapNode(queued, [{ kind: "joined" }, { kind: "away" }], {
    policy: { timeoutMs: 80, onError: "skip" },
  });
  const answer = await runtime.runNode(held.descriptor, held.state);
  const timedOut = entryOf(answer);
  // The delivery has not started yet — that is the whole point — so this waits
  // for the permit its predecessor gives up when the deadline aborts it.
  await sleep(200);
  observed.deadlineWhileQueued = {
    outcome: timedOut.outcome,
    timedOut: String(timedOut.error).includes("timed out"),
    attempted,
    delivered,
  };

  // …and the record. Item 0 completes and item 1 is detached before the budget
  // runs out; item 2 is still in flight when it does, so it has no outcome to
  // report and the entry's own error is what accounts for it.
  const partial = descriptor({
    node: "partial",
    maxConcurrency: 2,
    onItemError: "skip",
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        writes: [],
        run: async (input, context) => {
          await naps(input.at === 0 ? 10 : 400, context.signal);
          return { output: {} };
        },
      }),
    ],
    fallback: sink({ run: async () => ({ output: {} }) }),
  });
  const cut = mapNode(
    partial,
    [
      { at: 0, kind: "joined" },
      { at: 1, kind: "away" },
      { at: 2, kind: "joined" },
    ],
    { policy: { timeoutMs: 90, onError: "skip" } },
  );
  const entry = entryOf(await runtime.runNode(cut.descriptor, cut.state));
  await sleep(200);
  observed.deadlineKeepsTheRecord = {
    outcome: entry.outcome,
    dispatches: (entry.dispatches ?? null)?.map((record) => [record.index, record.outcome]) ?? null,
    keys: (entry.dispatches ?? []).map((record) => record.idempotencyKey),
  };

  // …and the other half of the same rule: the plan is read off a **map**, and
  // nothing else. The block above is the only path that recovers records from a
  // node's input rather than from its answer or its failure, and `runNode`
  // reaches it for *every* node that fails — so what it recognises a plan by
  // decides whether a node that is not a map can be made to file one.
  // `instances` and `records` are field names grammar 2.1 allows, so a
  // composition can declare an `input:` carrying both arrays; `docs/trace.md`
  // §3 says only a `map` node's entry has `dispatches` and §5 says the elements
  // are `DispatchRecord`s, and the composition's own data is neither.
  const impostor = {
    flow: "flow.probe",
    node: "impostor",
    policy: { onError: "skip" },
    shapes: { input: { properties: {} }, state: { properties: {} }, output: "any" },
    // Shaped exactly like a `MapPlan`, down to the `index` the sort reads.
    input: () => ({
      instances: [{ index: 0 }],
      records: [{ index: 0, route: "$default", target: "agent.worker", outcome: "completed" }],
      admission: "exec_impostor/impostor",
    }),
    run: async () => {
      throw new Error("the activity failed");
    },
    writes: [],
    edges: [{ to: "next" }],
  };
  const filed = entryOf(
    await runtime.runNode(impostor, {
      $run: { ...runtime.emptyRun(), execution: { id: "exec_impostor", session_key: "" } },
    }),
  );
  observed.aPlanShapedInputIsNotAPlan = {
    outcome: filed.outcome,
    dispatches: filed.dispatches ?? null,
  };
}

// --- The bound belongs to the node, not to the call (grammar 8.6's key table)
//
// A detached delivery outlives the call that issued it (D94), so a bound counted
// inside one call is not a bound at all: the next execution of the same node —
// a second traversal of a bounded cycle, or the node's own `retry:` — would
// admit a fresh set of permits beside deliveries that are still in flight, and a
// map declaring 1 would reach 2. Both routes to a second execution are driven.
{
  let live = 0;
  let peak = 0;
  const watched = async () => {
    live += 1;
    peak = Math.max(peak, live);
    await sleep(120);
    live -= 1;
    return { output: {} };
  };

  const cycled = descriptor({
    node: "cycled",
    maxConcurrency: 1,
    routes: [route({ detach: true, target: "tool.sink", writes: [], run: watched })],
  });
  const items = [{ at: 0 }];
  // Two traversals of one node in one flow instance. The first returns at once —
  // its only dispatch is detached — so the second is planned while the first's
  // delivery is still running.
  const options = { id: "exec_cycle" };
  await runtime.runMap(cycled, runtime.mapPlan(cycled, viewOf(items, options)), context);
  await runtime.runMap(
    cycled,
    runtime.mapPlan(cycled, viewOf(items, { ...options, traversals: { cycled: 1 } })),
    context,
  );
  await sleep(400);
  observed.admissionAcrossTraversals = { declared: 1, peak };

  live = 0;
  peak = 0;
  // …and the node's own `retry:`, which re-executes the whole fan-out. Attempt
  // 1's joined item fails immediately and its detached delivery runs on; attempt
  // 2 must wait for that delivery's permit before it can dispatch anything.
  const retried = descriptor({
    node: "retried",
    maxConcurrency: 1,
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        writes: [],
        run: async () => {
          throw new Error("item refused");
        },
      }),
    ],
    fallback: route({
      tag: "$default",
      detach: true,
      target: "tool.sink",
      writes: [],
      run: watched,
    }),
  });
  const node = mapNode(retried, [{ kind: "joined" }, { kind: "away" }], {
    policy: {
      retry: { max: 1, backoffMs: 1, multiplier: 1, jitter: false },
      onError: "skip",
    },
    id: "exec_retry",
  });
  const absorbed = entryOf(await runtime.runNode(node.descriptor, node.state));
  await sleep(400);
  observed.admissionAcrossRetries = {
    declared: 1,
    peak,
    attempts: absorbed.attempts,
    outcome: absorbed.outcome,
  };
}

// --- …and that bound must not queue a join behind a delivery (rule 7) ------
//
// The two facts above are in tension. The permits belong to the node, so they
// outlive the call — and a *later* execution knows nothing of the deliveries an
// earlier one left waiting for one. Rule 7 says nothing a detached dispatch does
// can delay the enclosing flow instance, and a second traversal whose joined
// instance sat behind two queued deliveries is delayed by exactly that. Issuing
// deliveries after the join is admitted answers it inside one call; across calls
// only the queue can, so a joined waiter is served before a detached one.
//
// Reaching the node's queue takes **two** detached routes. A route gate is
// `min(route, map)`, so two deliveries down one route saturate that route's gate
// first and queue there, where no joined instance of another route is waiting —
// the ordering only becomes observable when a delivery's own route gate is free
// and the node's is not.
{
  const order = [];
  const id = "exec_span";
  const sink = (tag) =>
    route({
      tag,
      detach: true,
      target: "tool.sink",
      writes: [],
      run: async (input) => {
        await sleep(60);
        order.push(`delivered-${input.at}`);
        return { output: {} };
      },
    });
  const spanning = (joinedRun) =>
    descriptor({
      node: "spanning",
      maxConcurrency: 1,
      routeBy: "kind",
      routes: [route({ tag: "joined", writes: [], run: joinedRun }), sink("a")],
      fallback: sink("$default"),
    });

  // Execution 1: delivery 0 takes the node's only permit, delivery 1 finds its
  // own route gate free and queues on the node's. The call returns at once —
  // both its dispatches are detached, so it has no join to wait for.
  const first = spanning(async () => ({ output: {} }));
  const away = [{ at: 0, kind: "a" }, { at: 1, kind: "b" }];
  await runtime.runMap(first, runtime.mapPlan(first, viewOf(away, { id })), context);
  await sleep(10);

  // Execution 2 of that same node, with a joined instance. It is served the
  // permit delivery 0 releases, ahead of delivery 1 which has been queued for it
  // longer: a join waits out a delivery already *running*, and nothing more.
  // Under a first-come queue this reads `delivered-1` before `joined`, and the
  // second traversal takes both sleeps rather than one.
  const second = spanning(async () => {
    order.push("joined");
    return { output: {} };
  });
  await runtime.runMap(
    second,
    runtime.mapPlan(second, viewOf([{ at: 2, kind: "joined" }], { id })),
    context,
  );
  order.push("returned");
  await sleep(400);
  observed.joinAheadOfAQueuedDelivery = order;
}

// --- A dispatched `flow.*` that failed keeps its own trace (grammar 8.5) ----
//
// `DispatchRecord.inner` is the instance's trace, and the case it exists for is
// the one where the caller learns nothing else: under `on_item_error: skip` the
// run carries on and succeeds, so every guard, budget and attempt inside the
// boundary is either on this record or nowhere (PRD 5.3).
{
  const innerTrace = [
    {
      step: 1,
      flow: "flow.worker",
      node: "work",
      traversal: 0,
      outcome: "failed",
      attempts: 2,
      error: "Error: boom",
    },
  ];
  const map = descriptor({
    node: "sub",
    onItemError: "skip",
    routes: [
      route({
        target: "flow.worker",
        writes: [],
        run: async () => {
          throw new runtime.SubflowFailure(
            "flow.worker",
            "did not run to quiescence",
            innerTrace,
            new Error("boom"),
          );
        },
      }),
    ],
  });
  const answer = await runtime.runMap(
    map,
    runtime.mapPlan(map, viewOf([{ at: 0 }], { id: "exec_sub" })),
    context,
  );
  const record = answer.dispatches[0];
  observed.failedSubflowRecord = {
    outcome: record.outcome,
    inner: record.inner ?? null,
    error: record.error,
  };
}

// --- A subflow instance is on the instantiating node's clock (grammar 9.2) --
//
// A subgraph is the one activity a `timeout:` can really **stop**. Everything
// else a node runs is cooperative (`fetch`, `spawn`) or uncancellable (a host
// function), so `runActivity` races the budget and leaves the work to finish
// into a discarded value — but an instance nothing aborted is a whole graph, and
// it would carry on to quiescence after the node that started it had already
// failed: running every node it had left, issuing every effect those were going
// to issue, and holding the map node's admission permit for the whole of it.
//
// Both halves are driven, because the fix is only right if it stops at the
// boundary D94 draws. A `flow:` node's instance stops; a **detached** dispatch's
// runs on, because its context carries the signal nothing aborts.
{
  /**
   * A two-node compiled subgraph that reports which of its nodes ran, and the
   * `runtime.SubflowBinding` a `flow:` node reaches it through — exactly the
   * shape `codegen::graph` emits, `outputKeys` aside.
   */
  const subflow = (effects, firstNodeMs) => {
    const compiled = new StateGraph(
      Annotation.Root({
        $run: Annotation({ reducer: runtime.mergeRun, default: runtime.emptyRun }),
        note: Annotation({
          reducer: (left, right) => runtime.setReduce(left, right),
          default: () => "",
        }),
      }),
    )
      .addNode("one", async () => {
        // Deliberately deaf to any signal: what is asserted is that the
        // *instance* stopped, not that this activity noticed.
        await sleep(firstNodeMs);
        effects.push("one");
        return { note: "one" };
      })
      .addNode("two", async () => {
        effects.push("two");
        return { note: "two" };
      })
      .addEdge("__start__", "one")
      .addEdge("one", "two")
      .addEdge("two", "__end__")
      .compile();
    return {
      address: "flow.worker",
      outputs: ["note"],
      recursionLimit: 25,
      stream: (initial, options) => compiled.stream(initial, { ...options, streamMode: "values" }),
    };
  };

  // A `flow:` node whose budget runs out while the instance's first node is
  // still working. Its `run` is what `codegen::graph` writes for `flow:`.
  const stopped = [];
  const held = subflow(stopped, 300);
  const outer = {
    flow: "flow.probe",
    node: "sub",
    policy: { timeoutMs: 120, onError: "skip" },
    shapes: { input: { properties: {} }, state: { properties: {} }, output: "any" },
    input: () => ({}),
    run: async (input, context, view) =>
      runtime.runSubflow(held, {
        inputs: input,
        execution: view.run.execution,
        path: runtime.instancePath(view, "sub"),
        signal: context.signal,
      }),
    writes: [],
    edges: [{ to: "next" }],
  };
  const entry = entryOf(
    await runtime.runNode(outer, {
      $run: { ...runtime.emptyRun(), execution: { id: "exec_clock", session_key: "" } },
    }),
  );
  const atReturn = [...stopped];

  // …and the counterpart. A detached dispatch to a `flow.*` is off the node's
  // clock (D94): its delivery context carries the signal nothing aborts, so the
  // instance runs to quiescence after the map node has long since given up. A
  // joined item beside it is what makes the node's budget expire at all — a
  // detached-only map returns the moment it has issued, and a deadline nothing
  // reached would prove nothing about what survives one.
  const detached = [];
  const sink = subflow(detached, 200);
  const away = descriptor({
    node: "away",
    maxConcurrency: 2,
    onItemError: "skip",
    routeBy: "kind",
    routes: [
      route({
        tag: "joined",
        writes: [],
        run: async (_input, context) => {
          await naps(400, context.signal);
          return { output: {} };
        },
      }),
      route({
        tag: "away",
        detach: true,
        target: "flow.worker",
        writes: [],
        run: async (input, context, site) =>
          runtime.runSubflow(sink, {
            inputs: input,
            execution: site.execution,
            path: site.path,
            signal: context.signal,
          }),
      }),
    ],
  });
  const fired = mapNode(away, [{ kind: "away" }, { kind: "joined" }], {
    policy: { timeoutMs: 60, onError: "skip" },
    id: "exec_clock_detached",
  });
  const dispatch = entryOf(await runtime.runNode(fired.descriptor, fired.state));

  // Long enough for every node of both instances to have run had nothing
  // stopped them.
  await sleep(600);
  observed.subflowOnTheNodesClock = {
    outcome: entry.outcome,
    timedOut: String(entry.error).includes("timed out"),
    // Empty at the node's deadline — the first inner node is still working —
    // and holding only that node once it finishes. `two` is the one that says
    // whether the instance stopped: it had nothing to wait for.
    atReturn,
    effects: stopped,
    detached: {
      outcome: dispatch.outcome,
      effects: detached,
    },
  };
}

// --- A variant tagged `default` is not the catch-all (grammar 8.6 rules 2, 4)
//
// Routes are keyed by variant tag and `default:` is a map-block key beside them,
// so a union may declare a variant called `default` *and* a catch-all. The
// runtime resolves them correctly — named routes are searched first — and the
// record has to keep them apart too, because two routes may share a target.
{
  const map = descriptor({
    node: "collide",
    routeBy: "kind",
    routes: [
      route({
        tag: "default",
        target: "agent.named",
        writes: [],
        run: async () => ({ output: {} }),
      }),
    ],
    fallback: route({
      tag: "$default",
      target: "agent.catchall",
      writes: [],
      run: async () => ({ output: {} }),
    }),
  });
  const answer = await runtime.runMap(
    map,
    runtime.mapPlan(map, viewOf([{ kind: "default" }, { kind: "other" }], { id: "exec_tags" })),
    context,
  );
  observed.defaultTagCollision = answer.dispatches.map((record) => [
    record.index,
    record.route,
    record.target,
  ]);
}

// --- Grammar 9.3 level 1, and D79's outermost-wins --------------------------
{
  observed.policy = {
    outermost: runtime.instancePolicy({ timeoutMs: 30_000 }, { timeoutMs: 10_000 }),
    filledIn: runtime.instancePolicy({ timeoutMs: 30_000 }, { onError: "skip" }),
    none: runtime.instancePolicy(undefined, undefined) ?? null,
    // A `human` node takes neither `timeout` nor `retry` from any level (D102).
    exempt: runtime.effectivePolicy(
      { onError: "fail" },
      true,
      { timeoutMs: 10_000, retry: { max: 1, backoffMs: 1, multiplier: 1, jitter: false }, onError: "skip" },
    ),
    plain: runtime.effectivePolicy({ onError: "fail", timeoutMs: 1 }, false, { timeoutMs: 10_000 }),
  };
}

process.stdout.write(JSON.stringify(observed));
