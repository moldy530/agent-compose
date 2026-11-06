// A keyed write made by a **process that is not the one that settles** the
// execution it belongs to, for the case in `store-contract.mjs` about PRD
// resolved q69 (d)'s "in every store partition the execution wrote".
//
// That is what a placement is: a placed node runs in a worker, under the hub's
// execution id, and writes a dialled store from there (grammar 14.1 rule 5,
// Decision D131) — and the hub, which settles the execution, never sees the
// write. So this process makes the write and exits, and the parent, which has
// not written the store under that execution, settles it and reads what the
// ledger kept. On the local backend it is the same file opened by a second
// process, which is what a resumed generation in a new process is.
//
// Usage: bun store-placed-write.mjs <project> <provider> <data directory> <binding json> <execution> <idempotency key>
// Prints one JSON object: what the write answered.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project, provider, data, binding, execution, idempotencyKey] = process.argv;
if (idempotencyKey === undefined) {
  throw new Error(
    "usage: bun store-placed-write.mjs <project> <provider> <data directory> <binding json> <execution> <idempotency key>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

const store = JSON.parse(binding);
if (store.backend?.provider !== provider) {
  throw new Error(`the binding names \`${store.backend?.provider}\` and the parent drives \`${provider}\``);
}
const context = {
  execution: { id: execution, session_key: "" },
  signal: new AbortController().signal,
  node: "placed",
  storeRecords: [],
};
const answer = await stores.runStoreOp(store, "set", { key: "placed", value: { n: 1 } }, context, {
  via: "node",
  idempotencyKey,
});
await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
