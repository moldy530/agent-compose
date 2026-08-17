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

/** A `NodeView` over a `state.items` array, which is what `over` reads here. */
function viewOf(items, { path: instancePath = [], traversals = {} } = {}) {
  const shape = { properties: { items: { items: "any" } } };
  const run = {
    ...runtime.emptyRun(),
    execution: { id: "exec_gate", session_key: "" },
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
      tag: "default",
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
    // declaring a field called `idempotency_key` names this same variable. The
    // delivery wins that collision: grammar 9.4 says the key is never part of
    // the declared input schema, and a sink with nothing to dedupe on is the
    // failure the key exists to prevent.
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
      tag: "default",
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
      tag: "default",
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
  observed.detachedBound = { declared: 2, peak, queuedThenDelivered: order };
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
