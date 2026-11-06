// One of the two writer **processes** `store-contract.mjs` races over one
// authored idempotency key (PRD resolved q73).
//
// An authored key is a business key two **executions** share — two runs of one
// webhook handler writing `event/<id>` under `idempotency_key: "input.event_id"`
// — and what the ruling promises is that the pair applies once and the second
// is answered with the first's whole answer. Two executions are two processes
// as often as not (a `serve` and a worker, two replicas behind one queue), so
// the parent spawns two of these, each with an execution id of its own and its
// own SQLite handle or its own server connection, and both start every round
// at one agreed instant so the two writes really overlap — and, every other
// round, from two **session partitions** of one store, which share the key
// because an authored key is a store's and not a partition's.
//
// Every op goes through `runStoreOp`, the entry point a store-op node calls,
// with the site key `src/graph.ts` builds for a node that wrote
// `idempotency_key:` (`stores.authoredKey`), so what is raced is the module a
// deployment runs.
//
// Usage: bun store-authored-race.mjs <project> <provider> <data directory> <run> <role> <start at, ms>
// Prints one JSON object: every answer this process was given.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data, run, role, startAt] = process.argv;
if (startAt === undefined) {
  throw new Error(
    "usage: bun store-authored-race.mjs <project> <provider> <data directory> <run> <role> <start at>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/** How many authored writes this process issues. The parent reads the same number. */
export const ROUNDS = 20;

/** One binding, on the backend the parent is driving. */
const kv = (name, scope) => ({
  address: `store.${run}_${name}`,
  name: `${run}_${name}`,
  kind: "kv",
  scope,
  metadata: false,
  value: z.object({ owner: z.string() }).strict(),
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: "AGENT_COMPOSE_STORE_URL" }),
  },
});

const events = kv("authored_race", "global");
const sessions = kv("authored_race_sessions", "session");

/** This process's execution — its own id, as a second run of the handler has. */
const context = (session = "") => ({
  execution: { id: `exec_${run}_authored_race_${role}`, session_key: session },
  signal: new AbortController().signal,
  node: "record",
  storeRecords: [],
});
const run_ = context();
const sessionRun = context(`session-${role}`);
const authored = (store, key) => ({ via: "node", idempotencyKey: stores.authoredKey(store.name, key) });

// The schema is created before the clock, so the race is over the rows.
await stores.runStoreOp(events, "get", { key: "warm" }, run_, { via: "node" });
await stores.runStoreOp(sessions, "get", { key: "warm" }, sessionRun, { via: "node" });

/** How long one round has before the next begins (see `store-race.mjs`). */
const ROUND_MS = 60;

/** Sleep until `at`, a wall-clock instant both processes share. */
const until = async (at) => {
  const wait = at - Date.now();
  if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
};

const answer = { global: [], sessions: [] };
for (let round = 0; round < ROUNDS; round += 1) {
  await until(Number(startAt) + round * ROUND_MS);
  // The same business key from both processes, each in its own execution —
  // and an unconditional `set`, so nothing but the ledger can make one of the
  // two land alone.
  answer.global.push(
    await stores.runStoreOp(
      events,
      "set",
      { key: `event/${round}`, value: { owner: role }, idempotencyKey: `evt-${round}` },
      run_,
      authored(events, `evt-${round}`),
    ),
  );
  // …and from two session partitions of one store: two partitions' sequence
  // locks, one store's authored key.
  answer.sessions.push(
    await stores.runStoreOp(
      sessions,
      "set",
      { key: `event/${round}`, value: { owner: role }, idempotencyKey: `evt-${round}` },
      sessionRun,
      authored(sessions, `evt-${round}`),
    ),
  );
}
answer.records = [...run_.storeRecords, ...sessionRun.storeRecords].map((record) => ({
  idempotencyKey: record.idempotencyKey,
  deduped: record.deduped,
}));
await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
