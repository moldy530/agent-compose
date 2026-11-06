// One of the **processes** `store-contract.mjs` races `kv` batches with (PRD
// resolved q71): a writer issuing batches over keys another writer's batches
// name in the opposite order, or a reader asking for all of those keys at once
// while they do.
//
// A batch is one transaction that locks its partition's version sequence once
// and then each of its keys in entry order (`#batch` in `src/stores.ts`). Two
// batches naming the same keys in opposite orders are what that order is for:
// without the sequence lock first, each would hold a key the other is about to
// reach for — a deadlock the server breaks by failing one of them — and both
// would read one `last` and hand out the same versions. With it, the second
// batch waits for the first to commit before it reads anything, and the two
// apply one after the other whichever keys they share. That is a statement
// about **processes**, each with its own server connection or its own SQLite
// handle, for `store-race.mjs`'s reason: two bindings in one process are still
// one event loop.
//
// Two stores, one per question:
//
//  * `batch_race`, `scope: global` — every batch of either writer writes the
//    same six keys, `a` to `e` and `z`, the `forward` writer in that order and
//    the `reverse` writer in the opposite one, and each deletes `z` and sets it
//    again, so the delete path takes its row lock in the race too. Every key a
//    batch sets holds that batch's tag, so a reader that finds two tags among
//    the six has read one batch half landed.
//  * `batch_race_session`, `scope: session` — each writer in a partition of
//    its own, inserting six keys no batch wrote before, in opposite orders:
//    first writes into adjacent partitions, which is where a server's gap
//    locks would have two batches that share no key wait on each other in a
//    circle.
//
// Every op goes through `runStoreOp`, the entry point a store-op node calls.
//
// Usage: bun store-batch-race.mjs <project> <provider> <data directory> <run> <role> <start at, ms> <done marker>
//   role: `forward` or `reverse` (a writer), or `reader`, which reads until
//   the parent writes the done marker.
// Prints one JSON object: every answer this process was given.

import fs from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data, run, role, startAt, done] = process.argv;
if (done === undefined) {
  throw new Error(
    "usage: bun store-batch-race.mjs <project> <provider> <data directory> <run> <role> <start at> <done marker>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/** How many batches each writer issues into each store. The parent reads the same number. */
const ROUNDS = 100;

/** The keys every `global` batch sets, beside `z`, which it deletes and sets again. */
const KEYS = ["a", "b", "c", "d", "e"];

/** What every key a batch sets holds: which writer, and which of its batches. */
const TAGGED = z.object({ by: z.string(), n: z.number().int() }).strict();

/** One binding, on the backend the parent is driving. */
const kv = (name, scope) => ({
  address: `store.${run}_${name}`,
  name: `${run}_${name}`,
  kind: "kv",
  scope,
  metadata: false,
  value: TAGGED,
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: "AGENT_COMPOSE_STORE_URL" }),
  },
});

const shared = kv("batch_race", "global");
const partitioned = kv("batch_race_session", "session");

const context = (session) => ({
  execution: { id: `exec_${run}_batch_race_${role}`, session_key: session },
  signal: new AbortController().signal,
  node: "batch_race",
  storeRecords: [],
});
const node = (key) => ({ via: "node", idempotencyKey: `${run}/batch_race/${role}/${key}` });

/**
 * How long one round has before the next begins.
 *
 * Every round starts at an instant both writers computed from the one they
 * were handed, so each round's two batches really are issued at once — two
 * loops left to run free drift apart — and the rest of the round is the
 * reader's: a SQLite file has one lock, and two writers taking it back to
 * back would leave a reader nothing to read until they were done. A round a
 * slow machine overruns starts the next at once, which is a race too.
 */
const ROUND_MS = 30;

/** Sleep until `at`, a wall-clock instant every process shares. */
const until = async (at) => {
  const wait = at - Date.now();
  if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
};

/** One op, its answer or the refusal it met — a failed batch is the finding, not a crash. */
const attempt = async (work) => {
  const started = Date.now();
  try {
    const answered = await work();
    return { ...answered, started, ended: Date.now() };
  } catch (error) {
    return { error: String(error?.message ?? error), started, ended: Date.now() };
  }
};

if (role === "reader") {
  // Opened before the clock, so the race is over the rows.
  const reading = context("");
  await stores.runStoreOp(shared, "get", { key: "warm" }, reading, { via: "node" });
  await until(Number(startAt));
  const reads = [];
  // Until the parent says both writers are done — and at least once after,
  // so the last read is of the state they left.
  for (;;) {
    const finished = fs.existsSync(done);
    reads.push(
      await attempt(() =>
        stores.runStoreOp(shared, "get_many", { keys: [...KEYS, "z"] }, reading, { via: "node" }),
      ),
    );
    if (finished) break;
    await new Promise((resolve) => setTimeout(resolve, 2));
  }
  await stores.releaseStores();
  process.stdout.write(JSON.stringify({ reads }));
} else {
  const tag = (n) => ({ by: role, n });
  const forward = role === "forward";
  const own = context(role);
  // Both stores' schemas are made before the clock, so the race is over the
  // rows and not over who creates the tables first — which
  // `store-contract.mjs` drives as its own case.
  await stores.runStoreOp(shared, "get", { key: "warm" }, own, { via: "node" });
  await stores.runStoreOp(partitioned, "get", { key: "warm" }, own, { via: "node" });

  const global_ = [];
  for (let round = 0; round < ROUNDS; round += 1) {
    await until(Number(startAt) + round * ROUND_MS);
    const sets = KEYS.map((key) => ({ op: "set", key, value: tag(round) }));
    const z_ = [
      { op: "delete", key: "z" },
      { op: "set", key: "z", value: tag(round) },
    ];
    const ops = forward ? [...sets, ...z_] : [...z_, ...sets.reverse()];
    global_.push(await attempt(() => stores.runStoreOp(shared, "batch", { ops }, own, node(`global/${round}`))));
  }

  const session = [];
  for (let round = 0; round < ROUNDS; round += 1) {
    await until(Number(startAt) + (ROUNDS + round) * ROUND_MS);
    const sets = [...KEYS, "z"].map((key) => ({ op: "set", key: `${round}-${key}`, value: tag(round) }));
    const ops = forward ? sets : sets.reverse();
    session.push(
      await attempt(() => stores.runStoreOp(partitioned, "batch", { ops }, own, node(`session/${round}`))),
    );
  }
  await stores.releaseStores();
  process.stdout.write(JSON.stringify({ global: global_, session }));
}
