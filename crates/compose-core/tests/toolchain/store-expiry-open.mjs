// The **next process** to open a store, for the case in `store-contract.mjs`
// about the second half of PRD resolved q69's cleanup schedule: an expired key
// leaves storage at the next write to its partition *or the next process
// start*, whichever comes first.
//
// A process rather than a second binding in the parent, because what is under
// test is an open: on SQLite a process opens a store file once and keeps the
// handle, and on a server the parent's connection has already made its first
// op. This one's first op on the store is a **read** in one partition, which is
// the case that matters — a store nobody writes to again still sheds what
// expired, every partition of it, the first time a process reaches it.
//
// The clock it reads is the one the parent hands it, through the same
// `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS` every other case moves.
//
// Usage: bun store-expiry-open.mjs <project> <provider> <data directory> <binding json> <session>
// Prints one JSON object: what the read answered.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project, provider, data, binding, session] = process.argv;
if (session === undefined) {
  throw new Error(
    "usage: bun store-expiry-open.mjs <project> <provider> <data directory> <binding json> <session>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

const store = JSON.parse(binding);
if (store.backend?.provider !== provider) {
  throw new Error(`the binding names \`${store.backend?.provider}\` and the parent drives \`${provider}\``);
}
const context = {
  execution: { id: `exec_${store.name}_opener`, session_key: session },
  signal: new AbortController().signal,
  node: "opener",
  storeRecords: [],
};
const answer = await stores.runStoreOp(store, "get", { key: "k0" }, context, { via: "node" });
await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
