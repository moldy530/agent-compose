// One of the two **processes** `store-contract.mjs` races against each other
// over one dialled store: a writer refreshing keys, and a process whose first
// op on the store, over each of several fresh connections, starts a
// store-wide sweep of what has expired (PRD resolved q69 (c)).
//
// What is under test is the two halves of the cleanup schedule meeting on one
// server. A write sweeps its own partition's expired rows inside its
// transaction; a connection's first op starts a sweep of the whole store; and
// both delete the same expired rows. Two `DELETE`s that reach one row through
// two different indexes lock it in two different orders, which the server
// answers by failing one of them as a deadlock — so the store has to keep them
// off each other, and a case that ran them one after the other would pass a
// store that does not. So the parent spawns one of each, both start at one
// agreed instant, and the parent then reads what every write left.
//
// The sweeping process reads a clock an hour and a minute ahead (the store's
// own `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS`, set by the parent), so every key
// the parent seeded with an hour's `ttl` is expired to it and live to the
// writer, which refreshes each one for two hours — past the sweeper's instant,
// so no key a write refreshed is one the sweep may take.
//
// Usage: bun store-sweep-race.mjs <project> <provider> <data directory> <binding json> <role> <start at, ms> <count>
//   role = write | sweep
// Prints one JSON object: how many writes applied, and every error an op raised.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project, provider, data, binding, role, startAt, count] = process.argv;
if (count === undefined) {
  throw new Error(
    "usage: bun store-sweep-race.mjs <project> <provider> <data directory> <binding json> <role> <start at> <count>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

const store = JSON.parse(binding);
if (store.backend?.provider !== provider) {
  throw new Error(`the binding names \`${store.backend?.provider}\` and the parent drives \`${provider}\``);
}

/** How many fresh connections the sweeping process opens — one sweep each. */
const CONNECTIONS = 8;

const HOUR = 3_600_000;
const context = {
  execution: { id: `exec_${store.name}_${role}`, session_key: "" },
  signal: new AbortController().signal,
  node: role,
  storeRecords: [],
};
const errors = [];
const answer = { applied: 0 };

await new Promise((resolve) => setTimeout(resolve, Math.max(0, Number(startAt) - Date.now())));

if (role === "write") {
  for (let index = 0; index < Number(count); index += 1) {
    try {
      const written = await stores.runStoreOp(
        store,
        "set",
        { key: `k-${String(index).padStart(5, "0")}`, value: { n: -index }, ttl: 2 * HOUR },
        context,
        { via: "node", idempotencyKey: `${context.execution.id}/refresh/${index}` },
      );
      if (written.applied === true) answer.applied += 1;
    } catch (error) {
      errors.push(String(error?.message ?? error));
    }
  }
} else {
  // Each binding names a variable of its own for the one server, which is how
  // one process holds several connections — and so starts several sweeps.
  for (let index = 0; index < CONNECTIONS; index += 1) {
    const variable = `AGENT_COMPOSE_SWEEP_URL_${index}`;
    process.env[variable] = process.env[store.backend.urlEnv];
    try {
      await stores.runStoreOp(
        { ...store, backend: { ...store.backend, urlEnv: variable } },
        "get",
        { key: "k-00000" },
        context,
        { via: "node" },
      );
    } catch (error) {
      errors.push(String(error?.message ?? error));
    }
  }
}

// Waits out every sweep this process started, which is the half of the race
// the sweeping process exists for.
await stores.releaseStores();
answer.errors = errors;
process.stdout.write(JSON.stringify(answer));
