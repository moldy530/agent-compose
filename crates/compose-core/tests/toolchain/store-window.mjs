// The writer **process** that holds one `kv` write's read-modify-write window
// open while `store-contract.mjs` writes the same key (PRD resolved q68).
//
// `store-race.mjs` races two processes from one agreed instant, and on the
// local backend that proves less than it reads as: a write's transaction is a
// few synchronous microseconds, so two racers almost never overlap inside one,
// and a store that read its key outside the lock it writes under would pass
// those cases most of the time. This makes the window wide instead. The
// store's `value_schema` here is the emitted Zod with a refinement that holds
// the process for [`HOLD_MS`] — and a partial update holds its result to that
// schema between reading the key and writing it, inside the transaction every
// arm opens (`decideKvWrite` in `src/stores.ts`). So the window is open, with
// whatever lock the arm takes held, for as long as the refinement runs, and the
// refinement writes a marker file the moment it starts. The parent waits for
// the marker and then writes the key itself: a store that serializes the whole
// read-modify-write makes that write wait for this one's commit and build on
// it, and one that does not lets it land inside the window, where this write
// then overwrites it.
//
// Usage: bun store-window.mjs <project> <provider> <data directory> <run> <marker file>
// Prints one JSON object: the answer this process's merge was given.

import fs from "node:fs";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data, run, marker] = process.argv;
if (marker === undefined) {
  throw new Error("usage: bun store-window.mjs <project> <provider> <data directory> <run> <marker file>");
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/**
 * How long the window stays open once the marker is written.
 *
 * Long beside the few milliseconds the parent needs to notice the marker and
 * issue its write, and short beside every wait a correct arm makes that write
 * sit through: SQLite's `busy_timeout` is 5 s, and neither server times a row
 * lock out sooner. A parent that was slow to notice issues its write after this
 * commit, which a correct store and a broken one answer alike — so a slow
 * machine can make this case pass a broken store, and never fail a correct one.
 */
const HOLD_MS = 1_500;

/**
 * The window: synchronous, so this process does nothing else while it is open
 * — the lock its transaction took is held, and nothing it holds is released.
 */
const hold = () => {
  fs.writeFileSync(marker, "open");
  const until = Date.now() + HOLD_MS;
  while (Date.now() < until) {
    // Spinning is the point: an `await` here would be a turn of the event loop
    // in which this process could answer anything else.
  }
};

const store = {
  address: `store.${run}_window`,
  name: `${run}_window`,
  kind: "kv",
  scope: "global",
  metadata: false,
  value: z
    .object({ a: z.number().int().optional(), b: z.number().int().optional() })
    .strict()
    .superRefine(hold),
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: "AGENT_COMPOSE_STORE_URL" }),
  },
};

const context = {
  execution: { id: `exec_${run}_window`, session_key: "" },
  signal: new AbortController().signal,
  node: "window",
  storeRecords: [],
};

// The store is opened — its file or its connection, and the schema — before
// the window, so what the parent waits on is this write and not an open.
await stores.runStoreOp(store, "get", { key: "warm" }, context, { via: "node" });
const answer = await stores.runStoreOp(
  store,
  "merge",
  { key: "shared", fields: { a: 1 } },
  context,
  { via: "node", idempotencyKey: `${run}/window/a` },
);
await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
