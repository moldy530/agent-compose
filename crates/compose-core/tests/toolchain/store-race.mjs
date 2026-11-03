// One of the writer **processes** `store-contract.mjs` races against another
// over one store (PRD resolved q68).
//
// A case about "two writers" that runs both in one process proves less than it
// says: on the local backend one process's ops are one SQLite handle and run
// one at a time, and on a dialled one two bindings in one process are still one
// event loop. What PRD resolved q68 promises is about **processes** — two runs
// claiming one key with `if: absent` get exactly one `applied: true`, two
// concurrent appends keep both items — so the parent spawns two of these, each
// a process of its own with its own SQLite handle or its own server
// connection, and both start their work at one agreed instant so the writes
// really overlap.
//
// Every op goes through `runStoreOp`, the entry point a store-op node calls, so
// what is raced is the module a deployment runs.
//
// Usage: bun store-race.mjs <project> <provider> <data directory> <run> <role> <start at, ms>
// Prints one JSON object: every answer this process was given.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data, run, role, startAt] = process.argv;
if (startAt === undefined) {
  throw new Error(
    "usage: bun store-race.mjs <project> <provider> <data directory> <run> <role> <start at>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/** How many of each op this process issues. The parent reads the same number. */
export const ROUNDS = 20;

/** One binding, on the backend the parent is driving — its names, its schema. */
const kv = (name, value) => ({
  address: `store.${run}_${name}`,
  name: `${run}_${name}`,
  kind: "kv",
  scope: "global",
  metadata: false,
  value,
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: "AGENT_COMPOSE_STORE_URL" }),
  },
});

const claims = kv("race_claims", z.object({ owner: z.string() }).strict());
const lists = kv("race_lists", z.object({ items: z.array(z.string()) }).strict());
const counters = kv("race_counters", z.object({ count: z.number().int() }).strict());
const records = kv(
  "race_records",
  z.object({ a: z.number().int().optional(), b: z.number().int().optional() }).strict(),
);

const context = {
  execution: { id: `exec_${run}_race_${role}`, session_key: "" },
  signal: new AbortController().signal,
  node: "race",
  storeRecords: [],
};
const node = (key) => ({ via: "node", idempotencyKey: `${run}/race/${role}/${key}` });

// The schema is created before the clock, so the race is over the rows rather
// than over who creates the tables first — which `store-contract.mjs` drives
// as its own case.
await stores.runStoreOp(claims, "get", { key: "warm" }, context, { via: "node" });

/**
 * How long one round has before the next begins.
 *
 * Every round starts at an instant both processes computed from the one they
 * were handed, so each round's four writes really are issued at once by the two
 * — a pair of loops left to run free drifts apart within a few rounds, and one
 * process then wins every claim by being ahead rather than by winning a race.
 */
const ROUND_MS = 60;

/** Sleep until `at`, a wall-clock instant both processes share. */
const until = async (at) => {
  const wait = at - Date.now();
  if (wait > 0) await new Promise((resolve) => setTimeout(resolve, wait));
};

const answer = { claims: [], appends: [], incrs: [], merges: [] };
for (let round = 0; round < ROUNDS; round += 1) {
  await until(Number(startAt) + round * ROUND_MS);
  answer.claims.push(
    await stores.runStoreOp(
      claims,
      "set",
      { key: `claim-${round}`, value: { owner: role }, condition: "absent" },
      context,
      node(`claim/${round}`),
    ),
  );
  answer.appends.push(
    await stores.runStoreOp(
      lists,
      "append",
      { key: "shared", field: "items", value: `${role}-${round}` },
      context,
      node(`append/${round}`),
    ),
  );
  answer.incrs.push(
    await stores.runStoreOp(
      counters,
      "incr",
      { key: "shared", field: "count" },
      context,
      node(`incr/${round}`),
    ),
  );
  answer.merges.push(
    await stores.runStoreOp(
      records,
      "merge",
      { key: "shared", fields: { [role]: round } },
      context,
      node(`merge/${round}`),
    ),
  );
}
answer.records = context.storeRecords.length;
await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
