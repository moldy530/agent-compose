// A process's **first op** on a store with a backlog of expired keys, for the
// case in `store-contract.mjs` about where PRD resolved q69's "next process
// start" sweep runs: behind that op, never in front of it.
//
// The op is a `get`. The moment it answers, this process counts — over a
// connection of its own, the way an operator would look — how much of the
// backlog is still stored: a sweep that ran in front of the op has already
// removed all of it, and one that runs behind it has removed at most a batch
// or two. Then it lets go of the store, which waits out the sweep, and counts
// again: the backlog is gone by the time the process is.
//
// On SQLite the count is synchronous and the sweep's first batch waits for a
// turn of the event loop, so the first count sees the whole backlog; on a
// server the sweep's batches run while the count is in flight, so it sees
// whatever they have not reached yet.
//
// Usage: bun store-first-op.mjs <project> <provider> <data directory> <binding json> <live until, ms>
// Prints one JSON object: what the `get` answered, and the two counts.

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, provider, data, binding, liveUntil] = process.argv;
if (liveUntil === undefined) {
  throw new Error(
    "usage: bun store-first-op.mjs <project> <provider> <data directory> <binding json> <live until>",
  );
}
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

const store = JSON.parse(binding);
if (store.backend?.provider !== provider) {
  throw new Error(`the binding names \`${store.backend?.provider}\` and the parent drives \`${provider}\``);
}

/** How many of the store's rows expire before `liveUntil` — the backlog, still stored. */
async function backlog() {
  if (provider === "sqlite") {
    const module = await import("node-sqlite3-wasm");
    const { Database } = module.default ?? module;
    const database = new Database(path.join(data, "stores", `${stores.encodeKey(store.name)}.sqlite`));
    try {
      database.exec("PRAGMA busy_timeout = 5000;");
      return Number(
        database.all("SELECT COUNT(*) AS n FROM entries WHERE expires_at < ?", [Number(liveUntil)])[0]?.n,
      );
    } finally {
      database.close();
    }
  }
  const address = process.env[store.backend.urlEnv];
  if (provider === "postgres") {
    const pg = await import("pg");
    const Client = pg.Client ?? pg.default.Client;
    const client = new Client({ connectionString: address });
    client.on("error", () => undefined);
    await client.connect();
    try {
      const { rows } = await client.query(
        "SELECT COUNT(*) AS n FROM store_entries WHERE store = $1 AND expires_at < $2",
        [store.name, Number(liveUntil)],
      );
      return Number(rows[0]?.n);
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const mysql = await import("mysql2/promise");
  const createConnection = mysql.createConnection ?? mysql.default.createConnection;
  const connection = await createConnection({ uri: address });
  connection.on("error", () => undefined);
  try {
    const [rows] = await connection.query(
      "SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND expires_at < ?",
      [store.name, Number(liveUntil)],
    );
    return Number(rows[0]?.n);
  } finally {
    await connection.end().catch(() => undefined);
  }
}

const context = {
  execution: { id: `exec_${store.name}_first`, session_key: "" },
  signal: new AbortController().signal,
  node: "first",
  storeRecords: [],
};
const answer = await stores.runStoreOp(store, "get", { key: "kept" }, context, { via: "node" });
const atAnswer = await backlog();
await stores.releaseStores();
const afterRelease = await backlog();
process.stdout.write(JSON.stringify({ answer, atAnswer, afterRelease }));
