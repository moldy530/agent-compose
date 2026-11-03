// The writer **process** `store-contract.mjs` kills in the middle of a `kv`
// `batch` — after its first entry has been written inside the batch's
// transaction and before the batch commits (PRD resolved q71).
//
// A batch applies its entries in order inside one transaction, each one
// written as it is reached, and holds every `set` entry's value to the store's
// `value_schema` as it reaches it (`batchStep` in `src/stores.ts`). So a
// schema whose refinement never returns on one entry's value is a process
// stopped *between two entry writes* with the transaction open: the first
// entry has landed in it, the second has not, and nothing has committed. The
// refinement writes a marker the moment it starts and then spins, and the
// parent kills this process with `SIGKILL` once it sees the marker — the
// crash the ruling's verification names, on whichever arm the parent drives:
// SQLite's file, its lock and its journal left as a killed process leaves
// them, or a server connection whose socket the kernel closes.
//
// Before the batch, this process seeds the key the batch's third entry
// deletes, in a write of its own that commits — so the parent finds, after the
// crash, one key that was there before the batch, at the version the seed took,
// and none of the batch.
//
// Usage: node store-batch-crash.mjs <project> <provider> <data directory> <store> <idempotency key> <marker file>
// Prints nothing it is not killed before printing; exits 3 if it never was.

import fs from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data, name, key, marker] = process.argv;
if (marker === undefined) {
  throw new Error(
    "usage: node store-batch-crash.mjs <project> <provider> <data directory> <store> <idempotency key> <marker file>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/**
 * How long the window stays open before this process gives up on being killed
 * — long past anything the parent needs to see the marker, and a bound so a
 * parent that died first leaves no process spinning for ever.
 */
const HOLD_MS = 60_000;

const store = {
  address: `store.${name}`,
  name,
  kind: "kv",
  scope: "global",
  metadata: false,
  value: z
    .object({ n: z.number().int(), hold: z.boolean().optional() })
    .strict()
    .superRefine((value) => {
      if (value.hold !== true) return;
      fs.writeFileSync(marker, "open");
      const until = Date.now() + HOLD_MS;
      while (Date.now() < until) {
        // Spinning is the point: the transaction stays open with the first
        // entry in it, and this process answers nothing until it is killed.
      }
    }),
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: "AGENT_COMPOSE_STORE_URL" }),
  },
};

const context = {
  execution: { id: `exec_${name}`, session_key: "" },
  signal: new AbortController().signal,
  node: "crash",
  storeRecords: [],
};

await stores.runStoreOp(
  store,
  "set",
  { key: "seeded", value: { n: 0 } },
  context,
  // A key of its own, outside the batch's prefix, so the ledger rows the
  // parent counts under the batch's key are the batch's alone.
  { via: "node", idempotencyKey: `seed/${key}` },
);
await stores.runStoreOp(
  store,
  "batch",
  {
    ops: [
      { op: "set", key: "first", value: { n: 1 } },
      { op: "set", key: "second", value: { n: 2, hold: true } },
      { op: "delete", key: "seeded" },
    ],
  },
  context,
  { via: "node", idempotencyKey: key },
);
process.exit(3);
