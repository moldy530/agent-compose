// A process whose first op on a store starts PRD resolved q69's store-wide
// sweep over a backlog of expired keys, and which then counts that backlog
// **between the sweep's batches**, for the case in `store-contract.mjs` about
// how much one batch takes: at most [`SWEEP_PER_BATCH`] (1000) rows, so no op
// waits for more than the one batch in progress however large the backlog is
// — and, on a dialled store, only the rows of the one partition whose sequence
// lock the batch holds, which is what keeps a batch from deadlocking a write
// to another partition (`StoreDialect.sweepPartition`, Decision D153).
//
// A count that merely ran *while* the sweep did would prove neither: a sweep of
// one unbounded statement has removed some backlog or all of it by whenever
// the count happens to land. So each count is taken at a point that is between
// two batches by construction, and where that point is depends on the arm.
//
// **On a server**, a batch is one step of the connection's queue, and the next
// batch is queued only when the one before it has finished — at the tail,
// behind every op that arrived meanwhile (`RemoteKv`'s `#startSweep`). So an
// op issued the moment the op before it answers lands behind exactly one batch
// and ahead of the next. The first op here — a read — starts the sweep, and
// every probe after it is a `merge` whose `value_schema` refinement counts the
// backlog while the probe holds the queue. The count is made by a child
// process this one waits for synchronously (`spawnSync`), so nothing of this
// process runs while it counts: the batch before the probe has committed and
// the batch after it has not begun. Each count is the backlog after exactly
// one more batch.
//
// **On SQLite**, the driver is synchronous and the sweep takes one batch per
// turn of the event loop, each behind a `setTimeout` of its own. So this
// process counts — synchronously, over a handle of its own — the moment its
// first op answers, before it yields at all, and then once per turn, behind a
// `setTimeout` of its own queued after the sweep's next one: whether or not the
// sweep's first batch ran while that op was in flight, its counts alternate
// with the batches from there on. The local sweep is the store file's (one
// file per store, and no sequence lock to hold in one), so it is bounded by
// the batch and not by the partition.
//
// The clock: the parent hands this process an offset an hour and a minute
// ahead, which is what the first op — and so the sweep it starts — judges
// "expired" by; every probe after it runs with no offset at all, so the
// backlog is alive to the probe's own write, whose sweep of its partition
// takes nothing, and the batches are the only thing removing rows.
//
// **The ledger's half of the sweep** (PRD resolved q73) is counted the same
// way, with `ledger` as the last argument: once a store's keys are swept, its
// expired ledger rows — authored ones, the only ledger rows with an expiry —
// leave a batch at a time behind the ops too (`#sweepLedgerBatch` on a server,
// the second table of `startLocalSweep` on SQLite), at most the same 1000 a
// batch. That count is the store's whole ledger backlog, as one number under
// `ledger`: an authored row is a store's rather than a partition's. The
// probes' own ledger rows are derived ones, which never expire, so they are
// never part of it.
//
// **The history's half** (PRD resolved q74) is counted the same way, with
// `history` as the last argument, on a binding that declares `history: true`
// and a `history_retention:`: once the keys and the ledger are swept, the
// history rows past the retention leave a batch at a time behind the ops — on
// a server each batch one partition's, under that partition's sequence lock
// (the `history:` step of `RemoteKv`'s `#sweepStep`), on SQLite the third
// table of `startLocalSweep` — at most the same 1000 a batch. That count is
// every history row of the store, by partition: the parent lays the backlog
// down with nothing beside it, and the probes' own rows are in a partition of
// their own, which the parent does not read.
//
// Usage: bun store-sweep-batches.mjs <project> <provider> <data directory> <binding json> <probes> [entries|ledger|history]
//        bun store-sweep-batches.mjs --count <provider> <binding json> [entries|ledger|history]
// The first prints one JSON object: `observed`, the backlog by partition at each
// probe, and `after`, the backlog once this process has let go of the store.
// The second is the count a probe on a server spawns: the backlog by
// partition, as the server holds it now.

import { spawnSync } from "node:child_process";
import path from "node:path";
import process from "node:process";
import { fileURLToPath, pathToFileURL } from "node:url";

import { z } from "zod";

/** The variable `src/stores.ts` reads its clock offset from. */
const CLOCK = "AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS";

/**
 * The rows still waiting to be swept: every row with an expiry, since nothing
 * but the backlog the parent laid down carries one — the keys that give each
 * partition its sequence row, and the probe's, never expire, and neither does
 * a derived ledger row. Of the keys, by partition (`entries`); of the ledger,
 * as one count (`ledger`); and of the history, every row by partition
 * (`history`), since a history row carries no expiry of its own and the
 * parent lays down nothing but the backlog in the partitions it reads. `store`
 * is the placeholder a server's statement binds the store's name to, and is
 * absent on SQLite, whose file is the store's.
 */
function backlog(table, store = undefined) {
  const where = store === undefined ? "" : `store = ${store} AND `;
  if (table === "history") {
    const from = store === undefined ? "history" : `store_history WHERE store = ${store}`;
    return `SELECT scope_key, COUNT(*) AS n FROM ${from} GROUP BY scope_key`;
  }
  if (table === "ledger") {
    const from = store === undefined ? "applied" : "store_applied";
    return `SELECT 'ledger' AS scope_key, COUNT(*) AS n FROM ${from} WHERE ${where}expires_at IS NOT NULL`;
  }
  if (table !== "entries") {
    throw new Error(`there is no \`${table}\` backlog to count: \`entries\`, \`ledger\` or \`history\``);
  }
  const from = store === undefined ? "entries" : "store_entries";
  return `SELECT scope_key, COUNT(*) AS n FROM ${from} WHERE ${where}expires_at IS NOT NULL GROUP BY scope_key`;
}

/** A count's rows, as one object of partition → rows. */
const byPartition = (rows) => Object.fromEntries(rows.map((row) => [String(row.scope_key), Number(row.n)]));

if (process.argv[2] === "--count") {
  // The child a probe on a server waits for: one connection of its own, the
  // way an operator would look, and the count it prints.
  const [, , , provider, binding, table = "entries"] = process.argv;
  const store = JSON.parse(binding);
  const address = process.env[store.backend.urlEnv];
  if (provider === "postgres") {
    const pg = await import("pg");
    const Client = pg.Client ?? pg.default.Client;
    const client = new Client({ connectionString: address });
    client.on("error", () => undefined);
    await client.connect();
    try {
      const { rows } = await client.query(backlog(table, "$1"), [store.name]);
      process.stdout.write(JSON.stringify(byPartition(rows)));
    } finally {
      await client.end().catch(() => undefined);
    }
  } else {
    const mysql = await import("mysql2/promise");
    const createConnection = mysql.createConnection ?? mysql.default.createConnection;
    const connection = await createConnection({ uri: address });
    connection.on("error", () => undefined);
    try {
      const [rows] = await connection.query(backlog(table, "?"), [store.name]);
      process.stdout.write(JSON.stringify(byPartition(rows)));
    } finally {
      await connection.end().catch(() => undefined);
    }
  }
  process.exit(0);
}

const [, , project, provider, data, binding, probes, table = "entries"] = process.argv;
if (probes === undefined) {
  throw new Error(
    "usage: bun store-sweep-batches.mjs <project> <provider> <data directory> <binding json> <probes> [entries|ledger|history]",
  );
}
// A table there is no backlog of is refused here, before any op, rather than
// by the first count.
backlog(table);
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const stores = await import(pathToFileURL(path.resolve(project, "src/stores.ts")).href);

/** The SQLite driver, loaded before any count so that a count is synchronous. */
const sqlite = provider === "sqlite" ? await import("node-sqlite3-wasm") : undefined;

/** The backlog by partition — or the ledger's, as one count — from this process, synchronously. */
function counted() {
  if (sqlite !== undefined) {
    const { Database } = sqlite.default ?? sqlite;
    const database = new Database(path.join(data, "stores", `${stores.encodeKey(store.name)}.sqlite`));
    try {
      database.exec("PRAGMA busy_timeout = 5000;");
      return byPartition(database.all(backlog(table)));
    } finally {
      database.close();
    }
  }
  const child = spawnSync(process.execPath, [fileURLToPath(import.meta.url), "--count", provider, binding, table], {
    encoding: "utf8",
    timeout: 30_000,
  });
  if (child.status !== 0) {
    throw new Error(`the count exited ${child.status ?? child.signal ?? child.error}: ${child.stderr}`);
  }
  return JSON.parse(child.stdout);
}

/** What each probe counted, in order — or what stopped it counting. */
const observed = [];

/** Set for the length of one probe, so the refinement counts once per probe and only then. */
let probing = false;

const parsed = JSON.parse(binding);
if (parsed.backend?.provider !== provider) {
  throw new Error(`the binding names \`${parsed.backend?.provider}\` and the parent drives \`${provider}\``);
}
// The probe's `value_schema`: its refinement runs inside the probe's
// transaction — a partial update holds its result to the schema between
// reading its key and writing it (`decideKvWrite` in `src/stores.ts`) — which
// on a server is inside the probe's step of the connection's queue.
const store = {
  ...parsed,
  value: z
    .object({ n: z.number().int() })
    .strict()
    .superRefine(() => {
      if (!probing) return;
      probing = false;
      try {
        observed.push(counted());
      } catch (error) {
        observed.push({ error: String(error?.message ?? error) });
      }
    }),
};

const context = (session) => ({
  execution: { id: `exec_${store.name}_${session}`, session_key: session },
  signal: new AbortController().signal,
  node: "batches",
  storeRecords: [],
});

// The first op: a read, on the clock the parent handed this process, which
// starts the sweep behind it.
const first = await stores.runStoreOp(store, "get", { key: "kept" }, context("a"), { via: "node" });
process.env[CLOCK] = "0";

for (let probe = 0; probe < Number(probes); probe += 1) {
  if (sqlite !== undefined) {
    // The first count is the moment the first op answers, before this process
    // has yielded at all; every later one is behind a timer of its own.
    if (probe > 0) await new Promise((resolve) => setTimeout(resolve, 0));
    observed.push(counted());
    continue;
  }
  probing = true;
  await stores.runStoreOp(store, "merge", { key: "probe", fields: { n: probe } }, context("probe"), {
    via: "node",
    idempotencyKey: `${store.name}/probe/${probe}`,
  });
}

await stores.releaseStores();
const after = counted();
process.stdout.write(JSON.stringify({ first, observed, after }));
