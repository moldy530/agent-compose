// A **process start** that never reaches a store, for the case in
// `store-contract.mjs` about PRD resolved q69 (c)'s "or the next process
// start": an expired key leaves storage when the deployment next starts, even
// where nothing that process does will ever read or write the store.
//
// This process does what a `run`, a `resume` and a `serve` do as they start —
// registers the stores the composition binds, exactly as `src/graph.ts` does as
// it loads (`stores.bindsStores`), and starts their sweep
// (`stores.sweepBoundStores`, which `src/cli.ts` and `src/serve.ts` call) —
// and then makes **no op at all** before it lets go of the stores, which is
// where a command waits for what it started. The first-op sweep is therefore
// not what removes anything here: there is no first op.
//
// The clock it reads is the one the parent hands it, through the same
// `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS` every other case moves.
//
// Usage: bun store-process-start.mjs <project> <provider> <data directory> <binding json>
// Prints one JSON object: `{ "started": true }` once the stores are let go of.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project, provider, data, binding] = process.argv;
if (binding === undefined) {
  throw new Error(
    "usage: bun store-process-start.mjs <project> <provider> <data directory> <binding json>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

const store = JSON.parse(binding);
if (store.backend?.provider !== provider) {
  throw new Error(`the binding names \`${store.backend?.provider}\` and the parent drives \`${provider}\``);
}
stores.bindsStores([store]);
stores.sweepBoundStores();
await stores.releaseStores();
process.stdout.write(JSON.stringify({ started: true }));
