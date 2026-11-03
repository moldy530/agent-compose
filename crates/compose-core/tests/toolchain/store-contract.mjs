// Drives grammar 11.4's `kv` contract against one emitted project's store
// backends, whichever backend that project's deploy layer bound (grammar §14.3,
// PRD resolved q63).
//
// The point of running this rather than reading it: the ops in `src/stores.ts`
// are **one** implementation shared by every backend — a store-op node and a
// synthesized tool call `runStoreOp` and cannot tell which arm answered — so
// what a second or third backend can break is not the logic but the server's
// answer to it: a collation that folds case where a `kv` key must not, one that
// pads a trailing space away, a column too small for a value, an upsert spelled
// two ways, a placeholder dialect, a unique index that does or does not make a
// double-delivered write land once. None of that is visible to `tsc`, and none
// of it is visible to a fake. So the same cases run against each backend a
// deployment can really bind, and the Rust side compares the answers.
//
// The bindings are written here rather than read out of `src/graph.ts`, for
// `store-backends.mjs`'s reason: what `graph.ts` emits for a `store.*` is
// already held by the golden corpus, and what is in question here is the backend
// underneath it.
//
// Usage: node store-contract.mjs <project> <provider> <data directory>
// Environment: AGENT_COMPOSE_STORE_URL and AGENT_COMPOSE_STORE_URL_B … _E on a
// dialled provider — five names for one server, which is how the interleaved
// writers below become two connections and the killed one becomes a sixth this
// file can take away without touching the others.
// Prints one JSON object of everything the suite asserts about.

import { spawn } from "node:child_process";
import { randomUUID } from "node:crypto";
import fs from "node:fs";
import { fileURLToPath, pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { z } from "zod";

const [, , project, provider, data] = process.argv;
if (project === undefined || provider === undefined || data === undefined) {
  throw new Error("usage: node store-contract.mjs <project> <provider> <data directory>");
}

// Read by `dataRoot()` at every call, so setting it before the import is not
// required — but setting it here keeps the whole run, journal included, inside
// one directory the caller can throw away.
process.env["AGENT_COMPOSE_DATA_DIR"] = data;

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const stores = await import(at("src/stores.ts"));
const journal = await import(at("src/journal.ts"));
// The module `src/stores.ts` imports, so `ToolCallRefused` here is the class it
// throws rather than a second copy of it.
const runtime = await import(at("src/runtime.ts"));

/** The variable each of the five connections reads its address from. */
const URL_ENV = "AGENT_COMPOSE_STORE_URL";
const OTHER_URL_ENV = "AGENT_COMPOSE_STORE_URL_B";
const THIRD_URL_ENV = "AGENT_COMPOSE_STORE_URL_C";
const FOURTH_URL_ENV = "AGENT_COMPOSE_STORE_URL_D";
/** …and the fifth, which is the one this file gets the server to take away. */
const KILLED_URL_ENV = "AGENT_COMPOSE_STORE_URL_E";

/**
 * A prefix nothing else on this server is using.
 *
 * Every case below keys on a store **name**, and a `global` partition is shared
 * by every run that ever addressed it: CI's containers are fresh, and the
 * developer this suite's skip notice invites to "set it locally" points it at a
 * server that keeps rows. A fixed name would make their second run read as a
 * backend failure — a row from last run answering a case about this one — which
 * is this runner's bookkeeping rather than anything a server did.
 */
const RUN = `c${randomUUID().replaceAll("-", "").slice(0, 12)}`;

/** A value with the shapes a `TEXT` column and a folding collation break. */
const UNICODE = "héllo → 世界 🙂 \\\" ' ; = -- /* not sql */";

/** …and one past the 64 KiB a MySQL `TEXT` holds (PRD resolved q63). */
const LARGE = "x".repeat(200_000);

/**
 * A key past the few hundred characters a column indexed whole can hold.
 *
 * Grammar 11.4 puts no restriction on a `kv` key and the local backend's SQLite
 * `TEXT` has none either, so a key this long is a green `store set` on one
 * backend and has to be one on all three — which is the same sentence this file
 * already drives about a value past 64 KiB. 1024 characters rather than the
 * largest any arm takes: what is under test is that the arms agree, and the
 * Postgres arm's own ceiling is its btree tuple.
 */
const LONG_KEY = `long-${"k".repeat(1019)}`;

/**
 * …and a session key whose *partition* name is the long one.
 *
 * The quieter half of the same column question, and the half a composition does
 * not choose: a partition is `session/` plus `encodeKey(sessionKey)`, which
 * percent-encodes every byte outside `[A-Za-z0-9_-]` — so one CJK character of a
 * session key a trigger supplied becomes nine ASCII ones, and 100 of them are
 * 908.
 */
const LONG_SESSION = "世".repeat(100);

/**
 * One binding, on the backend this run is driving.
 *
 * `value` is the store's `value_schema` as the emitted Zod, which is what a
 * partial update holds its result to (PRD resolved q68); a binding the cases
 * below only `set` and `get` through needs none.
 */
const kv = (name, scope, variable = URL_ENV, value = undefined) => ({
  address: `store.${RUN}_${name}`,
  name: `${RUN}_${name}`,
  kind: "kv",
  scope,
  metadata: false,
  ...(value === undefined ? {} : { value }),
  backend: {
    provider,
    from: "the `kv` default of the `remote` target",
    ...(provider === "sqlite" ? {} : { urlEnv: variable }),
  },
});

/** A store-op node's context, with the record channel a trace entry reads. */
const ctx = (id, session = "", effects = undefined) => ({
  execution: { id, session_key: session },
  signal: new AbortController().signal,
  node: "probe",
  storeRecords: [],
  ...(effects === undefined ? {} : { effects }),
});

const node = (key) => ({ via: "node", ...(key === undefined ? {} : { idempotencyKey: key }) });

/** An execution id nothing else in this run or any other uses. */
const execution = (label) => `exec_${RUN}_${label}`;

const same = (left, right) => JSON.stringify(left) === JSON.stringify(right);

const set = (store, key, value, run, key_ = undefined) =>
  stores.runStoreOp(store, "set", { key, value }, run, node(key_));
const get = (store, key, run) => stores.runStoreOp(store, "get", { key }, run, node());
const list = (store, run, prefix = undefined, limit = 50) =>
  stores.runStoreOp(store, "list", { limit, ...(prefix === undefined ? {} : { prefix }) }, run, node());

/**
 * Run one helper process of this directory's beside this one, and answer the
 * JSON it printed with what it said on stderr.
 */
const helper = (script, args, env = process.env) =>
  new Promise((resolve, reject) => {
    const child = spawn(
      process.execPath,
      [path.join(path.dirname(fileURLToPath(import.meta.url)), script), ...args],
      { env, stdio: ["ignore", "pipe", "pipe"] },
    );
    let out = "";
    let err = "";
    child.stdout.on("data", (chunk) => (out += chunk));
    child.stderr.on("data", (chunk) => (err += chunk));
    child.on("error", reject);
    child.on("close", (code) =>
      code === 0
        ? resolve({ answer: JSON.parse(out), stderr: err })
        : reject(new Error(`\`${script}\` exited ${code}: ${err}`)),
    );
  });

// --- The second connection, which is this file's and not a store's -----------
//
// The last case needs two things no `kv` op can do: hold a row lock on the key a
// store is about to write, and end that store's session. Both are the server's
// own vocabulary, so they are spoken over a connection this file opens with the
// project's pinned driver directly — resolved out of the toolchain fixture's
// `node_modules` beside this runner, which pins the same `pg` and `mysql2` the
// emitted `package.json` does.

/**
 * A second connection to the server the killed store binding dials.
 *
 * Both drivers are CommonJS, so the export is taken off the namespace or off its
 * `default` — whichever this runtime's interop put it on. Each connection gets
 * an `error` listener for the reason the arms' own do: an `EventEmitter` that
 * emits `error` with nothing listening ends the process, and this one is beside
 * a connection deliberately being killed.
 */
async function openKiller() {
  return await openServer(process.env[KILLED_URL_ENV]);
}

/**
 * A connection of this file's own to `address`, with the pinned driver — the
 * killer's, and the one the reader and the earlier release's schema below are
 * spoken over. See [`openKiller`].
 */
async function openServer(address) {
  if (provider === "postgres") {
    const pg = await import("pg");
    const Client = pg.Client ?? pg.default.Client;
    const client = new Client({ connectionString: address });
    client.on("error", () => undefined);
    await client.connect();
    return client;
  }
  const mysql = await import("mysql2/promise");
  const createConnection = mysql.createConnection ?? mysql.default.createConnection;
  const connection = await createConnection({ uri: address });
  connection.on("error", () => undefined);
  return connection;
}

/** Rows, out of whichever driver answered. */
async function rows(killer, sql, parameters = []) {
  if (provider === "postgres") return (await killer.query(sql, parameters)).rows;
  const [answered] = await killer.query(sql, parameters);
  return answered;
}

async function begin(killer) {
  await rows(killer, provider === "postgres" ? "BEGIN" : "START TRANSACTION");
}

async function rollback(killer) {
  await rows(killer, "ROLLBACK");
}

/**
 * Take — and keep — the row lock the store's next write has to wait for.
 *
 * An uncommitted insert of the very key the store is about to write is a lock on
 * that key's index record on both servers, and the store's own
 * `INSERT … ON CONFLICT`/`ON DUPLICATE KEY` waits on it rather than failing. The
 * columns are the four the arms declare; a MySQL row's two hashes are generated
 * and are the server's to fill.
 */
async function holdTheKey(killer, store, key) {
  const value = JSON.stringify({ held: "by the killer" });
  await rows(
    killer,
    provider === "postgres"
      ? 'INSERT INTO store_entries (store, scope_key, "key", value) VALUES ($1, $2, $3, $4)'
      : "INSERT INTO store_entries (store, scope_key, `key`, value) VALUES (?, ?, ?, ?)",
    [store, "global", key, value],
  );
}

/**
 * Take — and keep — the lock a keyed write's **ledger** insert has to wait for:
 * an uncommitted ledger row under `key` in `store`, on the primary key
 * `(store, idempotency_key)` every arm's claim conflicts on.
 *
 * Both servers run a store's writes at `READ COMMITTED`, and neither shows a
 * row another transaction has not committed to a plain read — so the write's
 * own ledger check, made before its effect, finds nothing and the write goes
 * ahead. It is the write's ledger insert, after every statement of its effect,
 * that meets the row and waits on it: `ON CONFLICT DO NOTHING` and
 * `ON DUPLICATE KEY UPDATE` both wait for the transaction that inserted a
 * conflicting row to end rather than answer before it has.
 */
async function holdTheLedgerRow(killer, store, key) {
  const result = JSON.stringify({ held: "by the killer" });
  await rows(
    killer,
    provider === "postgres"
      ? "INSERT INTO store_applied (store, scope_key, idempotency_key, op, result) VALUES ($1, $2, $3, $4, $5)"
      : "INSERT INTO store_applied (store, scope_key, idempotency_key, op, result) VALUES (?, ?, ?, ?, ?)",
    [store, "global", key, "batch", result],
  );
}

/**
 * The session id of the store's connection, once the server can see it waiting.
 *
 * Polled rather than assumed, because "the statement is in flight" is the whole
 * premise of the case that calls this: a kill that landed on an idle connection
 * would exercise the driver's `error` event instead, which is the half that was
 * never in doubt. `undefined` when it never turns up, which the caller reports
 * rather than hiding.
 *
 * MySQL is asked by the statement text — `mysql2` interpolates parameters on the
 * client, so the blocked statement carries this run's unique store name — of a
 * session **waiting on a row lock** (`performance_schema.data_lock_waits`), and
 * not by the text alone: a store's connection runs several statements naming
 * the store before the one that blocks, as does the store-wide sweep its first
 * op started, so a poll on the text alone can land on one of those and answer
 * before the store has written anything — a batch's first entry included,
 * which a probe then finds free. `information_schema.innodb_trx` would say
 * `LOCK WAIT` too, but it is a cache refreshed only once nobody has read it for
 * a tenth of a second, which a poll every 50 ms never allows. Postgres is asked
 * by state, since `pg` binds parameters on the server and its
 * `pg_stat_activity.query` holds `$1` where the name would be — and by the
 * session waiting on a **lock**, among **client** backends: an autovacuum worker
 * reports `state = 'active'` with `autovacuum: VACUUM … store_entries` as its
 * query, and the cases above write that table often enough to wake one, so a
 * filter on the text alone finds two sessions and names neither.
 *
 * **The Postgres view is cleared before every poll**, and that is what makes it
 * a poll at all. The killer asks from inside the transaction holding the lock,
 * and Postgres hands a transaction one snapshot of the activity view at its
 * first read and the same one on every read after (`stats_fetch_consistency`):
 * a first poll that landed while the store was still on the statements before
 * the one that blocks — a write locks its partition's sequence row and reads
 * its key before it inserts — would be the answer to all hundred.
 *
 * **`table` is the table the blocked statement writes** — `store_entries` for
 * a write held at its key ([`holdTheKey`]), `store_applied` for one held at its
 * ledger row ([`holdTheLedgerRow`]) — and both arms ask for it, so a poll lands
 * on the statement the killer's lock is in front of and on no earlier one. On
 * Postgres, whose query text carries no store name, the session must also be
 * one **this killer** blocks (`pg_blocking_pids`): the text and the wait alone
 * would match another run's store on the same server — a second suite pointed
 * at one database — and this one would end it.
 */
async function waitingOn(killer, store, table = "store_entries") {
  const found = async () => {
    if (provider === "postgres") {
      await rows(killer, "SELECT pg_stat_clear_snapshot()");
      return await rows(
        killer,
        "SELECT pid AS id FROM pg_stat_activity WHERE datname = current_database() " +
          "AND pid <> pg_backend_pid() AND backend_type = 'client backend' " +
          "AND state = 'active' AND wait_event_type = 'Lock' AND query LIKE $1 " +
          "AND pg_backend_pid() = ANY (pg_blocking_pids(pid))",
        [`%${table}%`],
      );
    }
    return await rows(
      killer,
      "SELECT DISTINCT t.PROCESSLIST_ID AS id FROM performance_schema.data_lock_waits w " +
        "JOIN performance_schema.threads t ON t.THREAD_ID = w.REQUESTING_THREAD_ID " +
        "WHERE t.PROCESSLIST_DB = DATABASE() AND t.PROCESSLIST_ID <> CONNECTION_ID() " +
        "AND t.PROCESSLIST_INFO LIKE ? AND t.PROCESSLIST_INFO LIKE ?",
      [`%${store}%`, `%${table}%`],
    );
  };
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const held = await found();
    if (held.length === 1) return Number(held[0].id);
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  return undefined;
}

/** End that session, from outside the process that owns it. */
async function kill(killer, id) {
  if (provider === "postgres") {
    await rows(killer, "SELECT pg_terminate_backend($1)", [id]);
    return;
  }
  // `KILL` takes no placeholder, and `id` is a number this file read off the
  // server's own session list.
  await rows(killer, `KILL ${id}`);
}

const answer = {};

// --- A fresh schema, opened by four connections at once ----------------------

{
  // **First in this file on purpose.** A dialled store's schema is created at
  // the first op of whichever process gets there first, and grammar 14.1 rule 5
  // admits a dialled store from a *placement* — a hub and its workers, each of
  // which opens one at its own first op — so several first opens against one
  // fresh database is the ordinary start-up rather than an edge case. Neither
  // server makes `CREATE TABLE IF NOT EXISTS` atomic against a concurrent
  // creator: Postgres can answer one of the two a duplicate key in `pg_type` or
  // `pg_class`, which is a catalog error naming nothing an operator wrote. This
  // case is only really a race on a database whose tables do not exist yet,
  // which is what running it before anything else buys — CI's service containers
  // are fresh, and a developer's second run against a kept server drives the
  // weaker half of it.
  //
  // On the local backend the four bindings are one store by design (a local
  // provider names no `url:`), and the case still says what it says about four
  // concurrent first ops.
  const openers = [URL_ENV, OTHER_URL_ENV, THIRD_URL_ENV, FOURTH_URL_ENV];
  const run = ctx(execution("opening"));
  const opened = openers.map((variable) => kv("opening", "global", variable));
  const settled = await Promise.allSettled(
    opened.map((store, index) => set(store, `k${index}`, { opener: index }, run, `n/${index}`)),
  );
  answer.a_fresh_schema_takes_every_opener_at_once = settled.every(
    (outcome) => outcome.status === "fulfilled",
  );
  // Not asserted on, and printed by the Rust side beside the failure: a schema
  // that lost the race says so by SQLSTATE, and this is where that sentence is.
  answer.opening_errors = settled
    .filter((outcome) => outcome.status === "rejected")
    .map((outcome) => String(outcome.reason?.message ?? outcome.reason));
  const held = await Promise.all(opened.map((store, index) => get(store, `k${index}`, run)));
  answer.every_opener_wrote_through_its_own_connection = held.every(
    (row, index) => row.found === true && row.value?.opener === index,
  );
}

// --- An open beside a reader -------------------------------------------------

if (provider !== "sqlite") {
  // A dialled store runs its schema on **every** open — every process start,
  // and every redial after a lost connection — so a statement in it that waits
  // on a reader is a `serve` that cannot open for as long as a `pg_dump` or an
  // operator's open transaction lasts, and every other process's store ops
  // queue behind the open's own lock request meanwhile. Postgres takes
  // `ACCESS EXCLUSIVE` for an `ALTER TABLE … ADD COLUMN IF NOT EXISTS` before it
  // looks for the column, which is why the arm probes for the one migration it
  // has rather than attempting it (`stores-postgres.ts`).
  //
  // The reader is the lock a `get` takes, held open: a transaction that has
  // read every table the schema names. A fresh variable is a fresh connection,
  // so the `get` beside it is an open and runs the whole schema.
  const READER_URL_ENV = "AGENT_COMPOSE_STORE_URL_READER";
  process.env[READER_URL_ENV] = process.env[URL_ENV];
  const store = kv("beside_a_reader", "global", READER_URL_ENV);
  const run = ctx(execution("beside_a_reader"));
  const reader = await openServer(process.env[URL_ENV]);
  let opening;
  try {
    await begin(reader);
    for (const table of ["store_entries", "store_applied", "store_sequences"]) {
      await rows(reader, `SELECT COUNT(*) AS held FROM ${table}`);
    }
    // Settled into a string the moment the promise exists: a rejection nothing
    // is listening to yet would end this process.
    opening = get(store, "anything", run).then(
      () => "answered",
      (error) => `refused: ${String(error?.message ?? error)}`,
    );
    const waited = await Promise.race([
      opening,
      new Promise((resolve) => setTimeout(() => resolve("still waiting after 3 s"), 3_000)),
    ]);
    answer.an_open_does_not_wait_on_a_reader = waited === "answered";
    answer.open_beside_a_reader = waited;
  } catch (error) {
    answer.open_beside_a_reader = String(error?.message ?? error);
  } finally {
    // Released before the open is awaited, so an open that *is* waiting on the
    // reader finishes and this runner fails the case rather than hanging.
    await rollback(reader).catch(() => undefined);
    if (opening !== undefined) await opening;
    await reader.end().catch(() => undefined);
  }
}

// --- First writes into distinct partitions, at once --------------------------

{
  // The other half of what a write's isolation level decides, and the half a
  // case about one key cannot see. A `kv` write locks the key it reads, and
  // every **first** write of a key — every `if: absent` claim, every append
  // that starts a list — reads a key that is not there yet. Under InnoDB's
  // default `REPEATABLE READ` that read takes a gap lock spanning the index
  // between two existing rows, so two writers in two different partitions
  // whose keys fall in one gap each hold one and then wait on the other's to
  // insert: a deadlock the server breaks by failing one of them, between
  // writes that share nothing. The MySQL arm asserts `READ COMMITTED` on its
  // session and refuses a session that will not hold it
  // (`MYSQL_STORE_ISOLATION`), which is what this case is the test of; the
  // single-partition race above passes under either level.
  //
  // Eight connections to one server, each making the first two writes of a
  // fresh `session` partition per round — a claim and an append — at once
  // with the seven others. On the local backend the eight are one handle and
  // the case says what it says about distinct partitions.
  const PARTITION_URL_ENVS = Array.from({ length: 8 }, (_, index) => `AGENT_COMPOSE_STORE_URL_PART_${index}`);
  if (provider !== "sqlite") {
    for (const variable of PARTITION_URL_ENVS) process.env[variable] = process.env[URL_ENV];
  }
  const schema = z.object({ owner: z.string().optional(), items: z.array(z.string()).optional() }).strict();
  const writers = PARTITION_URL_ENVS.map((variable) => kv("partitions", "session", variable, schema));
  const ROUNDS = 40;
  const failures = [];
  let landed = 0;
  for (let round = 0; round < ROUNDS; round += 1) {
    const settled = await Promise.allSettled(
      writers.map(async (store, index) => {
        const run = ctx(execution(`partition_${round}_${index}`), `partition-${round}-${index}`);
        const claimed = await stores.runStoreOp(
          store,
          "set",
          { key: "claim", value: { owner: `writer-${index}` }, condition: "absent" },
          run,
          node(`pt/${round}/${index}/claim`),
        );
        const appended = await stores.runStoreOp(
          store,
          "append",
          { key: "list", field: "items", value: `round-${round}` },
          run,
          node(`pt/${round}/${index}/append`),
        );
        return claimed.applied === true && appended.applied === true;
      }),
    );
    for (const outcome of settled) {
      if (outcome.status === "rejected") failures.push(String(outcome.reason?.message ?? outcome.reason));
      else if (outcome.value) landed += 1;
    }
  }
  answer.concurrent_first_writes_into_distinct_partitions_all_land =
    failures.length === 0 && landed === ROUNDS * writers.length;
  answer.partition_failures = { count: failures.length, first: failures.slice(0, 3) };
}

// --- Round trip: unicode, size, and the types `value_schema` admits ----------

{
  const store = kv("roundtrip", "global");
  const run = ctx(execution("round"));
  const value = {
    text: UNICODE,
    count: 42,
    ratio: 1.5,
    flag: false,
    empty: null,
    nested: { list: [1, "two", null, { deep: true }] },
  };
  await set(store, UNICODE, value, run, "r/1");
  const held = await get(store, UNICODE, run);
  answer.round_trips_a_unicode_key_and_value = held.found === true && same(held.value, value);
  // Type fidelity rather than "it looks the same": a column that stored the
  // document rather than the text would come back with numbers respelled and a
  // key order of the server's choosing.
  answer.round_trips_the_types_value_schema_admits =
    typeof held.value?.count === "number" &&
    held.value?.count === 42 &&
    held.value?.ratio === 1.5 &&
    held.value?.flag === false &&
    held.value?.empty === null &&
    Array.isArray(held.value?.nested?.list) &&
    held.value?.nested?.list?.[3]?.deep === true;

  await set(store, "large", { body: LARGE }, run, "r/2");
  const big = await get(store, "large", run);
  answer.round_trips_a_value_past_64_kib =
    big.found === true && big.value?.body?.length === LARGE.length;

  const missed = await get(store, "nothing-here", run);
  // A miss answers `found: false` and **no** `value` at all (Decision D110).
  answer.a_miss_answers_found_false_with_no_value =
    missed.found === false && !("value" in missed);
}

// --- Keys: case, padding, order, prefix --------------------------------------

{
  const store = kv("keys", "global");
  const run = ctx(execution("keys"));
  await set(store, "Key", { which: "upper" }, run, "k/1");
  await set(store, "key", { which: "lower" }, run, "k/2");
  const upper = await get(store, "Key", run);
  const lower = await get(store, "key", run);
  answer.keys_are_case_sensitive =
    upper.value?.which === "upper" && lower.value?.which === "lower";

  // A collation that pads is the second half of the same failure: under
  // PAD SPACE `'pad'` and `'pad '` are one row, so a store holding both holds
  // one and a `get` of either answers the survivor.
  await set(store, "pad", { which: "bare" }, run, "k/3");
  await set(store, "pad ", { which: "spaced" }, run, "k/4");
  const bare = await get(store, "pad", run);
  const spaced = await get(store, "pad ", run);
  answer.keys_keep_a_trailing_space =
    bare.value?.which === "bare" && spaced.value?.which === "spaced";

  const deleted = await stores.runStoreOp(store, "delete", { key: "pad " }, run, node("k/5"));
  const again = await stores.runStoreOp(store, "delete", { key: "pad " }, run, node("k/6"));
  answer.a_delete_answers_once = deleted.deleted === true && again.deleted === false;
}

{
  // `list` is one row of one catalogue and every backend answers it in UTF-8
  // byte order — which on a server is the column's collation answering. U+FF00
  // is `EF BC 80` and U+1F600 is `F0 9F 98 80`, so bytes put U+FF00 first while
  // a UTF-16 comparison puts the surrogate first.
  const store = kv("order", "global");
  const run = ctx(execution("order"));
  for (const [index, key] of ["zz", "＀", "\u{1F600}", "😀alpha", "😀beta"].entries()) {
    await set(store, key, { n: index }, run, `o/${index}`);
  }
  answer.list_order = (await list(store, run)).keys;
  answer.list_limited = (await list(store, run, undefined, 2)).keys;
  // A prefix outside the BMP, which two string lengths disagree about: the
  // servers count characters in `SUBSTR` and a JavaScript `.length` counts
  // UTF-16 code units.
  answer.prefix_keys = (await list(store, run, "😀")).keys;
  answer.deeper_prefix_keys = (await list(store, run, "😀al")).keys;
}

// --- Keys and partitions longer than a column indexed whole can hold ---------

{
  // A `kv` key is whatever the composition evaluated — a URL with a query
  // string, a document path, a concatenated identifier — and grammar 11.4 bounds
  // none of it. A backend that bounds it where the others do not turns a
  // composition green on a laptop into an `ER_DATA_TOO_LONG` the moment a deploy
  // file swaps the backend under it, which is the divergence this whole suite is
  // about.
  const store = kv("long_key", "global");
  const run = ctx(execution("long"));
  await set(store, LONG_KEY, { text: "long" }, run, "l/1");
  const held = await get(store, LONG_KEY, run);
  answer.round_trips_a_key_longer_than_512_characters =
    held.found === true && held.value?.text === "long";
  // …and it is still a whole key rather than a prefix of one: a column that
  // truncated instead of refusing would make these two one row, and a `get` of
  // either would answer the survivor.
  const neighbour = `${LONG_KEY}-and-more`;
  await set(store, neighbour, { text: "neighbour" }, run, "l/2");
  answer.two_long_keys_sharing_a_prefix_are_two_rows =
    (await get(store, LONG_KEY, run)).value?.text === "long" &&
    (await get(store, neighbour, run)).value?.text === "neighbour";
}

{
  // The partition name is the quieter half, because the composition does not
  // choose it: it is `session/` plus the encoded session key a **trigger**
  // supplied, and `encodeKey` inflates a non-ASCII one ninefold.
  const store = kv("long_session", "session");
  const mine = ctx(execution("l_s1"), LONG_SESSION);
  const later = ctx(execution("l_s2"), LONG_SESSION);
  const other = ctx(execution("l_s3"), `${LONG_SESSION}世`);
  await set(store, "note", { text: "mine" }, mine, "ls/1");
  answer.a_long_session_key_is_its_own_partition =
    (await get(store, "note", later)).value?.text === "mine" &&
    (await get(store, "note", other)).found === false;
}

// --- Reading at scale: pages, counts and `get_many` (PRD resolved q70) -------

/** Two keys in UTF-8 byte order, which is the order every arm pages in. */
const byBytes = (left, right) => Buffer.compare(Buffer.from(left, "utf8"), Buffer.from(right, "utf8"));

/** One page of a `kv` `list`, through a store-op node's surface. */
const page = (store, run, { limit, prefix, after, includeValues, site } = {}) =>
  stores.runStoreOp(
    store,
    "list",
    {
      limit,
      ...(prefix === undefined ? {} : { prefix }),
      ...(after === undefined ? {} : { after }),
      ...(includeValues === undefined ? {} : { includeValues }),
    },
    run,
    node(site),
  );

/**
 * Every page of a walk: the first page with no cursor, and each one after it
 * taking the `next` the page before answered — what a flow paging a store
 * does, one store-op node run per page.
 */
async function walk(store, run, options) {
  const pages = [];
  let after = options.after;
  for (;;) {
    const answered = await page(store, run, { ...options, after });
    pages.push(answered);
    if (!("next" in answered)) return pages;
    // A walk that does not move is a cursor that does not advance, which the
    // assertions below would read as a hang rather than a failure.
    if (answered.next === after || pages.length > 1_000_000) return pages;
    after = answered.next;
  }
}

/**
 * Lay `seeded` into one partition of `store` in bulk, beside the store rather
 * than through it — a 100,000-key store seeded one `set` at a time is a
 * hundred thousand transactions, and what is under test is how the store
 * reads, not how fast it writes. The store has already been opened once, so
 * the schema the rows go into is the one this release created; each row
 * carries the version its write would have taken.
 */
async function bulkSeed(store, scopeKey, seeded) {
  const BATCH = 1000;
  if (provider === "sqlite") {
    const module = await import("node-sqlite3-wasm");
    const { Database } = module.default ?? module;
    const file = new Database(path.join(data, "stores", `${stores.encodeKey(store.name)}.sqlite`));
    try {
      file.exec("PRAGMA busy_timeout = 5000;");
      file.exec("BEGIN IMMEDIATE");
      for (let start = 0; start < seeded.length; start += BATCH) {
        const batch = seeded.slice(start, start + BATCH);
        file.run(
          `INSERT INTO entries (scope_key, key, value, version) VALUES ${batch.map(() => "(?, ?, ?, ?)").join(", ")}`,
          batch.flatMap((row) => [scopeKey, row.key, JSON.stringify(row.value), row.version]),
        );
      }
      file.exec("COMMIT");
    } finally {
      file.close();
    }
    return;
  }
  const seeder = await openServer(process.env[URL_ENV]);
  try {
    await begin(seeder);
    for (let start = 0; start < seeded.length; start += BATCH) {
      const batch = seeded.slice(start, start + BATCH);
      let next = 0;
      const tuple = () =>
        provider === "postgres"
          ? `($${++next}, $${++next}, $${++next}, $${++next}, $${++next})`
          : "(?, ?, ?, ?, ?)";
      await rows(
        seeder,
        `INSERT INTO store_entries (store, scope_key, ${provider === "postgres" ? '"key"' : "`key`"}, value, version) VALUES ${batch
          .map(tuple)
          .join(", ")}`,
        batch.flatMap((row) => [store.name, scopeKey, row.key, JSON.stringify(row.value), row.version]),
      );
    }
    await rows(seeder, "COMMIT");
  } finally {
    await seeder.end().catch(() => undefined);
  }
}

/**
 * Take every row of `store` back off the server — or its file off the disk —
 * and answer whether none is left.
 *
 * [`bulkSeed`]'s other half. Every store this runner names is run-unique, so
 * no later run reads what this one left; but nothing deletes it either, and the
 * server a developer points this suite at keeps it: a hundred thousand rows a
 * run, per dialled arm, accumulating until the disk is full. So the walk hands
 * its rows back when it is done, whether its cases passed or not. Only this
 * run's store, by its run-unique name, because the server is shared with
 * whatever else is pointed at it.
 *
 * The SQLite arm's file is removed rather than emptied: the store's own handle
 * is still open on it, and an unlinked file is one nothing reaches again.
 */
async function forget(store) {
  if (provider === "sqlite") {
    const file = path.join(data, "stores", `${stores.encodeKey(store.name)}.sqlite`);
    for (const left of [file, `${file}-journal`, `${file}.lock`]) {
      fs.rmSync(left, { recursive: true, force: true });
    }
    return !fs.existsSync(file);
  }
  return await sweep("=", store.name);
}

/**
 * Delete the server's rows of every store whose name stands in `relation` to
 * `operand` — in every table a store keeps rows in, entries, ledger and
 * version sequence alike — and answer whether none is left.
 *
 * MySQL's `REPEATABLE READ` default would have each `DELETE` take next-key
 * locks over the range it scans and the gap past it, and that gap is between
 * this run's stores and whatever another suite sharing the server is writing
 * beside them. `READ COMMITTED` locks the rows deleted and nothing else, which
 * is all a cleanup needs; Postgres takes no gap locks at any level.
 */
async function sweep(relation, operand) {
  const admin = await openServer(process.env[URL_ENV]);
  const placeholder = provider === "postgres" ? "$1" : "?";
  try {
    if (provider === "mysql") await rows(admin, "SET SESSION TRANSACTION ISOLATION LEVEL READ COMMITTED");
    let left = 0;
    for (const table of ["store_entries", "store_applied", "store_sequences"]) {
      await rows(admin, `DELETE FROM ${table} WHERE store ${relation} ${placeholder}`, [operand]);
      const [held] = await rows(admin, `SELECT COUNT(*) AS n FROM ${table} WHERE store ${relation} ${placeholder}`, [
        operand,
      ]);
      left += Number(held?.n);
    }
    return left === 0;
  } finally {
    await admin.end().catch(() => undefined);
  }
}

/**
 * How many keys the large walk seeds — on every arm, because PRD resolved
 * q70's verification is "a flow reads every key of a 100,000-key store page
 * by page on each arm and sees each key once". Reported, so the Rust side
 * holds the number as well as the verdict.
 */
const WALK_KEYS = 100_000;
answer.walk_keys = WALK_KEYS;

/** The marks a seeded key's tail is drawn from: case, accents, CJK, astral. */
const MIXED = ["a", "B", "z", "É", "é", "Ω", "中", "😀", "＀", "A b"];

/** The large walk's store, named outside its case so the `finally` can forget it. */
const WALKED = kv("walk", "global");

try {
  // **A walk of a large store, page by page.** Half the keys sit under `p/`
  // and half under `q/`, and every one carries a mark from [`MIXED`], so the
  // byte order the pages come back in is not the order the keys were written
  // in, and a page boundary falls between keys whose order a UTF-16 or a
  // case-folding comparison would answer differently.
  const store = WALKED;
  const run = ctx(execution("walk"));
  // Opened through the store first, so the rows go into this release's schema.
  answer.a_fresh_store_counts_nothing = (await stores.runStoreOp(store, "count", {}, run, node())).count === 0;
  const seeded = Array.from({ length: WALK_KEYS }, (_, index) => ({
    key: `${index % 2 === 0 ? "p" : "q"}/${MIXED[index % MIXED.length]}${index}`,
    value: { text: `value ${index}` },
    version: index + 1,
  }));
  const started = Date.now();
  await bulkSeed(store, "global", seeded);
  answer.walk_seed_ms = Date.now() - started;
  const expected = seeded.map((row) => row.key).sort(byBytes);
  const byKey = new Map(seeded.map((row) => [row.key, row]));

  const walked = Date.now();
  const pages = await walk(store, run, { limit: 1000 });
  answer.walk_ms = Date.now() - walked;
  answer.walk_pages = pages.length;
  const seen = pages.flatMap((answered) => answered.keys);
  // Gapless, duplicate-free and in order, as one comparison: the pages laid
  // end to end are the sorted key set, so a key a page boundary skipped, a key
  // two pages both answered and a page in the wrong order all fail it.
  answer.a_walk_sees_every_key_exactly_once_in_byte_order = same(seen, expected);
  answer.a_walk_answers_no_key_twice = new Set(seen).size === seen.length && seen.length === WALK_KEYS;
  // Every page but the last is full, and its `next` is its own last key —
  // which is the cursor the next page took.
  answer.every_page_but_the_last_is_full_and_its_next_is_its_last_key = pages
    .slice(0, -1)
    .every((answered) => answered.keys.length === 1000 && answered.next === answered.keys.at(-1));
  // 100,000 is a multiple of 1000, so the last page is as full as the rest and
  // still answers no `next`: "more remain" is asked of the store, not read off
  // a page that came back full.
  const last = pages.at(-1);
  answer.the_last_page_answers_no_next = last !== undefined && !("next" in last);
  answer.a_full_last_page_answers_no_next =
    WALK_KEYS % 1000 !== 0 || (last?.keys.length === 1000 && !("next" in last));
  answer.a_page_without_include_values_answers_no_entries = pages.every((answered) => !("entries" in answered));

  // …under a prefix, at a limit that divides nothing.
  const prefixed = await walk(store, run, { prefix: "p/", limit: 777 });
  const underP = expected.filter((key) => key.startsWith("p/"));
  answer.a_walk_under_a_prefix_sees_every_prefixed_key_exactly_once = same(
    prefixed.flatMap((answered) => answered.keys),
    underP,
  );
  answer.a_prefixed_walks_last_page_is_short_and_answers_no_next =
    !("next" in prefixed.at(-1)) && prefixed.at(-1).keys.length === underP.length % 777;

  // `count` agrees with the walk: the whole partition, one prefix, and none.
  const counted = await stores.runStoreOp(store, "count", {}, run, node());
  const countedP = await stores.runStoreOp(store, "count", { prefix: "p/" }, run, node());
  const countedNone = await stores.runStoreOp(store, "count", { prefix: "nothing/" }, run, node());
  answer.count_agrees_with_the_walk =
    counted.count === seen.length && countedP.count === underP.length && countedNone.count === 0;
  answer.count_answers_an_integer = Number.isInteger(counted.count);

  // A walk carrying its values: every entry is its page's key, in its order,
  // with the value and the version that key holds.
  const valued = await walk(store, run, { prefix: "q/", limit: 1000, includeValues: true });
  answer.include_values_entries_carry_each_keys_value_and_version = valued.every(
    (answered) =>
      Array.isArray(answered.entries) &&
      answered.entries.length === answered.keys.length &&
      answered.entries.every(
        (entry, index) =>
          entry.key === answered.keys[index] &&
          same(entry.value, byKey.get(entry.key)?.value) &&
          entry.version === byKey.get(entry.key)?.version,
      ),
  );
  answer.a_valued_walk_sees_every_key_once = same(
    valued.flatMap((answered) => answered.keys),
    expected.filter((key) => key.startsWith("q/")),
  );

  // `get_many` of 1000 keys: present ones spread across the whole store, keys
  // it does not hold, and two asked for twice, in an order nothing sorted.
  const present = Array.from({ length: 990 }, (_, index) => seeded[(index * 97) % WALK_KEYS].key);
  const missing = Array.from({ length: 8 }, (_, index) => `missing/${index}`);
  const asked = [...present.slice(0, 500), ...missing, present[3], ...present.slice(500), present[700]].reverse();
  answer.get_many_asked = asked.length;
  const many = await stores.runStoreOp(store, "get_many", { keys: asked }, run, node());
  answer.get_many_of_1000_keys_answers_one_entry_per_key_in_order =
    Array.isArray(many.entries) &&
    many.entries.length === 1000 &&
    many.entries.every((entry, index) => entry.key === asked[index]);
  answer.get_many_answers_found_with_value_and_version_or_neither = many.entries.every((entry) => {
    const held = byKey.get(entry.key);
    return held === undefined
      ? entry.found === false && !("value" in entry) && !("version" in entry)
      : entry.found === true && same(entry.value, held.value) && entry.version === held.version;
  });
  answer.get_many_answers_a_key_asked_twice_twice =
    many.entries.filter((entry) => entry.key === present[3]).length === 2;
  answer.get_many_of_no_keys_answers_no_entries = same(
    await stores.runStoreOp(store, "get_many", { keys: [] }, run, node()),
    { entries: [] },
  );

  // …and 1001 is refused, by name and before anything was read or recorded:
  // the node fails, and its own `retry:`/`on_error:` decide what happens.
  const before = run.storeRecords.length;
  try {
    await stores.runStoreOp(store, "get_many", { keys: [...asked, "one-too-many"] }, run, node());
    answer.get_many_refusal = null;
  } catch (error) {
    answer.get_many_refusal = String(error?.message ?? error).replaceAll(store.address, "<store>");
  }
  answer.a_refused_get_many_files_no_record = run.storeRecords.length === before;
} finally {
  // A hundred thousand rows are given back however the walk went, and a
  // cleanup that failed says so beside the cases rather than in place of them.
  try {
    answer.the_walk_gives_its_rows_back = await forget(WALKED);
  } catch (error) {
    answer.walk_cleanup_error = String(error?.message ?? error);
  }
}

{
  // **The order, the same on every arm**, for a set whose byte order differs
  // from every other order a comparison could answer in: case, a space, the
  // accents, two-, three- and four-byte characters. Written in an order
  // nothing sorted, and walked three at a time — so the cursor, not one
  // `ORDER BY`, is what has to agree.
  const store = kv("paged_order", "global");
  const run = ctx(execution("paged_order"));
  const mixed = ["é", "Z", "😀", "a b", "B", "ab", "中", "a", "ß", "z", "Ω", "A", "＀", "aB", "É", "b"];
  for (const [index, key] of mixed.entries()) await set(store, key, { n: index }, run, `po/${index}`);
  answer.paged_order = (await walk(store, run, { limit: 3 })).flatMap((answered) => answered.keys);
  // A cursor is a position in the order, not a key the store has to hold.
  answer.after_a_key_the_store_does_not_hold = (await page(store, run, { limit: 3, after: "aA" })).keys;
  answer.after_the_last_key_answers_an_empty_last_page = same(await page(store, run, { limit: 3, after: "😀" }), {
    keys: [],
  });
}

{
  // **A write landing mid-walk** moves no page the walk has yet to read: a key
  // added behind the cursor is not seen, one added ahead of it is, one deleted
  // ahead of it is not, and nothing already answered comes back — which is
  // what a keyset cursor gives and an offset would not.
  const store = kv("moving_walk", "global");
  const run = ctx(execution("moving_walk"));
  const keys = Array.from({ length: 10 }, (_, index) => `k${String(index + 1).padStart(2, "0")}`);
  for (const [index, key] of keys.entries()) await set(store, key, { n: index }, run, `mw/${index}`);
  const first = await page(store, run, { limit: 3 });
  await set(store, "k00", { n: -1 }, run, "mw/behind");
  await set(store, "k055", { n: 55 }, run, "mw/ahead");
  await stores.runStoreOp(store, "delete", { key: "k07" }, run, node("mw/deleted"));
  const rest = await walk(store, run, { limit: 3, after: first.next });
  // `walk` starts from no cursor; this one starts from the first page's.
  const seen = [...first.keys, ...rest.flatMap((answered) => answered.keys)];
  answer.a_write_mid_walk_moves_no_later_page = same(seen, [
    "k01",
    "k02",
    "k03",
    "k04",
    "k05",
    "k055",
    "k06",
    "k08",
    "k09",
    "k10",
  ]);
}

{
  // **The trace.** A page, a `count` and a `get_many` are reads: each files a
  // record with `effect: "read"` and the answer a replay consumes, and none of
  // them addresses one key or carries an idempotency key — not even from a
  // node site that hands one to every op it runs, which the emitted graph does
  // (PRD resolved q29, q70; `docs/trace.md` §6).
  const store = kv("read_records", "global");
  const run = ctx(execution("read_records"));
  await set(store, "r1", { n: 1 }, run, "rr/1");
  run.storeRecords.length = 0;
  const listed = await page(store, run, { limit: 5, after: "r0", includeValues: true, site: "rr/list" });
  const counted = await stores.runStoreOp(store, "count", { prefix: "r" }, run, node("rr/count"));
  const fetched = await stores.runStoreOp(store, "get_many", { keys: ["r1", "r2"] }, run, node("rr/many"));
  const [listRecord, countRecord, manyRecord] = run.storeRecords;
  const isRead = (record, op, answered) =>
    record?.op === op &&
    record?.effect === "read" &&
    record?.via === "node" &&
    same(record?.answer, answered) &&
    !("key" in record) &&
    !("idempotencyKey" in record) &&
    !("deduped" in record) &&
    !("applied" in record);
  answer.a_list_page_record_is_a_read_with_no_key = isRead(listRecord, "list", listed);
  answer.a_count_record_is_a_read_with_no_key = isRead(countRecord, "count", counted);
  answer.a_get_many_record_is_a_read_with_no_key = isRead(manyRecord, "get_many", fetched);
  answer.the_reads_wrote_no_ledger_row =
    (await set(store, "r1", { n: 2 }, run, "rr/count")).deduped === false &&
    (await set(store, "r1", { n: 3 }, run, "rr/many")).deduped === false;
}

{
  // **Replay.** A resumed generation answers a page, a `count` and a
  // `get_many` out of the record rather than out of the live store, exactly as
  // it answers a `get` (PRD resolved q29) — so the world moving between the
  // two generations changes nothing the replay hands the graph.
  const handle = await journal.openJournal();
  const store = kv("replayed_reads", "global");
  const id = execution("replay_reads");
  await handle.begin({
    id,
    flow: "flow.probe",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  const live = ctx(id);
  const reads = async () => ({
    counted: await stores.runStoreOp(
      store,
      "count",
      { prefix: "r/" },
      ctx(id, "", journal.recorderFor(id, "flow.probe/0#count")),
      node(),
    ),
    fetched: await stores.runStoreOp(
      store,
      "get_many",
      { keys: ["r/1", "r/2"] },
      ctx(id, "", journal.recorderFor(id, "flow.probe/0#many")),
      node(),
    ),
    paged: await page(store, ctx(id, "", journal.recorderFor(id, "flow.probe/0#page")), {
      limit: 1,
      prefix: "r/",
      includeValues: true,
    }),
  });

  journal.openSession(id, handle, false);
  await set(store, "r/1", { generation: "first" }, live, "rp/seed/1");
  const recorded = await reads();
  journal.closeSession(id);

  // The world moves under it, out of band and with nothing recording: a key
  // the `count` would now count, the `get_many` find, and the page answer as
  // its `next`.
  await set(store, "r/2", { generation: "moved" }, live, "rp/seed/2");
  await set(store, "r/0", { generation: "moved" }, live, "rp/seed/0");

  journal.openSession(id, handle, true);
  const replayed = await reads();
  journal.closeSession(id);

  answer.a_replayed_count_answers_out_of_the_record =
    recorded.counted.count === 1 &&
    same(replayed.counted, recorded.counted) &&
    (await stores.runStoreOp(store, "count", { prefix: "r/" }, live, node())).count === 3;
  answer.a_replayed_get_many_answers_out_of_the_record =
    recorded.fetched.entries[1]?.found === false && same(replayed.fetched, recorded.fetched);
  answer.a_replayed_page_answers_out_of_the_record =
    same(recorded.paged.keys, ["r/1"]) && !("next" in recorded.paged) && same(replayed.paged, recorded.paged);
  await journal.releaseJournal();
}

// --- Scope: three partitions, keyed as the local backend keys them -----------

{
  const store = kv("scoped_exec", "execution");
  const first = ctx(execution("s_e1"));
  const second = ctx(execution("s_e2"));
  await set(store, "k", { whose: "first" }, first, "e/1");
  await set(store, "k", { whose: "second" }, second, "e/2");
  const mine = await get(store, "k", first);
  const theirs = await get(store, "k", second);
  answer.executions_do_not_share_an_execution_scoped_store =
    mine.value?.whose === "first" && theirs.value?.whose === "second";

  // …and it dies with the run. On a dialled backend that is a `DELETE` rather
  // than a closed handle, made on the way out of `runFlow` and not awaited, so
  // the read is retried until it answers or the wait runs out.
  stores.releaseExecution(first.execution.id);
  let gone = false;
  for (let attempt = 0; attempt < 40 && !gone; attempt += 1) {
    gone = (await get(store, "k", first)).found === false;
    if (!gone) await new Promise((resolve) => setTimeout(resolve, 50));
  }
  answer.an_execution_scoped_store_dies_with_the_run = gone;
  // …and the run that did not end still has its own.
  answer.another_execution_keeps_its_own_partition =
    (await get(store, "k", second)).value?.whose === "second";
}

{
  const store = kv("scoped_session", "session");
  const mine = ctx(execution("s_s1"), "session-a");
  const later = ctx(execution("s_s2"), "session-a");
  const other = ctx(execution("s_s3"), "session-b");
  await set(store, "note", { text: "mine" }, mine, "s/1");
  answer.one_session_is_shared_across_executions =
    (await get(store, "note", later)).value?.text === "mine";
  answer.sessions_do_not_share_a_session_scoped_store =
    (await get(store, "note", other)).found === false;
}

{
  const store = kv("scoped_global", "global");
  await set(store, "note", { text: "everyone" }, ctx(execution("s_g1")), "g/1");
  answer.a_global_store_is_shared_across_executions =
    (await get(store, "note", ctx(execution("s_g2")))).value?.text === "everyone";
}

// --- The idempotency key of grammar 9.4, enforced by the backend -------------

{
  const store = kv("dedupe", "global");
  const run = ctx(execution("dedupe"));
  const key = `${execution("dedupe")}/write/0`;
  const first = await set(store, "k", { attempt: "first" }, run, key);
  // The redelivery: the same effect site, carrying the key its first attempt
  // carried. It must apply **once** — the answer is the first attempt's, whole,
  // marked `deduped`, and the stored value has not moved (PRD resolved q68).
  const second = await set(store, "k", { attempt: "REWRITTEN" }, run, key);
  answer.a_double_delivered_write_lands_once =
    first.deduped === false &&
    second.deduped === true &&
    same({ ...first, deduped: true }, second) &&
    (await get(store, "k", run)).value?.attempt === "first";
  answer.the_second_delivery_is_recorded_as_deduped = same(
    run.storeRecords.filter((record) => record.idempotencyKey === key).map((r) => r.deduped),
    [false, true],
  );
  // …and a write under a *different* key is a different effect.
  await set(store, "k", { attempt: "third" }, run, `${key}-again`);
  answer.a_write_under_another_key_is_another_effect =
    (await get(store, "k", run)).value?.attempt === "third";
}

{
  // The same key delivered **at once over two connections**, which is the half
  // the block above cannot reach and the half that tells the two possible
  // implementations apart.
  //
  // Those two attempts run through one binding, so `#serial` puts them on one
  // queue on one socket and the first has committed before the second looks: a
  // dedupe that *read* the ledger and then wrote it would answer them exactly as
  // the real one does. Resolved q63 moved the dedupe from the caller to the
  // backend — "a repeated write carrying the key the first attempt carried
  // applies once, stated and tested rather than assumed of the caller" — and
  // what the server is doing that a read-then-write is not only shows when the
  // two attempts are two transactions in flight together: the claim is an
  // `INSERT` on the ledger's primary key whose conflict clause changes no row,
  // so exactly one of them can be the one that applied. Two readers of an empty
  // ledger would both apply.
  //
  // Redelivery really does arrive this way. Grammar 9.4's key is
  // `<execution id>/<instance path>` (PRD 5.8), so a retried delivery and the
  // attempt it is retrying are one key issued by two processes — and grammar
  // 14.1 rule 5 admits a dialled store from a placement precisely so that those
  // processes can be more than one.
  //
  // On the local backend the two bindings are one store by design (a local
  // provider names no `url:`), and the case still says what it says about the
  // ledger: `transact`'s body is synchronous, so the second attempt cannot begin
  // inside the first.
  const mine = kv("dedupe_race", "global", URL_ENV);
  const theirs = kv("dedupe_race", "global", OTHER_URL_ENV);
  const id = execution("dedupe_race");
  const key = `${id}/write/0`;
  // A context each, because the answers have to be told apart: `set` derives its
  // row from the key alone, so what says which attempt applied is the `deduped`
  // its own trace record carries.
  const mineRun = ctx(id);
  const theirsRun = ctx(id);
  await Promise.all([
    set(mine, "k", { attempt: "mine" }, mineRun, key),
    set(theirs, "k", { attempt: "theirs" }, theirsRun, key),
  ]);
  const deduped = [mineRun, theirsRun].map(
    (run) => run.storeRecords.find((record) => record.idempotencyKey === key)?.deduped,
  );
  answer.a_concurrent_double_delivery_is_deduped_by_the_backend = same([...deduped].sort(), [
    false,
    true,
  ]);
  // …and one value landed: the deduplicated attempt's effect is rolled back with
  // the claim it lost, so what is stored is the applying attempt's value and not
  // whichever write happened to run second.
  const applied = deduped[0] === false ? "mine" : "theirs";
  answer.a_concurrent_double_delivery_leaves_the_applied_value =
    (await get(mine, "k", ctx(id))).value?.attempt === applied;
}

// --- Versions, conditions and partial updates (PRD resolved q68) ------------

/** The value schema the partial-update cases hold their results to. */
const RECORD = z
  .object({
    title: z.string(),
    tags: z.array(z.string()),
    count: z.number().int(),
    note: z.string().optional(),
  })
  .strict();

/** A write with a condition, as a store-op node issues one. */
const write = (store, op, params, run, key_ = undefined) =>
  stores.runStoreOp(store, op, params, run, node(key_));

/** What a promise rejected with, or `undefined` when it settled. */
const refusal = async (promise) => {
  try {
    await promise;
    return undefined;
  } catch (error) {
    return String(error?.message ?? error);
  }
};

{
  // A version is a per-partition **sequence**, not a per-key counter: every
  // landed write takes the next value, so a key deleted and set again never
  // shows a version it showed before — the ABA race a compare-and-set exists
  // to close stays closed across a delete.
  const store = kv("versions", "global");
  const run = ctx(execution("versions"));
  const first = await set(store, "k", { n: 1 }, run, "v/1");
  const read = await get(store, "k", run);
  const second = await set(store, "k", { n: 2 }, run, "v/2");
  const other = await set(store, "other", { n: 0 }, run, "v/3");
  const removed = await write(store, "delete", { key: "k" }, run, "v/4");
  const again = await set(store, "k", { n: 3 }, run, "v/5");
  answer.a_set_answers_applied_and_the_version_it_took =
    first.applied === true &&
    Number.isInteger(first.version) &&
    first.deduped === false &&
    first.key === "k";
  answer.a_get_answers_the_version_its_key_holds =
    read.found === true && read.version === first.version;
  answer.a_miss_answers_no_version = !("version" in (await get(store, "nothing", run)));
  answer.versions_rise_with_every_write_to_a_partition =
    second.version > first.version && other.version > second.version;
  answer.a_version_is_never_reused_across_a_delete =
    removed.applied === true &&
    removed.deleted === true &&
    again.version > other.version &&
    ![first.version, second.version, other.version].includes(again.version);
  answer.version_trace = [first.version, second.version, other.version, again.version];
}

{
  // `if: absent` and `if: present`, and what a condition that does not hold
  // answers: `applied: false`, the key's current version where it has one, and
  // nothing written — a result an edge routes on rather than a failure.
  const store = kv("conditions", "global");
  const run = ctx(execution("conditions"));
  const claimed = await write(store, "set", { key: "job", value: { owner: "a" }, condition: "absent" }, run, "c/1");
  const lost = await write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent" }, run, "c/2");
  const held = await get(store, "job", run);
  answer.if_absent_applies_once_and_then_declines =
    claimed.applied === true &&
    lost.applied === false &&
    lost.version === claimed.version &&
    held.value?.owner === "a" &&
    held.version === claimed.version;
  const nobody = await write(store, "set", { key: "nobody", value: { owner: "c" }, condition: "present" }, run, "c/3");
  answer.if_present_declines_an_absent_key_with_no_version =
    nobody.applied === false &&
    !("version" in nobody) &&
    (await get(store, "nobody", run)).found === false;
  const updated = await write(store, "set", { key: "job", value: { owner: "a2" }, condition: "present" }, run, "c/4");
  answer.if_present_applies_to_a_key_that_holds_a_value =
    updated.applied === true && updated.version > claimed.version;
}

{
  // `if_version:` — compare-and-set on the version a read answered.
  const store = kv("cas", "global");
  const run = ctx(execution("cas"));
  const start = await set(store, "k", { n: 1 }, run, "cas/1");
  const won = await write(store, "set", { key: "k", value: { n: 2 }, ifVersion: start.version }, run, "cas/2");
  const stale = await write(store, "set", { key: "k", value: { n: 3 }, ifVersion: start.version }, run, "cas/3");
  const held = await get(store, "k", run);
  answer.if_version_applies_when_the_version_matches =
    won.applied === true && won.version > start.version;
  answer.if_version_declines_a_stale_version_and_answers_the_current_one =
    stale.applied === false && stale.version === won.version && held.value?.n === 2;
  const absent = await write(store, "set", { key: "nothing", value: { n: 0 }, ifVersion: 0 }, run, "cas/4");
  answer.if_version_declines_an_absent_key =
    absent.applied === false && !("version" in absent);
  const kept = await write(store, "delete", { key: "k", ifVersion: start.version }, run, "cas/5");
  const stillThere = await get(store, "k", run);
  const gone = await write(store, "delete", { key: "k", ifVersion: won.version }, run, "cas/6");
  answer.a_conditional_delete_declines_a_stale_version_and_applies_a_current_one =
    kept.applied === false &&
    kept.deleted === false &&
    stillThere.found === true &&
    gone.applied === true &&
    gone.deleted === true &&
    (await get(store, "k", run)).found === false;
  // A declined delete answers the version the key still holds, exactly as a
  // declined `set` does — it is what the next compare-and-set needs — and one
  // that applied answers none, because the key then holds nothing (PRD
  // resolved q68 (b), (d)).
  const nothing = await write(store, "delete", { key: "k", ifVersion: won.version }, run, "cas/7");
  answer.a_declined_delete_answers_the_version_the_key_holds =
    kept.version === won.version &&
    stillThere.version === won.version &&
    !("version" in gone) &&
    nothing.applied === false &&
    !("version" in nothing);
  answer.declined_delete = kept;
}

{
  // The ledger stores the op's **whole** answer, so a retried conditional
  // write — the same key, the same attempt — is answered with its first
  // attempt's `applied` and `version`, `deduped: true`, and never fails against
  // its own first write.
  const store = kv("retried_claim", "global");
  const run = ctx(execution("retried_claim"));
  const key = `${execution("retried_claim")}/claim/0`;
  const first = await write(store, "set", { key: "job", value: { owner: "me" }, condition: "absent" }, run, key);
  const retried = await write(store, "set", { key: "job", value: { owner: "me" }, condition: "absent" }, run, key);
  answer.a_retried_claim_answers_applied_and_deduped =
    first.applied === true &&
    first.deduped === false &&
    retried.applied === true &&
    retried.deduped === true &&
    retried.version === first.version;
  // …and over two connections at once, where the ledger's primary key is what
  // arbitrates.
  const mine = kv("retried_race", "global", URL_ENV);
  const theirs = kv("retried_race", "global", OTHER_URL_ENV);
  const raceKey = `${execution("retried_race")}/claim/0`;
  const both = await Promise.all([
    write(mine, "set", { key: "job", value: { owner: "x" }, condition: "absent" }, ctx(execution("retried_race")), raceKey),
    write(theirs, "set", { key: "job", value: { owner: "x" }, condition: "absent" }, ctx(execution("retried_race")), raceKey),
  ]);
  answer.a_concurrently_retried_claim_is_applied_once_and_answered_twice =
    both.every((row) => row.applied === true) &&
    both[0].version === both[1].version &&
    same(both.map((row) => row.deduped).sort(), [false, true]);
}

{
  // …and the other half of "the answer its first attempt gave": a retried
  // write whose first attempt **declined** is answered with that decline —
  // `applied: false`, the version the key held then, `deduped: true` — even
  // where the key has since been freed and the same request made fresh would
  // apply. The ledger answers the attempt, not the key, so a losing claim
  // retried after the winner let go does not quietly become a second winner
  // (PRD resolved q68 (d)). A declined partial update is the same: its retry
  // answers the decline, and writes nothing, whatever the key holds by then.
  const store = kv("retried_decline", "global");
  const run = ctx(execution("retried_decline"));
  const losing = `${execution("retried_decline")}/claim/b`;
  const winner = await write(store, "set", { key: "job", value: { owner: "a" }, condition: "absent" }, run, "rdc/a");
  const lost = await write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent" }, run, losing);
  const freed = await write(store, "delete", { key: "job" }, run, "rdc/free");
  const retried = await write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent" }, run, losing);
  answer.a_retried_declined_write_answers_its_first_decline =
    winner.applied === true &&
    lost.applied === false &&
    lost.version === winner.version &&
    lost.deduped === false &&
    freed.applied === true &&
    retried.applied === false &&
    retried.version === winner.version &&
    retried.deduped === true &&
    (await get(store, "job", run)).found === false;
  answer.retried_decline = retried;

  const records = kv("retried_decline_merge", "global", URL_ENV, RECORD);
  const merging = `${execution("retried_decline")}/merge/0`;
  const whole = { title: "t", tags: [], count: 0 };
  const missed = await write(records, "merge", { key: "r", fields: whole, condition: "present" }, run, merging);
  const seeded = await set(records, "r", { title: "seeded", tags: [], count: 1 }, run, "rdc/seed");
  const remissed = await write(records, "merge", { key: "r", fields: whole, condition: "present" }, run, merging);
  const held = await get(records, "r", run);
  answer.a_retried_declined_partial_update_writes_nothing =
    missed.applied === false &&
    !("version" in missed) &&
    remissed.applied === false &&
    remissed.deduped === true &&
    !("version" in remissed) &&
    held.version === seeded.version &&
    held.value?.title === "seeded";
}

{
  // `incr`, `append`, `merge` on a key that holds nothing: an `incr` starts
  // from 0, an `append` from [], a `merge` creates the value and must satisfy
  // the schema whole.
  const store = kv("partials", "global", URL_ENV, RECORD);
  const run = ctx(execution("partials"));
  const created = await write(store, "merge", { key: "r", fields: { title: "t", tags: [], count: 0 } }, run, "p/1");
  const appended = await write(store, "append", { key: "r", field: "tags", value: "x" }, run, "p/2");
  const twice = await write(store, "append", { key: "r", field: "tags", value: "y" }, run, "p/3");
  const unique = await write(store, "append", { key: "r", field: "tags", value: "x", unique: true }, run, "p/4");
  const bumped = await write(store, "incr", { key: "r", field: "count", by: 5 }, run, "p/5");
  const merged = await write(store, "merge", { key: "r", fields: { note: "kept" } }, run, "p/6");
  const held = await get(store, "r", run);
  answer.a_merge_creates_a_value_that_satisfies_the_schema =
    created.applied === true && same(created.value, { title: "t", tags: [], count: 0 });
  answer.an_append_adds_one_item_to_the_end =
    same(appended.value?.tags, ["x"]) && same(twice.value?.tags, ["x", "y"]);
  answer.a_unique_append_of_a_held_item_changes_nothing =
    unique.applied === true && unique.version === twice.version && same(unique.value?.tags, ["x", "y"]);
  answer.an_incr_adds_by_to_the_field = bumped.value?.count === 5;
  answer.a_merge_keeps_every_field_it_does_not_name =
    same(merged.value, { title: "t", tags: ["x", "y"], count: 5, note: "kept" }) &&
    same(held.value, merged.value) &&
    held.version === merged.version;

  // An `incr` on a key holding nothing at all, against a schema it alone
  // satisfies, starts from 0.
  const counter = kv("counter_only", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  const one = await write(counter, "incr", { key: "c", field: "count" }, run, "p/7");
  const three = await write(counter, "incr", { key: "c", field: "count", by: 2 }, run, "p/8");
  answer.an_incr_from_absent_starts_at_zero =
    one.applied === true && one.value?.count === 1 && three.value?.count === 3;
  const list = kv("list_only", "global", URL_ENV, z.object({ items: z.array(z.string()) }).strict());
  const firstItem = await write(list, "append", { key: "l", field: "items", value: "a" }, run, "p/9");
  answer.an_append_from_absent_starts_from_an_empty_array = same(firstItem.value, { items: ["a"] });
}

{
  // A merge whose result the schema refuses fails — and leaves the row exactly
  // as it was, version included. The refusal names the field.
  const store = kv("refused", "global", URL_ENV, RECORD);
  const run = ctx(execution("refused"));
  const start = await set(store, "r", { title: "t", tags: ["a"], count: 1 }, run, "x/1");
  const failed = await refusal(write(store, "merge", { key: "r", fields: { count: "not a number" } }, run, "x/2"));
  const held = await get(store, "r", run);
  answer.a_merge_the_schema_refuses_fails_naming_the_field =
    typeof failed === "string" && failed.includes("count") && failed.includes("value_schema");
  answer.a_merge_the_schema_refuses_leaves_the_row_unchanged =
    same(held.value, { title: "t", tags: ["a"], count: 1 }) && held.version === start.version;
  answer.refused_merge = failed ?? null;
  // On an absent key a merge has to supply the whole value.
  const partial = await refusal(write(store, "merge", { key: "fresh", fields: { title: "only" } }, run, "x/3"));
  answer.a_merge_on_an_absent_key_must_satisfy_the_schema_whole =
    typeof partial === "string" && partial.includes("tags") && (await get(store, "fresh", run)).found === false;
  // A failed op is not a ledger entry: nothing was applied, so a retry under
  // the same key is a fresh attempt rather than a replay of the failure.
  const retried = await write(store, "merge", { key: "r", fields: { count: 2 } }, run, "x/2");
  answer.a_refused_merge_leaves_no_ledger_row = retried.applied === true && retried.deduped === false;
}

{
  // Decision D119's split, read at the op: a partial update whose result `V`
  // refuses is the **author's** failure on a store-op node — a plain error,
  // which the node's `retry:`/`on_error:` decide — and the **model's** on a
  // synthesized tool, a `ToolCallRefused` the agent loop hands back to it. One
  // op, one backend, one refusal; only who asked differs. So does an `item`
  // naming other than one field, which the tool refuses before the store sees
  // it. Every refusal here leaves the row as it was.
  const store = kv("tool_refusals", "global", URL_ENV, z.object({ items: z.array(z.string()).max(1) }).strict());
  const run = ctx(execution("tool_refusals"));
  const start = await set(store, "r", { items: ["only"] }, run, "tr/0");
  const caught = async (promise) => {
    try {
      await promise;
      return undefined;
    } catch (error) {
      return error;
    }
  };
  const fromNode = await caught(write(store, "append", { key: "r", field: "items", value: "one too many" }, run, "tr/1"));
  const fromTool = await caught(stores.runStoreTool(store, "append", { key: "r", item: { items: "one too many" } }, run));
  const merged = await caught(stores.runStoreTool(store, "merge", { key: "fresh", fields: {} }, run));
  const none = await caught(stores.runStoreTool(store, "append", { key: "r", item: {} }, run));
  const two = await caught(stores.runStoreTool(store, "append", { key: "r", item: { items: "a", other: "b" } }, run));
  const held = await get(store, "r", run);
  const toModel = (error) => error instanceof runtime.ToolCallRefused;
  answer.a_refused_partial_update_fails_a_node_and_goes_back_to_a_model =
    fromNode instanceof Error &&
    !toModel(fromNode) &&
    String(fromNode.message).includes("value_schema") &&
    toModel(fromTool) &&
    String(fromTool.message).includes("value_schema") &&
    toModel(merged) &&
    toModel(none) &&
    String(none.message).includes("naming no field") &&
    toModel(two) &&
    String(two.message).includes("naming 2 fields") &&
    same(held.value, { items: ["only"] }) &&
    held.version === start.version &&
    (await get(store, "fresh", run)).found === false;
  answer.tool_refusal_classes = [fromNode, fromTool, merged, none, two].map((error) =>
    error === undefined ? "none" : `${error.name}: ${String(error.message).slice(0, 60)}`,
  );
}

{
  // Schema evolution, written down: a value stored under one schema and read
  // by a partial update under another is validated whole, so the op fails
  // naming the field the stored value lacks — `get` would have answered it as
  // stored.
  const before = kv("evolved", "global", URL_ENV, z.object({ title: z.string() }).strict());
  const after = kv("evolved", "global", URL_ENV, RECORD);
  const run = ctx(execution("evolved"));
  await set(before, "r", { title: "old" }, run, "e/1");
  const failed = await refusal(write(after, "incr", { key: "r", field: "count" }, run, "e/2"));
  answer.a_partial_update_under_a_changed_schema_fails_naming_the_field =
    typeof failed === "string" && failed.includes("tags");
  answer.a_get_under_a_changed_schema_answers_the_stored_value =
    same((await get(after, "r", run)).value, { title: "old" });
}

{
  // Two concurrent appends keep both items, on two connections at once.
  const mine = kv("append_race", "global", URL_ENV, z.object({ items: z.array(z.string()) }).strict());
  const theirs = kv("append_race", "global", OTHER_URL_ENV, z.object({ items: z.array(z.string()) }).strict());
  const run = ctx(execution("append_race"));
  const items = Array.from({ length: 10 }, (_, index) => `item-${index}`);
  await Promise.all(
    items.map((item, index) =>
      write(index % 2 === 0 ? mine : theirs, "append", { key: "shared", field: "items", value: item }, run, `ar/${index}`),
    ),
  );
  const held = await get(mine, "shared", run);
  answer.two_connections_appending_at_once_keep_every_item =
    same([...(held.value?.items ?? [])].sort(), [...items].sort());
}

{
  // …and on a server whose **default** isolation is stricter than the
  // `READ COMMITTED` the arms' writes are built for. A default is what a
  // database or a role can change (`ALTER DATABASE … SET
  // default_transaction_isolation`), and under a snapshot a write queued
  // behind another one's commit finds the row it waited for "concurrently
  // updated": on Postgres it fails with `40001` instead of serializing behind
  // it. So each dialled arm asserts the level on its own session (PRD resolved
  // q63: one contract on every arm; q68: the writes serialize rather than
  // fail). The Postgres connections here are opened with
  // `default_transaction_isolation=serializable` in their startup options; a
  // stock MySQL server already defaults to `REPEATABLE READ`, so its arm is
  // asked the same question by the server it is given. On SQLite the case is
  // the two-connection race above, which it has to answer the same way.
  const STRICT_URL_ENV = "AGENT_COMPOSE_STORE_URL_STRICT";
  const OTHER_STRICT_URL_ENV = "AGENT_COMPOSE_STORE_URL_STRICT_B";
  if (provider !== "sqlite") {
    const strict = new URL(process.env[URL_ENV]);
    if (provider === "postgres") {
      strict.searchParams.set("options", "-c default_transaction_isolation=serializable");
    }
    process.env[STRICT_URL_ENV] = strict.href;
    process.env[OTHER_STRICT_URL_ENV] = strict.href;
  }
  const schema = z.object({ items: z.array(z.string()) }).strict();
  const mine = kv("strict_race", "global", STRICT_URL_ENV, schema);
  const theirs = kv("strict_race", "global", OTHER_STRICT_URL_ENV, schema);
  const run = ctx(execution("strict_race"));
  const items = Array.from({ length: 20 }, (_, index) => `item-${index}`);
  const settled = await Promise.allSettled(
    items.map((item, index) =>
      write(index % 2 === 0 ? mine : theirs, "append", { key: "shared", field: "items", value: item }, run, `sr/${index}`),
    ),
  );
  const failures = settled
    .filter((outcome) => outcome.status === "rejected")
    .map((outcome) => String(outcome.reason?.message ?? outcome.reason));
  const held = await get(mine, "shared", run);
  answer.writes_serialize_whatever_isolation_the_server_defaults_to =
    failures.length === 0 && same([...(held.value?.items ?? [])].sort(), [...items].sort());
  answer.strict_isolation_failures = failures.slice(0, 3);
}

{
  // A tool write carries no idempotency key and so no `deduped` on its record,
  // whichever op it ran; a node write's record carries `applied` and `version`
  // beside them.
  const store = kv("records", "global", URL_ENV, RECORD);
  const run = ctx(execution("records"));
  await write(store, "set", { key: "r", value: { title: "t", tags: [], count: 0 }, condition: "absent" }, run, "rec/1");
  await stores.runStoreOp(store, "incr", { key: "r", field: "count" }, run, { via: "tool" });
  const [fromNode, fromTool] = run.storeRecords;
  answer.a_node_write_record_carries_applied_version_and_its_key =
    fromNode?.applied === true &&
    Number.isInteger(fromNode?.version) &&
    fromNode?.idempotencyKey === "rec/1" &&
    fromNode?.deduped === false;
  answer.a_tool_write_record_carries_applied_and_version_but_no_key =
    fromTool?.via === "tool" &&
    fromTool?.applied === true &&
    fromTool?.version > fromNode?.version &&
    !("idempotencyKey" in (fromTool ?? {})) &&
    !("deduped" in (fromTool ?? {}));
}

{
  // **Two processes**, each with its own handle or its own server connection,
  // racing one store from one agreed instant. See `store-race.mjs`.
  const race = path.join(path.dirname(fileURLToPath(import.meta.url)), "store-race.mjs");
  const startAt = Date.now() + 2_500;
  const spawned = ["a", "b"].map(
    (role) =>
      new Promise((resolve, reject) => {
        const child = spawn(process.execPath, [race, project, provider, data, RUN, role, String(startAt)], {
          env: process.env,
          stdio: ["ignore", "pipe", "pipe"],
        });
        let out = "";
        let err = "";
        child.stdout.on("data", (chunk) => (out += chunk));
        child.stderr.on("data", (chunk) => (err += chunk));
        child.on("error", reject);
        child.on("close", (code) =>
          code === 0 ? resolve(JSON.parse(out)) : reject(new Error(`racer ${role} exited ${code}: ${err}`)),
        );
      }),
  );
  try {
    const [a, b] = await Promise.all(spawned);
    const rounds = a.claims.length;
    const claimStore = kv("race_claims", "global");
    const winners = [];
    let exactlyOne = rounds > 0 && rounds === b.claims.length;
    for (let round = 0; round < rounds; round += 1) {
      const applied = [a.claims[round].applied, b.claims[round].applied].filter(Boolean).length;
      const held = await get(claimStore, `claim-${round}`, ctx(execution("race_read")));
      const winner = a.claims[round].applied ? "a" : "b";
      winners.push(winner);
      if (applied !== 1 || held.value?.owner !== winner) exactlyOne = false;
    }
    answer.two_processes_claiming_one_key_get_exactly_one_applied = exactlyOne;
    answer.race_winners = winners.join("");
    // Every partial update answers the whole value it left at the version it
    // took, so the two processes' answers, put in version order, are the key's
    // history replayed — and each one has to be the one before it with its
    // own change applied. That is a stronger reading than the final value:
    // a lost update (a write that read before the other's commit and wrote
    // after it) is invisible there whenever the loser writes again later, and
    // every process here writes again every round. Distinct versions are part
    // of it, since two writes taking one version leave no order to replay.
    const history = (pick) =>
      ["a", "b"]
        .flatMap((role) => pick(role === "a" ? a : b).map((row, round) => ({ role, round, row })))
        .sort((left, right) => left.row.version - right.row.version);
    const ordered = (rows) =>
      rows.every((entry) => entry.row.applied === true && Number.isInteger(entry.row.version)) &&
      new Set(rows.map((entry) => entry.row.version)).size === rows.length;
    const appended = history((side) => side.appends);
    const lists = await get(kv("race_lists", "global"), "shared", ctx(execution("race_read")));
    const expected = ["a", "b"].flatMap((role) => Array.from({ length: rounds }, (_, i) => `${role}-${i}`));
    answer.two_processes_appending_at_once_keep_every_item =
      same([...(lists.value?.items ?? [])].sort(), [...expected].sort()) &&
      ordered(appended) &&
      appended.every((entry, index) =>
        same(
          entry.row.value?.items,
          appended.slice(0, index + 1).map((earlier) => `${earlier.role}-${earlier.round}`),
        ),
      );
    const incremented = history((side) => side.incrs);
    const counters = await get(kv("race_counters", "global"), "shared", ctx(execution("race_read")));
    answer.two_processes_incrementing_at_once_lose_no_increment =
      counters.value?.count === rounds * 2 &&
      ordered(incremented) &&
      incremented.every((entry, index) => entry.row.value?.count === index + 1);
    const merged = history((side) => side.merges);
    const records = await get(kv("race_records", "global"), "shared", ctx(execution("race_read")));
    const latest = {};
    answer.two_processes_merging_different_fields_keep_both =
      records.value?.a === rounds - 1 &&
      records.value?.b === rounds - 1 &&
      ordered(merged) &&
      merged.every((entry) => {
        latest[entry.role] = entry.round;
        return entry.row.value?.a === latest.a && entry.row.value?.b === latest.b;
      });
    // Every write that landed in one partition took a distinct version, and
    // each process saw its own versions rise.
    const versions = [...a.appends, ...b.appends].map((row) => row.version);
    const rising = (rows) => rows.every((row, i) => i === 0 || row.version > rows[i - 1].version);
    answer.two_processes_never_take_one_version =
      new Set(versions).size === versions.length &&
      rising(a.appends) &&
      rising(b.appends) &&
      lists.version === Math.max(...versions);
  } catch (error) {
    answer.race_error = String(error?.message ?? error);
  }
}

{
  // **A write that arrives inside another write's read-modify-write window
  // waits for that write's commit, and builds on it.** The race above starts
  // its writes together, and on SQLite a write's transaction is a few
  // synchronous microseconds, so two racers almost never overlap inside one:
  // it would pass a store that read its key outside the lock it writes under.
  // Here the window is held open on purpose — `store-window.mjs` is a second
  // process whose merge validates against a schema that spins for a second and
  // a half, and a partial update validates between reading its key and writing
  // it, inside its transaction — and this process writes the same key only
  // once the other's marker says the window is open. Correctly serialized,
  // this write waits on the other's lock (`BEGIN IMMEDIATE`'s on SQLite, the
  // partition's sequence row on a server), reads what it committed, and keeps
  // both fields; a store that let it land inside the window would have the
  // other's write overwrite it a moment later.
  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "store-window.mjs");
  const marker = path.join(data, `${RUN}-window.marker`);
  const schema = z.object({ a: z.number().int().optional(), b: z.number().int().optional() }).strict();
  const mine = kv("window", "global", OTHER_URL_ENV, schema);
  const run = ctx(execution("window"));
  try {
    // Opened first, so this side's file or connection is not what it waits on.
    await get(mine, "warm", run);
    // A dialled arm writes nothing under the data directory, so it may not be
    // there yet; the marker is written into it.
    fs.mkdirSync(data, { recursive: true });
    const holder = new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [script, project, provider, data, RUN, marker], {
        env: process.env,
        stdio: ["ignore", "pipe", "pipe"],
      });
      let out = "";
      let err = "";
      child.stdout.on("data", (chunk) => (out += chunk));
      child.stderr.on("data", (chunk) => (err += chunk));
      child.on("error", reject);
      child.on("close", (code) =>
        code === 0 ? resolve(JSON.parse(out)) : reject(new Error(`the window holder exited ${code}: ${err}`)),
      );
    });
    // A rejection nothing is listening to yet would end this process, and the
    // holder can fail before the marker ever appears.
    const settled = holder.then(
      (row) => ({ row }),
      (error) => ({ error }),
    );
    const deadline = Date.now() + 30_000;
    let open = false;
    while (Date.now() < deadline) {
      if (fs.existsSync(marker)) {
        open = true;
        break;
      }
      const early = await Promise.race([settled, new Promise((resolve) => setTimeout(resolve, 5))]);
      if (early !== undefined) break;
    }
    answer.the_window_was_open_when_the_second_write_arrived = open;
    const late = open
      ? await write(mine, "merge", { key: "shared", fields: { b: 1 } }, run, `${RUN}/window/b`)
      : undefined;
    const outcome = await settled;
    if (outcome.error !== undefined) throw outcome.error;
    const early = outcome.row;
    const held = await get(mine, "shared", run);
    answer.a_write_inside_another_writes_window_waits_and_builds_on_it =
      open &&
      same(early.value, { a: 1 }) &&
      same(late?.value, { a: 1, b: 1 }) &&
      late.version > early.version &&
      same(held.value, { a: 1, b: 1 });
    answer.window = { early, late: late ?? null, held: held.value ?? null };
  } catch (error) {
    answer.window_error = String(error?.message ?? error);
  }
}

// --- Expiry (PRD resolved q69) ------------------------------------------------
//
// Every case here moves time with the store module's own clock hook rather than
// by sleeping: `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS` is read at every op and
// shifts the clock the arm already reads — `Date.now()` on SQLite, the server's
// clock inside the statement on a dialled one — so "an hour later" is one
// environment variable away, on every arm, and the margins can be minutes
// rather than the milliseconds a real wait would leave.

/** The variable `src/stores.ts` reads its clock offset from. */
const CLOCK = stores.STORE_CLOCK_OFFSET;

/** An hour: the `ttl` most cases below write, in milliseconds. */
const HOUR = 3_600_000;

/** A minute, which is every margin either side of an expiry below. */
const MINUTE = 60_000;

/** Run `work` with the store clock `offset` milliseconds ahead, and put it back. */
const later = async (offset, work) => {
  const before = process.env[CLOCK];
  process.env[CLOCK] = String(offset);
  try {
    return await work();
  } finally {
    if (before === undefined) delete process.env[CLOCK];
    else process.env[CLOCK] = before;
  }
};

/**
 * Rows of a store's storage as they really are — expired ones included —
 * answered by a connection this file opens itself, the way an operator would
 * look: SQLite's file with the pinned driver, a server over its own socket.
 * `sql` is written once with `?` placeholders over the dialled schema
 * (`store_entries`, `store_applied`), and the SQLite form drops the `store`
 * column, which a file per store does not have.
 */
async function storage(sql, parameters, sqliteSql = undefined, sqliteParameters = undefined) {
  if (provider === "sqlite") {
    const module = await import("node-sqlite3-wasm");
    const { Database } = module.default ?? module;
    const [file, statement, bound] = sqliteSql;
    const database = new Database(path.join(data, "stores", `${stores.encodeKey(file)}.sqlite`));
    try {
      database.exec("PRAGMA busy_timeout = 5000;");
      return database.all(statement, bound ?? sqliteParameters ?? []);
    } finally {
      database.close();
    }
  }
  const server = await openServer(process.env[URL_ENV]);
  try {
    let next = 0;
    const spelled = provider === "postgres" ? sql.replace(/\?/g, () => `$${++next}`) : sql;
    return await rows(server, spelled, parameters);
  } finally {
    await server.end().catch(() => undefined);
  }
}

/** How many rows one partition of a store holds in storage, expired ones included. */
async function storedRows(store, scopeKey) {
  const counted = await storage(
    "SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND scope_key = ?",
    [store.name, scopeKey],
    [store.name, "SELECT COUNT(*) AS n FROM entries WHERE scope_key = ?", [scopeKey]],
  );
  return Number(counted[0]?.n);
}

/** How many ledger rows a store holds under one key prefix. */
async function ledgerRows(store, prefix) {
  const counted = await storage(
    "SELECT COUNT(*) AS n FROM store_applied WHERE store = ? AND SUBSTR(idempotency_key, 1, ?) = ?",
    [store.name, prefix.length, prefix],
    [
      store.name,
      "SELECT COUNT(*) AS n FROM applied WHERE substr(idempotency_key, 1, ?) = ?",
      [prefix.length, prefix],
    ],
  );
  return Number(counted[0]?.n);
}

{
  // A key with a `ttl` is there until its expiry and absent from it on, to
  // `get` and to `list` alike — and absent the way a key never written is: no
  // `value`, no `version`.
  const store = kv("expiry", "global");
  const run = ctx(execution("expiry"));
  await later(0, () => write(store, "set", { key: "k", value: { n: 1 }, ttl: HOUR }, run, "e/1"));
  await later(0, () => write(store, "set", { key: "kept", value: { n: 2 } }, run, "e/2"));
  const before = await later(HOUR - MINUTE, () => get(store, "k", run));
  const listedBefore = await later(HOUR - MINUTE, () => list(store, run));
  const after = await later(HOUR + MINUTE, () => get(store, "k", run));
  const listedAfter = await later(HOUR + MINUTE, () => list(store, run));
  answer.a_key_with_a_ttl_is_readable_before_it_expires =
    before.found === true && before.value?.n === 1 && same(listedBefore.keys, ["k", "kept"]);
  answer.an_expired_key_is_absent_to_get_and_list =
    same(after, { found: false }) && same(listedAfter.keys, ["kept"]);

  // …and to PRD resolved q70's reads, which judge a key by the same
  // inequality a `get` does (`liveRows`): a `count` stops counting it, a
  // `get_many` answers it as a key never written, and a page skips it —
  // which is also what decides the page's `next`. With `limit: 1` the page
  // before the expiry is `k` with `kept` past it, so `next` is `k`; after it,
  // the extra row the page reads to know whether more remain is the expired
  // key itself, so a page that counted it would still answer a `next`. None
  // of these reads writes, so no sweep has run: the expired row is still
  // stored, and only the read's own filter keeps it out.
  const count = () => stores.runStoreOp(store, "count", {}, run, node());
  const many = () => stores.runStoreOp(store, "get_many", { keys: ["k", "kept"] }, run, node());
  const page = () => stores.runStoreOp(store, "list", { limit: 1, includeValues: true }, run, node());
  const countedBefore = await later(HOUR - MINUTE, count);
  const manyBefore = await later(HOUR - MINUTE, many);
  const pageBefore = await later(HOUR - MINUTE, page);
  const countedAfter = await later(HOUR + MINUTE, count);
  const manyAfter = await later(HOUR + MINUTE, many);
  const pageAfter = await later(HOUR + MINUTE, page);
  const storedAfter = await storedRows(store, "global");
  answer.an_expired_key_is_uncounted_by_count = countedBefore.count === 2 && countedAfter.count === 1;
  answer.an_expired_key_is_not_found_by_get_many =
    manyBefore.entries?.[0]?.found === true &&
    manyBefore.entries?.[0]?.value?.n === 1 &&
    same(manyAfter.entries?.[0], { key: "k", found: false }) &&
    manyAfter.entries?.[1]?.found === true &&
    manyAfter.entries?.[1]?.value?.n === 2;
  answer.an_expired_key_is_skipped_by_a_page_and_its_next =
    same(pageBefore.keys, ["k"]) &&
    pageBefore.next === "k" &&
    same(
      pageBefore.entries?.map((entry) => entry.key),
      ["k"],
    ) &&
    same(pageAfter.keys, ["kept"]) &&
    !("next" in pageAfter) &&
    same(
      pageAfter.entries?.map((entry) => [entry.key, entry.value?.n]),
      [["kept", 2]],
    );
  // The premise of the three: the expired row was still in storage when they
  // read past it.
  answer.the_expired_key_was_still_stored_when_those_reads_skipped_it = storedAfter === 2;
  answer.expiry_trace = {
    before,
    after,
    listedBefore,
    listedAfter,
    countedBefore,
    countedAfter,
    manyBefore,
    manyAfter,
    pageBefore,
    pageAfter,
    storedAfter,
  };
}

{
  // Every condition treats an expired key as absent: `if: present` and
  // `if_version:` decline it with no version to answer, and `if: absent`
  // applies and takes it — which, with a version, is a lease. Expiry consumes
  // no version: the key set again takes the partition's very next one.
  const store = kv("lease", "global");
  const run = ctx(execution("lease"));
  const claim = await later(0, () =>
    write(store, "set", { key: "job", value: { owner: "a" }, condition: "absent", ttl: HOUR }, run, "l/1"),
  );
  const held = await later(HOUR - MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent", ttl: HOUR }, run, "l/2"),
  );
  const present = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "c" }, condition: "present" }, run, "l/3"),
  );
  const versioned = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "c" }, ifVersion: claim.version }, run, "l/4"),
  );
  const retaken = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent", ttl: HOUR }, run, "l/5"),
  );
  answer.if_absent_declines_a_key_that_has_not_expired =
    claim.applied === true && held.applied === false && held.version === claim.version;
  answer.if_present_and_if_version_decline_an_expired_key_with_no_version =
    present.applied === false &&
    !("version" in present) &&
    versioned.applied === false &&
    !("version" in versioned);
  answer.if_absent_applies_on_an_expired_key = retaken.applied === true;
  answer.a_key_set_again_after_expiring_takes_the_next_version =
    retaken.version === claim.version + 1 &&
    (await later(HOUR + MINUTE, () => get(store, "job", run))).value?.owner === "b";
  answer.lease_trace = { claim, held, present, versioned, retaken };
}

{
  // **…and so does every condition while the expired key is still stored.** A
  // write's own sweep removes at most 100 of its partition's expired rows
  // before it reads its key, so above, where the partition holds the one
  // expired key, that sweep has deleted it before any condition looks — and
  // the conditions would pass with no notion of expiry at all. Here 400
  // fillers expire two minutes ahead of the key, so each write's sweep takes a
  // hundred of them and leaves the key a row: what the condition reads is an
  // expired row, and it has to read it as absent. Every count below says the
  // key was still stored when the condition read it.
  const store = kv("lease_backlog", "global");
  const run = ctx(execution("lease_backlog"));
  const FILLERS = 400;
  await later(0, async () => {
    for (let index = 0; index < FILLERS; index += 1) {
      await write(
        store,
        "set",
        { key: `f-${String(index).padStart(3, "0")}`, value: { n: index }, ttl: HOUR - 2 * MINUTE },
        run,
      );
    }
  });
  const claim = await later(0, () =>
    write(store, "set", { key: "job", value: { owner: "a" }, condition: "absent", ttl: HOUR }, run),
  );
  /** Whether `job` is still a row, expired or not, and how many rows the partition holds. */
  const stored = async () => [
    Number(
      (
        await storage(
          `SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND scope_key = ? AND ${provider === "mysql" ? "`key`" : '"key"'} = ?`,
          [store.name, "global", "job"],
          [store.name, "SELECT COUNT(*) AS n FROM entries WHERE scope_key = ? AND key = ?", ["global", "job"]],
        )
      )[0]?.n,
    ),
    await storedRows(store, "global"),
  ];
  const present = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "c" }, condition: "present" }, run),
  );
  const afterPresent = await stored();
  const read = await later(HOUR + MINUTE, () => get(store, "job", run));
  const versioned = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "c" }, ifVersion: claim.version }, run),
  );
  const afterVersioned = await stored();
  const retaken = await later(HOUR + MINUTE, () =>
    write(store, "set", { key: "job", value: { owner: "b" }, condition: "absent", ttl: HOUR }, run),
  );
  const afterRetaken = await stored();
  // 400 fillers and the key; each write's sweep takes a hundred fillers.
  answer.if_present_and_if_version_decline_an_expired_key_a_sweep_has_not_reached =
    claim.applied === true &&
    claim.version === FILLERS + 1 &&
    present.applied === false &&
    !("version" in present) &&
    same(afterPresent, [1, 301]) &&
    read.found === false &&
    versioned.applied === false &&
    !("version" in versioned) &&
    same(afterVersioned, [1, 201]);
  answer.if_absent_takes_an_expired_key_a_sweep_has_not_reached =
    retaken.applied === true && retaken.version === claim.version + 1 && same(afterRetaken, [1, 101]);
  answer.lease_backlog_trace = { claim, present, afterPresent, read, versioned, afterVersioned, retaken, afterRetaken };
}

{
  // A write refreshes its key's expiry to its own `ttl`; a write naming none on
  // a store with no default clears it; and a write that declined changes
  // nothing, the expiry included.
  const store = kv("refresh", "global", URL_ENV, RECORD);
  const run = ctx(execution("refresh"));
  const value = { title: "t", tags: [], count: 0 };
  await later(0, () => write(store, "set", { key: "r", value, ttl: HOUR }, run, "r/1"));
  await later(HOUR / 2, () =>
    write(store, "merge", { key: "r", fields: { note: "kept" }, ttl: HOUR }, run, "r/2"),
  );
  const refreshed = await later(HOUR + HOUR / 4, () => get(store, "r", run));
  const lapsed = await later(HOUR + HOUR / 2 + MINUTE, () => get(store, "r", run));

  await later(0, () => write(store, "set", { key: "c", value, ttl: HOUR }, run, "r/3"));
  await later(HOUR / 2, () => write(store, "incr", { key: "c", field: "count" }, run, "r/4"));
  const cleared = await later(10 * HOUR, () => get(store, "c", run));

  await later(0, () => write(store, "set", { key: "d", value, ttl: HOUR }, run, "r/5"));
  const declined = await later(HOUR / 2, () =>
    write(store, "set", { key: "d", value, condition: "absent", ttl: "never" }, run, "r/6"),
  );
  const unchanged = await later(HOUR + MINUTE, () => get(store, "d", run));

  answer.a_write_refreshes_expiry_to_its_own_ttl =
    refreshed.found === true && refreshed.value?.note === "kept" && lapsed.found === false;
  answer.a_write_with_no_ttl_and_no_default_clears_expiry =
    cleared.found === true && cleared.value?.count === 1;
  answer.a_declined_write_leaves_expiry_unchanged =
    declined.applied === false && unchanged.found === false;
  answer.refresh_trace = { refreshed, lapsed, cleared, declined, unchanged };
}

{
  // `default_ttl:` stands in for a write that names no `ttl`, and `ttl: never`
  // beats it — on a write that sets the key afresh and on one that clears an
  // expiry the default gave it — as does a write's own duration.
  const store = { ...kv("defaulted", "global"), defaultTtl: HOUR };
  const run = ctx(execution("defaulted"));
  await later(0, () => write(store, "set", { key: "plain", value: { n: 1 } }, run, "d/1"));
  await later(0, () => write(store, "set", { key: "never", value: { n: 2 }, ttl: "never" }, run, "d/2"));
  await later(0, () => write(store, "set", { key: "longer", value: { n: 3 }, ttl: 3 * HOUR }, run, "d/3"));
  await later(0, () => write(store, "set", { key: "cleared", value: { n: 4 } }, run, "d/4"));
  await later(HOUR / 2, () =>
    write(store, "set", { key: "cleared", value: { n: 5 }, ttl: "never" }, run, "d/5"),
  );
  const at = (key) => later(2 * HOUR, () => get(store, key, run));
  const [plain, never, longer, cleared] = [
    await at("plain"),
    await at("never"),
    await at("longer"),
    await at("cleared"),
  ];
  answer.a_write_with_no_ttl_takes_the_store_default = plain.found === false;
  answer.ttl_never_clears_a_store_default = never.found === true && cleared.value?.n === 5;
  answer.a_writes_own_ttl_beats_the_store_default = longer.found === true;
  answer.default_trace = { plain, never, longer, cleared };
}

{
  // The partial updates take a `ttl` like a `set`: an `append` and an `incr`
  // that create their key date it, and an `append` with `unique: true` whose
  // item is already there — which changes neither value nor version — still
  // refreshes the expiry, because it applied.
  const store = kv("partial_expiry", "global", URL_ENV, RECORD);
  const run = ctx(execution("partial_expiry"));
  await later(0, () =>
    write(store, "set", { key: "u", value: { title: "u", tags: ["a"], count: 0 }, ttl: HOUR }, run, "p/2"),
  );
  const same_ = await later(HOUR / 2, () =>
    write(store, "append", { key: "u", field: "tags", value: "a", unique: true, ttl: HOUR }, run, "p/3"),
  );
  const refreshed = await later(HOUR + HOUR / 4, () => get(store, "u", run));
  await later(0, () =>
    write(store, "set", { key: "a", value: { title: "a", tags: [], count: 0 } }, run, "p/4"),
  );
  await later(0, () => write(store, "append", { key: "a", field: "tags", value: "x", ttl: HOUR }, run, "p/5"));
  await later(0, () => write(store, "incr", { key: "a", field: "count", ttl: 3 * HOUR }, run, "p/6"));
  const appended = await later(2 * HOUR, () => get(store, "a", run));
  await later(0, () =>
    write(store, "merge", { key: "m", fields: { title: "m", tags: [], count: 1 }, ttl: HOUR }, run, "p/7"),
  );
  const merged = await later(HOUR + MINUTE, () => get(store, "m", run));
  answer.a_unique_append_that_changes_nothing_still_refreshes_expiry =
    same_.applied === true && refreshed.found === true && refreshed.version === same_.version;
  answer.merge_append_and_incr_each_take_a_ttl =
    appended.found === true && same(appended.value?.tags, ["x"]) && merged.found === false;
  answer.partial_expiry_trace = { same: same_, refreshed, appended, merged };
}

{
  // **An expired key is invisible at once and leaves storage lazily**: it is
  // still a row until a write to its partition sweeps it — and that sweep is
  // bounded, so a backlog of 150 loses 100 to one write and the rest to the
  // next.
  const store = kv("sweep", "global");
  const run = ctx(execution("sweep"));
  await later(0, async () => {
    // One key that never expires, beside 150 that do, so every count below is
    // the expired rows still stored plus the ones that are alive.
    await write(store, "set", { key: "kept", value: { n: -1 } }, run);
    for (let index = 0; index < 150; index += 1) {
      await write(store, "set", { key: `old-${String(index).padStart(3, "0")}`, value: { n: index }, ttl: HOUR }, run);
    }
  });
  const seeded = await storedRows(store, "global");
  const listed = await later(HOUR + MINUTE, () => list(store, run, undefined, 1000));
  const read = await later(HOUR + MINUTE, () => get(store, "old-007", run));
  const beforeSweep = await storedRows(store, "global");
  await later(HOUR + MINUTE, () => write(store, "set", { key: "new-1", value: { n: 0 } }, run));
  const afterOne = await storedRows(store, "global");
  await later(HOUR + MINUTE, () => write(store, "set", { key: "new-2", value: { n: 0 } }, run));
  const afterTwo = await storedRows(store, "global");
  answer.an_expired_key_stays_invisible_until_a_write_removes_it =
    seeded === 151 && beforeSweep === 151 && same(listed.keys, ["kept"]) && read.found === false;
  // 150 expired and one alive; one write removes 100 and adds itself.
  answer.the_write_sweep_removes_at_most_100_rows = afterOne === 151 - 100 + 1;
  // …and the next removes the other 50: `kept`, `new-1` and `new-2` are left.
  answer.an_expired_key_leaves_storage_at_the_next_writes_to_its_partition = afterTwo === 3;
  answer.sweep_trace = { seeded, beforeSweep, afterOne, afterTwo };
}

{
  // **…and that sweep is its own partition's, and no other's** (PRD resolved
  // q69 (c), Decision D153). A write sweeps under the sequence lock of the
  // partition it writes, and on a server that lock is what keeps its `DELETE`
  // and every other one off each other's rows: a sweep that reached into a
  // second partition would hold rows a write to that partition is about to
  // reach for, under a lock that write does not take — the deadlock the lock
  // exists to rule out. So two partitions each hold 50 expired keys, fewer than
  // one write's 100 together, and a write to the first must leave the second's
  // every one: a sweep with no partition in its `DELETE` takes all 100.
  const store = kv("sweep_partition", "session");
  const runs = {
    a: ctx(execution("sweep_partition_a"), "a"),
    b: ctx(execution("sweep_partition_b"), "b"),
  };
  await later(0, async () => {
    for (const session of ["a", "b"]) {
      await write(store, "set", { key: "kept", value: { n: -1 } }, runs[session]);
      for (let index = 0; index < 50; index += 1) {
        await write(store, "set", { key: `old-${String(index).padStart(2, "0")}`, value: { n: index }, ttl: HOUR }, runs[session]);
      }
    }
  });
  const partitions = async () => [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
  const seeded = await partitions();
  await later(HOUR + MINUTE, () => write(store, "set", { key: "new", value: { n: 0 } }, runs.a));
  const afterFirst = await partitions();
  await later(HOUR + MINUTE, () => write(store, "set", { key: "new", value: { n: 0 } }, runs.b));
  const afterSecond = await partitions();
  // 50 expired and `kept` in each; the write to `a` leaves `kept` and itself
  // there and all 51 in `b`, and the write to `b` then does the same there.
  answer.a_writes_sweep_takes_no_other_partitions_rows =
    same(seeded, [51, 51]) && same(afterFirst, [2, 51]) && same(afterSecond, [2, 2]);
  answer.sweep_partition_trace = { seeded, afterFirst, afterSecond };
}

{
  // …or when a process next opens the store, whichever comes first: a process
  // whose first op on the store is a **read** still removes every expired key
  // of it, every partition included. The second process is real — the open is
  // per process on SQLite, and per connection on a server — and it reads the
  // clock the parent hands it.
  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "store-expiry-open.mjs");
  const store = kv("reopen_expiry", "session", THIRD_URL_ENV);
  try {
    for (const session of ["a", "b"]) {
      const run = ctx(execution(`reopen_expiry_${session}`), session);
      await later(0, async () => {
        for (let index = 0; index < 3; index += 1) {
          await write(store, "set", { key: `k${index}`, value: { n: index }, ttl: HOUR }, run);
        }
        await write(store, "set", { key: "kept", value: { n: 9 } }, run);
      });
    }
    const before = [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
    const opened = await new Promise((resolve, reject) => {
      const child = spawn(
        process.execPath,
        [script, project, provider, data, JSON.stringify(store), "a"],
        {
          env: { ...process.env, [CLOCK]: String(HOUR + MINUTE) },
          stdio: ["ignore", "pipe", "pipe"],
        },
      );
      let out = "";
      let err = "";
      child.stdout.on("data", (chunk) => (out += chunk));
      child.stderr.on("data", (chunk) => (err += chunk));
      child.on("error", reject);
      child.on("close", (code) =>
        code === 0 ? resolve(JSON.parse(out)) : reject(new Error(`the opener exited ${code}: ${err}`)),
      );
    });
    const after = [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
    answer.an_expired_key_leaves_storage_when_a_process_next_opens_the_store =
      same(before, [4, 4]) && same(after, [1, 1]) && opened.found === false;
    answer.reopen_trace = { before, after, opened };
  } catch (error) {
    answer.reopen_error = String(error?.message ?? error);
  }
}

{
  // …**and when the deployment next starts, whether or not anything it does
  // reaches the store.** A process starting — a `run`, a `resume`, a `serve` —
  // starts the store-wide sweep of every `kv` store the composition binds
  // (`stores.sweepBoundStores`), so a store whose partitions nothing writes or
  // reads again still sheds what expired in it. The second process here makes
  // no op at all (`store-process-start.mjs`): what removes the backlog is its
  // start, not a first op.
  const store = kv("start_expiry", "session", FOURTH_URL_ENV);
  try {
    for (const session of ["a", "b"]) {
      const run = ctx(execution(`start_expiry_${session}`), session);
      await later(0, async () => {
        for (let index = 0; index < 3; index += 1) {
          await write(store, "set", { key: `k${index}`, value: { n: index }, ttl: HOUR }, run);
        }
        await write(store, "set", { key: "kept", value: { n: 9 } }, run);
      });
    }
    const before = [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
    // A start whose clock has not reached the expiry removes nothing…
    const early = await helper("store-process-start.mjs", [project, provider, data, JSON.stringify(store)], {
      ...process.env,
      [CLOCK]: String(HOUR - MINUTE),
    });
    const beforeExpiry = [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
    // …and one past it removes every expired key, both partitions included.
    const started = await helper("store-process-start.mjs", [project, provider, data, JSON.stringify(store)], {
      ...process.env,
      [CLOCK]: String(HOUR + MINUTE),
    });
    const after = [await storedRows(store, "session/a"), await storedRows(store, "session/b")];
    const kept = (await get(store, "kept", ctx(execution("start_expiry_read"), "a"))).value?.n;
    answer.an_expired_key_leaves_storage_when_a_process_starts_without_reaching_the_store =
      early.answer.started === true &&
      started.answer.started === true &&
      same(before, [4, 4]) &&
      same(beforeExpiry, [4, 4]) &&
      same(after, [1, 1]) &&
      kept === 9;
    answer.start_expiry_trace = { before, beforeExpiry, after, kept };
  } catch (error) {
    answer.start_expiry_error = String(error?.message ?? error);
  }
}

if (provider !== "sqlite") {
  // **A store-wide sweep and a write never deadlock**, and the sweep takes no
  // key a write refreshed. Both halves of the cleanup schedule delete expired
  // rows — a write its own partition's, a connection's first op the whole
  // store's — and on a server two `DELETE`s reaching one row through two
  // indexes lock it in two orders, a cycle the server breaks by failing one of
  // them. So the two run at once, in two processes (`store-sweep-race.mjs`),
  // over a partition holding a backlog of expired rows both reach for: every
  // write has to land, and every sweep has to finish.
  //
  // The backlog is laid down in bulk over this file's own connection rather
  // than written key by key, because what is under test is the sweeps that
  // remove it; the one write before it gives the partition its sequence row,
  // as the first write to any partition does.
  const store = kv("sweep_race", "global");
  //
  // The backlog expires **during** the race, a slice at a time, so the writer's
  // own sweep keeps finding fresh rows to take while the other process's
  // store-wide sweep — whose clock has them all expired — is taking the same
  // rows in bulk. That is the overlap a deadlock needs, and against a store
  // that lets the two `DELETE`s meet it fails the writer as a deadlock victim
  // on every run of this case measured; the sweeper starts a moment after the
  // writer so the writer is mid-stream when it does.
  const KEYS = 1_000;
  const BACKLOG = 20_000;
  /** How long after the race starts the backlog finishes expiring, to the writer. */
  const SPREAD = 3_000;
  /**
   * Rows of the same partition that only the store-wide sweep can take: they
   * expire half an hour from now, so they are never expired to the writer and
   * long expired to the sweeper. Without them every row the race removes is one
   * the writer's own bounded sweeps could have removed alone — a thousand
   * writes at a hundred rows each is five times the backlog — so a store whose
   * store-wide sweep never ran would pass all three cases below: nothing to
   * deadlock with, nothing to take a refreshed key, and a backlog gone anyway.
   */
  const SWEPT_ONLY = 5_000;
  try {
    await later(0, () => set(store, "seed", { n: 0 }, ctx(execution("sweep_race_seed"))));
    const serverNow = async () =>
      Number(
        (
          await storage(
            provider === "postgres"
              ? "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS now"
              : "SELECT CAST(FLOOR(UNIX_TIMESTAMP(NOW(3)) * 1000) AS SIGNED) AS now",
            [],
          )
        )[0]?.now,
      );
    const insert = async (entries) => {
      for (let at = 0; at < entries.length; at += 2_000) {
        const chunk = entries.slice(at, at + 2_000);
        await storage(
          `INSERT INTO store_entries (store, scope_key, ${provider === "postgres" ? '"key"' : "`key`"}, value, version, expires_at) VALUES ` +
            chunk.map(() => "(?, ?, ?, ?, ?, ?)").join(", "),
          chunk.flatMap(([key, expiresAt]) => [store.name, "global", key, '{"n":0}', 0, expiresAt]),
        );
      }
    };
    // The rows only the store-wide sweep can take — first, so laying them down
    // spends none of the two seconds the race below is timed from.
    const sweptOnlyExpiry = (await serverNow()) + HOUR / 2;
    await insert(
      Array.from({ length: SWEPT_ONLY }, (_, index) => [`y-${String(index).padStart(5, "0")}`, sweptOnlyExpiry]),
    );
    const now = await serverNow();
    const startAt = Date.now() + 2_000;
    // The backlog: expiring across the race's first three seconds, to the
    // writer, and long expired to the sweeper. The keys: live for an hour to
    // the writer, which refreshes each, and expired to the sweeper.
    await insert(
      Array.from({ length: BACKLOG }, (_, index) => [
        `z-${String(index).padStart(5, "0")}`,
        now + 2_000 + Math.floor((index * SPREAD) / BACKLOG),
      ]),
    );
    await insert(Array.from({ length: KEYS }, (_, index) => [`k-${String(index).padStart(5, "0")}`, now + HOUR]));
    const args = (role) => [
      project,
      provider,
      data,
      JSON.stringify(store),
      role,
      String(role === "sweep" ? startAt + 300 : startAt),
      String(KEYS),
    ];
    const [writer, sweeper] = await Promise.all([
      helper("store-sweep-race.mjs", args("write"), { ...process.env, [CLOCK]: "0" }),
      helper("store-sweep-race.mjs", args("sweep"), { ...process.env, [CLOCK]: String(HOUR + MINUTE) }),
    ]);
    const refreshed = await later(HOUR + MINUTE, async () => {
      let found = 0;
      for (let index = 0; index < KEYS; index += 1) {
        const held = await get(store, `k-${String(index).padStart(5, "0")}`, ctx(execution("sweep_race_read")));
        if (held.found === true && held.value?.n === -index) found += 1;
      }
      return found;
    });
    // Every backlog row expires before any key does, so this counts the
    // backlog still stored and nothing else.
    const backlog = Number(
      (
        await storage("SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND expires_at < ?", [
          store.name,
          now + HOUR,
        ])
      )[0]?.n,
    );
    // …and the rows no write could have taken, which are gone only if the
    // store-wide sweep ran — in the very partition the writer was writing.
    const sweptOnly = Number(
      (
        await storage(
          `SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND SUBSTR(${provider === "postgres" ? '"key"' : "`key`"}, 1, 2) = ?`,
          [store.name, "y-"],
        )
      )[0]?.n,
    );
    // The two processes' windows overlap: the sweeper's first op — which starts
    // its sweeps — came before the writer's last write, and its sweeps ended
    // after the writer's first. Two processes that ran one after the other
    // would be no race at all.
    const raced =
      sweeper.answer.from < writer.answer.to && sweeper.answer.to > writer.answer.from;
    const swept = sweptOnly === 0 && raced;
    const warned = (sweeper.stderr + writer.stderr).includes("could not finish removing");
    answer.a_store_wide_sweep_and_a_write_never_deadlock =
      swept &&
      writer.answer.errors.length === 0 &&
      sweeper.answer.errors.length === 0 &&
      writer.answer.applied === KEYS &&
      !warned;
    answer.a_store_wide_sweep_takes_no_key_a_write_refreshed = swept && refreshed === KEYS;
    answer.a_store_wide_sweep_removes_the_whole_backlog = swept && backlog === 0;
    answer.sweep_race_trace = {
      writer: {
        applied: writer.answer.applied,
        errors: writer.answer.errors.slice(0, 3),
        from: writer.answer.from,
        to: writer.answer.to,
      },
      sweeper: { errors: sweeper.answer.errors.slice(0, 3), from: sweeper.answer.from, to: sweeper.answer.to },
      warned,
      refreshed,
      backlog,
      sweptOnly,
      raced,
    };
  } catch (error) {
    answer.sweep_race_error = String(error?.message ?? error);
  } finally {
    // This run's rows, and only them: the server is shared.
    for (const table of ["store_entries", "store_applied", "store_sequences"]) {
      await storage(`DELETE FROM ${table} WHERE store = ?`, [store.name]).catch(() => undefined);
    }
  }
}

{
  // **The start sweep runs behind a process's first op on a store, never in
  // front of it, and has finished by the time the process lets go** (PRD
  // resolved q69 (c)). A backlog of expired keys is laid down in bulk; a second
  // process's first op on the store is a read, and the moment it answers that
  // process counts how much of the backlog is still stored, then lets go of the
  // store and counts again (`store-first-op.mjs`). A sweep in front of the op
  // would have removed all of it before the read answered — every op of a
  // process starting against a store with a large backlog waiting on it.
  const store = kv("first_op", "global");
  const BACKLOG = 20_000;
  try {
    await set(store, "kept", { n: 1 }, ctx(execution("first_op_seed")));
    const clock =
      provider === "sqlite"
        ? Date.now()
        : Number(
            (
              await storage(
                provider === "postgres"
                  ? "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS now"
                  : "SELECT CAST(FLOOR(UNIX_TIMESTAMP(NOW(3)) * 1000) AS SIGNED) AS now",
                [],
              )
            )[0]?.now,
          );
    // Live to this process for an hour; expired to the second, whose clock is
    // an hour and a minute ahead.
    const expiresAt = clock + HOUR;
    for (let at = 0; at < BACKLOG; at += 2_000) {
      const keys = Array.from({ length: Math.min(2_000, BACKLOG - at) }, (_, index) => `z-${at + index}`);
      if (provider === "sqlite") {
        await storage(undefined, undefined, [
          store.name,
          "INSERT INTO entries (scope_key, key, value, version, expires_at) VALUES " +
            keys.map(() => "(?, ?, ?, ?, ?)").join(", "),
          keys.flatMap((key) => ["global", key, '{"n":0}', 0, expiresAt]),
        ]);
      } else {
        await storage(
          `INSERT INTO store_entries (store, scope_key, ${provider === "postgres" ? '"key"' : "`key`"}, value, version, expires_at) VALUES ` +
            keys.map(() => "(?, ?, ?, ?, ?, ?)").join(", "),
          keys.flatMap((key) => [store.name, "global", key, '{"n":0}', 0, expiresAt]),
        );
      }
    }
    const opened = await helper(
      "store-first-op.mjs",
      [project, provider, data, JSON.stringify(store), String(expiresAt + 1)],
      { ...process.env, [CLOCK]: String(HOUR + MINUTE) },
    );
    const { atAnswer, afterRelease } = opened.answer;
    answer.a_first_op_answers_before_the_start_sweep_has_finished =
      opened.answer.answer?.found === true && atAnswer > 0;
    answer.the_start_sweep_has_removed_the_backlog_when_the_process_lets_go = afterRelease === 0;
    answer.first_op_trace = { atAnswer, afterRelease };
  } catch (error) {
    answer.first_op_error = String(error?.message ?? error);
  } finally {
    if (provider !== "sqlite") {
      for (const table of ["store_entries", "store_applied", "store_sequences"]) {
        await storage(`DELETE FROM ${table} WHERE store = ?`, [store.name]).catch(() => undefined);
      }
    }
  }
}

{
  // **The start sweep takes at most 1000 rows a batch, and on a server each
  // batch takes only the rows of its own partition** (PRD resolved q69 (c),
  // Decision D153). The bound is what keeps an op from waiting for more than
  // the one batch in progress however large the backlog is — on SQLite a batch
  // is a synchronous statement the event loop cannot interrupt, and on a server
  // one holds a partition's sequence lock and this connection's queue — and the
  // partition is what keeps a batch from deadlocking a write to another one. A
  // case that counted the backlog *while* the sweep ran would pass one
  // unbounded `DELETE`, so a second process counts it **between** the sweep's
  // batches (`store-sweep-batches.mjs` says how, per arm), after each of
  // several probes.
  //
  // Two partitions hold the backlog: `a` the ten oldest rows, `b` several
  // batches' worth behind them. Every batch may take at most 1000 rows of the
  // total; and on a server the first batch is `a`'s — the partition holding the
  // store's oldest expired row — and must take all ten of it and nothing of
  // `b`, where a batch with no partition in its `DELETE` takes 1000 rows of
  // the store, at least 990 of them `b`'s. Every later batch changes one
  // partition's count. The local sweep is the file's, so on SQLite only the
  // bound is asked of it.
  const store = kv("sweep_batches", "session");
  const FIRST = 10;
  const SECOND = 3_500;
  /** One more than the batches the backlog takes, so the last count is past the sweep's end. */
  const PROBES = 6;
  const partitions = ["session/a", "session/b"];
  /** A count, with a partition the backlog has left counted as zero rather than absent. */
  const backlog = (counted) =>
    Object.fromEntries(partitions.map((partition) => [partition, Number(counted?.[partition] ?? 0)]));
  try {
    // Each partition's sequence row — a store-wide batch takes its partition's
    // lock first, so a partition without one is one it cannot sweep — and a
    // key that never expires, beside the partition the probes write to.
    for (const session of ["a", "b", "probe"]) {
      await later(0, () => set(store, "kept", { n: 0 }, ctx(execution(`sweep_batches_${session}`), session)));
    }
    const clock =
      provider === "sqlite"
        ? Date.now()
        : Number(
            (
              await storage(
                provider === "postgres"
                  ? "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS now"
                  : "SELECT CAST(FLOOR(UNIX_TIMESTAMP(NOW(3)) * 1000) AS SIGNED) AS now",
                [],
              )
            )[0]?.now,
          );
    // Live to every probe for half an hour, and expired to the sweep, whose
    // clock is an hour and a minute ahead: `a`'s rows first, then `b`'s, one
    // millisecond apart.
    const laid = [
      ...Array.from({ length: FIRST }, (_, index) => ["session/a", `z-a-${index}`, clock + HOUR / 2]),
      ...Array.from({ length: SECOND }, (_, index) => [
        "session/b",
        `z-b-${String(index).padStart(4, "0")}`,
        clock + HOUR / 2 + MINUTE + index,
      ]),
    ];
    for (let at = 0; at < laid.length; at += 2_000) {
      const chunk = laid.slice(at, at + 2_000);
      if (provider === "sqlite") {
        await storage(undefined, undefined, [
          store.name,
          "INSERT INTO entries (scope_key, key, value, version, expires_at) VALUES " +
            chunk.map(() => "(?, ?, ?, ?, ?)").join(", "),
          chunk.flatMap(([partition, key, expiresAt]) => [partition, key, '{"n":0}', 0, expiresAt]),
        ]);
      } else {
        await storage(
          `INSERT INTO store_entries (store, scope_key, ${provider === "postgres" ? '"key"' : "`key`"}, value, version, expires_at) VALUES ` +
            chunk.map(() => "(?, ?, ?, ?, ?, ?)").join(", "),
          chunk.flatMap(([partition, key, expiresAt]) => [store.name, partition, key, '{"n":0}', 0, expiresAt]),
        );
      }
    }
    const before = backlog(
      Object.fromEntries(
        (
          await storage(
            "SELECT scope_key, COUNT(*) AS n FROM store_entries WHERE store = ? AND expires_at IS NOT NULL GROUP BY scope_key",
            [store.name],
            [store.name, "SELECT scope_key, COUNT(*) AS n FROM entries WHERE expires_at IS NOT NULL GROUP BY scope_key"],
          )
        ).map((row) => [String(row.scope_key), Number(row.n)]),
      ),
    );
    const swept = await helper(
      "store-sweep-batches.mjs",
      [project, provider, data, JSON.stringify(store), String(PROBES)],
      { ...process.env, [CLOCK]: String(HOUR + MINUTE) },
    );
    const failed = swept.answer.observed.find((counted) => counted?.error !== undefined);
    const observed = swept.answer.observed.map(backlog);
    const after = backlog(swept.answer.after);
    const total = (counted) => partitions.reduce((sum, partition) => sum + counted[partition], 0);
    const totals = [before, ...observed].map(total);
    const taken = totals.slice(1).map((left, index) => totals[index] - left);
    // Every probe found the sweep at most one batch further on, and at least one
    // found it under way — neither untouched nor finished — so the counts are
    // the sweep's own steps rather than its two ends.
    answer.the_start_sweep_takes_at_most_1000_rows_a_batch =
      failed === undefined &&
      swept.answer.first?.found === true &&
      same(before, { "session/a": FIRST, "session/b": SECOND }) &&
      observed.length === PROBES &&
      taken.every((n) => n >= 0 && n <= 1_000) &&
      totals.some((n) => n > 0 && n < totals[0]) &&
      total(after) === 0;
    if (provider !== "sqlite") {
      const steps = [before, ...observed];
      answer.a_store_wide_sweep_batch_takes_only_its_own_partitions_rows =
        failed === undefined &&
        same(observed[0], { "session/a": 0, "session/b": SECOND }) &&
        steps
          .slice(1)
          .every(
            (counted, index) =>
              partitions.filter((partition) => counted[partition] !== steps[index][partition]).length <= 1,
          );
    }
    answer.sweep_batches_trace = { before, observed, after, failed, totals };
  } catch (error) {
    answer.sweep_batches_error = String(error?.message ?? error);
  } finally {
    if (provider !== "sqlite") {
      for (const table of ["store_entries", "store_applied", "store_sequences"]) {
        await storage(`DELETE FROM ${table} WHERE store = ?`, [store.name]).catch(() => undefined);
      }
    }
  }
}

{
  // A clock offset nobody can read fails the op by name rather than being
  // read as no offset — a test that believed it moved the clock and did not
  // would pass for the wrong reason.
  const store = kv("bad_clock", "global");
  const run = ctx(execution("bad_clock"));
  const refused = await later("an hour", () => refusal(get(store, "k", run)));
  answer.a_malformed_clock_offset_is_refused_by_name = (refused ?? "").includes(CLOCK);
}

/** Ten days: how far off this process's own clock is put by [`skewed`]. */
const TEN_DAYS = 10 * 24 * HOUR;

/**
 * Run `work` with **this process's** clock — `Date.now` itself, not the store
 * clock [`later`] moves — `shift` milliseconds off, and put it back once `work`
 * has finished, every await inside it included.
 */
const skewed = async (shift, work) => {
  const realNow = Date.now;
  Date.now = () => realNow() + shift;
  try {
    return await work();
  } finally {
    Date.now = realNow;
  }
};

/** The server's own clock in epoch milliseconds, spelled here rather than borrowed from the arm. */
const SERVER_NOW =
  provider === "postgres"
    ? "SELECT FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS server_now"
    : "SELECT CAST(FLOOR(UNIX_TIMESTAMP(NOW(3)) * 1000) AS SIGNED) AS server_now";

if (provider !== "sqlite") {
  // **A dialled store's expiry is the server's clock's**, so two writers whose
  // own clocks disagree still agree about which keys are alive. This process's
  // clock is moved ten days ahead for the write; what the row holds is the
  // server's instant plus the `ttl`, and the key is alive to a reader on the
  // server's clock.
  const store = kv("server_clock", "global");
  const run = ctx(execution("server_clock"));
  const written = await skewed(TEN_DAYS, () =>
    write(store, "set", { key: "k", value: { n: 1 }, ttl: HOUR }, run),
  );
  // The server's own clock, spelled here rather than borrowed from the arm, so
  // the case does not take the arm's word for what that clock is. Epoch time
  // is zone-free on Postgres; MySQL's `UNIX_TIMESTAMP()` with no argument is
  // too, at a second's resolution — well inside the margin.
  const [held] = await storage(
    provider === "postgres"
      ? 'SELECT expires_at, FLOOR(EXTRACT(EPOCH FROM clock_timestamp()) * 1000)::bigint AS server_now FROM store_entries WHERE store = ? AND "key" = ?'
      : "SELECT expires_at, UNIX_TIMESTAMP() * 1000 AS server_now FROM store_entries WHERE store = ? AND `key` = ?",
    [store.name, "k"],
  );
  const ahead = Number(held?.expires_at) - Number(held?.server_now);
  answer.expiry_is_computed_by_the_servers_clock =
    written?.applied === true && ahead > HOUR - MINUTE && ahead < HOUR + MINUTE;

  // **…and so is every read's.** A `get`, a `list`, a `count` and a `get_many`
  // judge that row by the server's clock too, with this process's ten days
  // ahead and ten days behind:
  // the key is found while the server's clock holds it alive, and is absent
  // once the store clock is moved past its expiry. A read dated by this
  // process instead would miss it while it is alive (ahead) and still find it
  // once it has expired (behind) — and would disagree with the `if: absent` a
  // writer on the same server is judged by.
  const reads = {};
  for (const [label, shift] of [
    ["ahead", TEN_DAYS],
    ["behind", -TEN_DAYS],
  ]) {
    for (const [moment, offset] of [
      ["alive", 0],
      ["expired", HOUR + MINUTE],
    ]) {
      reads[`${label}_${moment}`] = await skewed(shift, () =>
        later(offset, async () => ({
          got: await get(store, "k", run),
          listed: (await list(store, run)).keys,
          // PRD resolved q70's reads carry the same server clock inside their
          // own statement (`liveRows`), so they agree with the `get` beside them.
          counted: (await stores.runStoreOp(store, "count", {}, run, node())).count,
          many: (await stores.runStoreOp(store, "get_many", { keys: ["k"] }, run, node())).entries,
        })),
      );
    }
  }
  const found = (read) =>
    read.got.found === true &&
    read.got.value?.n === 1 &&
    same(read.listed, ["k"]) &&
    read.counted === 1 &&
    read.many?.[0]?.found === true &&
    read.many?.[0]?.value?.n === 1;
  const missed = (read) =>
    same(read.got, { found: false }) &&
    same(read.listed, []) &&
    read.counted === 0 &&
    same(read.many, [{ key: "k", found: false }]);
  answer.a_dialled_read_finds_a_key_the_servers_clock_holds_alive =
    found(reads.ahead_alive) && found(reads.behind_alive);
  answer.a_dialled_read_misses_a_key_the_servers_clock_holds_expired =
    missed(reads.ahead_expired) && missed(reads.behind_expired);
  answer.server_clock_trace = { ahead, reads };
}

if (provider !== "sqlite") {
  // **…and so is a store-wide sweep's.** What the sweep a connection's first op
  // on a store starts removes is what had expired by the server's clock at
  // that op, whatever this process's clock says — so a writer's live key is
  // never taken by a process whose clock runs fast, and an expired one is not
  // left by a process whose clock runs slow. Two stores, each laid down
  // straight into storage (its partition's sequence row with it, which a sweep
  // batch locks), so this process's first op on each is a `get` that starts
  // its sweep; each is then swept to the end with this process's clock ten
  // days off. Ahead: a key alive on the server's clock stays, and one expired
  // on both clocks goes, which is the proof that the sweep ran. Behind, with
  // the store clock moved past a key's expiry: that key goes, and one alive on
  // both clocks stays.
  const fast = kv("server_clock_sweep_fast", "global");
  const slow = kv("server_clock_sweep_slow", "global");
  const key = provider === "postgres" ? '"key"' : "`key`";
  try {
    const serverNow = Number((await storage(SERVER_NOW, []))[0]?.server_now);
    const lay = async (store, rows) => {
      await storage("INSERT INTO store_sequences (store, scope_key, last) VALUES (?, ?, ?)", [
        store.name,
        "global",
        rows.length,
      ]);
      await storage(
        `INSERT INTO store_entries (store, scope_key, ${key}, value, version, expires_at) VALUES ` +
          rows.map(() => "(?, ?, ?, ?, ?, ?)").join(", "),
        rows.flatMap(([name, expiresAt], index) => [store.name, "global", name, '{"n":1}', index + 1, expiresAt]),
      );
    };
    await lay(fast, [
      ["alive", serverNow + HOUR],
      ["expired", serverNow - HOUR],
    ]);
    await lay(slow, [
      ["due", serverNow + HOUR],
      ["far", serverNow + 2 * TEN_DAYS],
    ]);
    const run = ctx(execution("server_clock_sweep"));
    // `drainStores` is the wait a command makes for its sweeps on its way out,
    // made inside the skew so the instant the sweep reads is read under it.
    await skewed(TEN_DAYS, async () => {
      await get(fast, "probe", run);
      await stores.drainStores();
    });
    await skewed(-TEN_DAYS, () =>
      later(HOUR + MINUTE, async () => {
        await get(slow, "probe", run);
        await stores.drainStores();
      }),
    );
    const kept = async (store) =>
      (await storage(`SELECT ${key} AS k FROM store_entries WHERE store = ? ORDER BY ${key}`, [store.name])).map(
        (row) => String(row.k),
      );
    const fastKept = await kept(fast);
    const slowKept = await kept(slow);
    answer.a_store_wide_sweep_keeps_a_key_the_servers_clock_holds_alive = same(fastKept, ["alive"]);
    answer.a_store_wide_sweep_takes_a_key_the_servers_clock_holds_expired = same(slowKept, ["far"]);
    answer.server_clock_sweep_trace = { fastKept, slowKept };
  } catch (error) {
    answer.server_clock_sweep_error = String(error?.message ?? error);
  } finally {
    // This run's rows, and only them: the server is shared.
    for (const store of [fast, slow]) {
      for (const table of ["store_entries", "store_applied", "store_sequences"]) {
        await storage(`DELETE FROM ${table} WHERE store = ?`, [store.name]).catch(() => undefined);
      }
    }
  }
}

// --- A batch: many keys, one transaction, one ledger row (PRD resolved q71) ----
//
// A `batch` is one `kv` write's shape with N entries inside its transaction:
// the partition's sequence locked once, the ledger asked once, each entry a
// single `set` or `delete` decided against what its key holds, all of them
// landing or none. Every case below is a promise the ruling makes of **every**
// arm, so a server whose savepoint, row lock or rollback answered differently
// from SQLite's is this list failing on that arm.

/** A batch, as a store-op node issues one. */
const batch = (store, ops, run, key_ = undefined) =>
  stores.runStoreOp(store, "batch", { ops }, run, node(key_));

/**
 * The last version a partition's sequence handed out, read out of storage
 * itself — which is what "no version consumed" is a statement about.
 */
async function sequenceLast(store, scopeKey = "global") {
  const held = await storage(
    "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ?",
    [store.name, scopeKey],
    [store.name, "SELECT last FROM sequences WHERE scope_key = ?", [scopeKey]],
  );
  return held[0] === undefined ? undefined : Number(held[0].last);
}

/**
 * The batch cases' `value_schema`, as the emitted Zod — which a `set` entry's
 * value is not parsed against at run time, as a single `set`'s is not.
 */
const BATCHED = z.object({ n: z.number().int().min(0) }).strict();

{
  // **A set, a set and a delete land together**, the two sets taking the
  // partition's next two versions in entry order, and the record the trace
  // files is one write with no single key and the whole answer.
  const store = kv("batch_lands", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_lands"));
  const seeded = await set(store, "old", { n: 0 }, run, `${RUN}/batch_lands/seed`);
  run.storeRecords.length = 0;
  const key = `${execution("batch_lands")}/move/0`;
  const moved = await batch(
    store,
    [
      { op: "set", key: "a", value: { n: 1 } },
      { op: "set", key: "b", value: { n: 2 } },
      { op: "delete", key: "old" },
    ],
    run,
    key,
  );
  const [a, b, old] = await Promise.all(["a", "b", "old"].map((name) => get(store, name, run)));
  answer.a_batch_of_set_set_delete_lands_all_three_with_consecutive_versions =
    moved.applied === true &&
    moved.deduped === false &&
    !("failed" in moved) &&
    same(moved.results, [
      { key: "a", version: seeded.version + 1 },
      { key: "b", version: seeded.version + 2 },
      { key: "old" },
    ]) &&
    a.value?.n === 1 &&
    a.version === seeded.version + 1 &&
    b.value?.n === 2 &&
    b.version === seeded.version + 2 &&
    old.found === false &&
    (await sequenceLast(store)) === seeded.version + 2;
  answer.batch_lands = moved;
  const [record] = run.storeRecords.filter((entry) => entry.op === "batch");
  answer.a_batch_record_is_one_write_with_no_key_and_its_answer =
    record !== undefined &&
    record.effect === "write" &&
    record.via === "node" &&
    !("key" in record) &&
    !("version" in record) &&
    record.applied === true &&
    record.idempotencyKey === key &&
    record.deduped === false &&
    same(record.answer?.results, moved.results);
  answer.batch_record = record ?? null;

  // **A retried batch is answered whole out of the ledger**: the same results,
  // `deduped: true`, and nothing written again — the versions the keys hold
  // are still the first attempt's, and the sequence has not moved.
  const retried = await batch(
    store,
    [
      { op: "set", key: "a", value: { n: 1 } },
      { op: "set", key: "b", value: { n: 2 } },
      { op: "delete", key: "old" },
    ],
    run,
    key,
  );
  answer.a_retried_batch_dedupes_whole =
    retried.applied === true &&
    retried.deduped === true &&
    same(retried.results, moved.results) &&
    (await get(store, "a", run)).version === seeded.version + 1 &&
    (await sequenceLast(store)) === seeded.version + 2;
}

{
  // **A condition that does not hold at index 2 applies nothing** — not the
  // two entries before it, which had already been written inside the
  // transaction — and answers `applied: false` naming the index. No version is
  // consumed. The decline is an answer, so it is the ledger row, and a retry
  // of it is handed the decline again even once the condition would hold.
  const store = kv("batch_declines", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_declines"));
  const held = await set(store, "held", { n: 9 }, run, `${RUN}/batch_declines/seed`);
  const key = `${execution("batch_declines")}/claim/0`;
  const ops = [
    { op: "set", key: "c", value: { n: 3 } },
    { op: "set", key: "d", value: { n: 4 } },
    { op: "set", key: "held", value: { n: 5 }, condition: "absent" },
  ];
  const declined = await batch(store, ops, run, key);
  const [c, d, still] = await Promise.all(["c", "d", "held"].map((name) => get(store, name, run)));
  answer.a_failed_condition_at_index_2_leaves_indexes_0_and_1_unapplied =
    declined.applied === false &&
    declined.deduped === false &&
    declined.failed === 2 &&
    same(declined.results, []) &&
    c.found === false &&
    d.found === false &&
    still.value?.n === 9 &&
    still.version === held.version &&
    (await sequenceLast(store)) === held.version;
  answer.batch_declined = declined;
  // The key the condition was about goes; the condition would hold now.
  await write(store, "delete", { key: "held" }, run, `${RUN}/batch_declines/clear`);
  const again = await batch(store, ops, run, key);
  answer.a_retried_declined_batch_answers_its_decline_deduped =
    again.applied === false &&
    again.deduped === true &&
    again.failed === 2 &&
    same(again.results, []) &&
    (await get(store, "c", run)).found === false &&
    (await get(store, "held", run)).found === false;
  // …while a batch under a key of its own, against the same state, applies.
  const fresh = await batch(store, ops, run, `${key}/fresh`);
  answer.the_same_entries_under_a_fresh_key_apply_once_the_condition_holds =
    fresh.applied === true && fresh.results.length === 3;
}

{
  // **A `set` entry's schema check is its single op's** (PRD resolved q71,
  // which names q68's): CEL typing at `validate` for a store-op node, and no
  // Zod at run time. So a value the store's emitted Zod would refuse — `n: -1`
  // under a `min(0)` no CEL type decides — is written by a single `set` and by
  // that same `set` as a batch entry alike, rather than landing one way and
  // failing the node the other; the entry before it and the `delete` after it
  // land beside it, each `set` at the partition's next version.
  const store = kv("batch_typed_once", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_typed_once"));
  const single = await set(store, "single", { n: -1 }, run, `${RUN}/batch_typed_once/single`);
  const kept = await set(store, "kept", { n: 7 }, run, `${RUN}/batch_typed_once/seed`);
  const moved = await batch(
    store,
    [
      { op: "set", key: "e", value: { n: 1 } },
      { op: "set", key: "f", value: { n: -1 } },
      { op: "delete", key: "kept" },
    ],
    run,
    `${execution("batch_typed_once")}/move/0`,
  );
  const [alone, e, f, gone] = await Promise.all(
    ["single", "e", "f", "kept"].map((name) => get(store, name, run)),
  );
  answer.batch_typed_once = moved;
  answer.a_batch_set_entry_writes_the_value_a_single_set_writes =
    BATCHED.safeParse({ n: -1 }).success === false &&
    single.applied === true &&
    alone.value?.n === -1 &&
    moved.applied === true &&
    same(moved.results, [
      { key: "e", version: kept.version + 1 },
      { key: "f", version: kept.version + 2 },
      { key: "kept" },
    ]) &&
    e.value?.n === 1 &&
    f.value?.n === -1 &&
    f.version === kept.version + 2 &&
    gone.found === false &&
    (await sequenceLast(store)) === kept.version + 2;
}

{
  // **A duplicate key inside one batch applies in order**: the second entry
  // naming a key reads what the first left inside the transaction, so an
  // `if: present` behind a `set` of an absent key holds, and a `set` then a
  // `delete` of one key leaves it absent.
  const store = kv("batch_in_order", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_in_order"));
  const ordered = await batch(
    store,
    [
      { op: "set", key: "k", value: { n: 1 } },
      { op: "set", key: "k", value: { n: 2 }, condition: "present" },
      { op: "set", key: "gone", value: { n: 3 } },
      { op: "delete", key: "gone" },
    ],
    run,
    `${execution("batch_in_order")}/move/0`,
  );
  const k = await get(store, "k", run);
  const first = ordered.results?.[0]?.version;
  answer.a_duplicate_key_within_a_batch_applies_in_order =
    ordered.applied === true &&
    same(ordered.results, [
      { key: "k", version: first },
      { key: "k", version: first + 1 },
      { key: "gone", version: first + 2 },
      { key: "gone" },
    ]) &&
    k.value?.n === 2 &&
    k.version === first + 1 &&
    (await get(store, "gone", run)).found === false;
  // …and a version condition on an entry reads the version an earlier entry
  // of the same batch took.
  const cas = await batch(
    store,
    [
      { op: "set", key: "k", value: { n: 4 } },
      { op: "delete", key: "k", ifVersion: k.version },
    ],
    run,
    `${execution("batch_in_order")}/move/1`,
  );
  answer.an_entry_condition_reads_what_the_entries_before_it_left =
    cas.applied === false && cas.failed === 1 && (await get(store, "k", run)).value?.n === 2;
}

{
  // **Each `set` entry's `ttl` and the store's `default_ttl:` apply exactly as
  // on a single write**: an entry's own `ttl` first, `never` beating the
  // default, and the default where an entry names none — each entry its own
  // expiry, from the one instant the batch read its clock.
  const store = { ...kv("batch_expiry", "global", URL_ENV, BATCHED), defaultTtl: 2 * HOUR };
  const run = ctx(execution("batch_expiry"));
  await later(0, () =>
    batch(
      store,
      [
        { op: "set", key: "short", value: { n: 1 }, ttl: HOUR },
        { op: "set", key: "defaulted", value: { n: 2 } },
        { op: "set", key: "kept", value: { n: 3 }, ttl: "never" },
      ],
      run,
      `${execution("batch_expiry")}/move/0`,
    ),
  );
  const at = (offset) =>
    later(offset, async () =>
      (await Promise.all(["short", "defaulted", "kept"].map((name) => get(store, name, run)))).map(
        (row) => row.found,
      ),
    );
  const [early, between, past] = [await at(HOUR - MINUTE), await at(HOUR + MINUTE), await at(2 * HOUR + MINUTE)];
  answer.each_batch_entrys_ttl_and_the_store_default_apply =
    same(early, [true, true, true]) && same(between, [false, true, true]) && same(past, [false, false, true]);
  answer.batch_expiry = { early, between, past };
}

{
  // **A batch sweeps its partition once, as one write does** (PRD resolved
  // q69 (c), D156): an expired key leaves storage at the next write to its
  // partition, and a batch is one write — so a batch of three entries into a
  // partition holding 150 expired rows takes 100 of them, not none and not a
  // hundred per entry, and the next batch takes the other 50. A key that never
  // expires beside them is every count's one live row until the first batch
  // deletes it.
  const store = kv("batch_sweep", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_sweep"));
  await later(0, async () => {
    await write(store, "set", { key: "kept", value: { n: 0 } }, run);
    // Laid down by batches, at most 100 entries each — the bound — and swept
    // of nothing, since nothing has expired at this instant.
    for (let start = 0; start < 150; start += 100) {
      await batch(
        store,
        Array.from({ length: Math.min(100, 150 - start) }, (_, offset) => ({
          op: "set",
          key: `old-${String(start + offset).padStart(3, "0")}`,
          value: { n: start + offset },
          ttl: HOUR,
        })),
        run,
      );
    }
  });
  const seeded = await storedRows(store, "global");
  const first = await later(HOUR + MINUTE, () =>
    batch(
      store,
      [
        { op: "set", key: "new-1", value: { n: 1 } },
        { op: "set", key: "new-2", value: { n: 2 } },
        { op: "delete", key: "kept" },
      ],
      run,
    ),
  );
  const afterFirst = await storedRows(store, "global");
  const second = await later(HOUR + MINUTE, () => batch(store, [{ op: "set", key: "new-3", value: { n: 3 } }], run));
  const afterSecond = await storedRows(store, "global");
  // 150 expired and `kept`; the first batch removes 100 of them, adds two keys
  // and deletes `kept`; the second removes the other 50 and adds one.
  answer.a_batch_sweeps_at_most_100_expired_rows_of_its_partition_once =
    seeded === 151 && first.applied === true && afterFirst === 151 - 100 + 2 - 1;
  answer.an_expired_key_leaves_storage_at_the_next_batch_to_its_partition =
    second.applied === true && afterSecond === 3;
  answer.batch_sweep_trace = { seeded, afterFirst, afterSecond };
}

{
  // **…and every entry's condition reads an expired key as absent** (PRD
  // resolved q69 (b)), behind a backlog the batch's own sweep has not
  // reached: 500 fillers expire two minutes ahead of `job`, so each batch's
  // sweep takes a hundred of them and leaves `job` a row — what each entry's
  // condition reads is an expired row, and every count below says so. A
  // `set` with `if: present` and one with `if_version:` decline it, as does a
  // `delete` with `if_version:`, each naming its index and consuming no
  // version; a `set` with `if: absent` takes it, at the partition's next
  // version — expiry consumed none.
  const store = kv("batch_lease", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_lease"));
  const FILLERS = 500;
  await later(0, async () => {
    for (let start = 0; start < FILLERS; start += 100) {
      await batch(
        store,
        Array.from({ length: 100 }, (_, offset) => ({
          op: "set",
          key: `f-${String(start + offset).padStart(3, "0")}`,
          value: { n: start + offset },
          ttl: HOUR - 2 * MINUTE,
        })),
        run,
      );
    }
  });
  const claim = await later(0, () =>
    batch(store, [{ op: "set", key: "job", value: { n: 1 }, condition: "absent", ttl: HOUR }], run),
  );
  const claimed = claim.results?.[0]?.version;
  /** Whether `job` is still a row, expired or not, and how many rows the partition holds. */
  const stored = async () => [
    Number(
      (
        await storage(
          `SELECT COUNT(*) AS n FROM store_entries WHERE store = ? AND scope_key = ? AND ${provider === "mysql" ? "`key`" : '"key"'} = ?`,
          [store.name, "global", "job"],
          [store.name, "SELECT COUNT(*) AS n FROM entries WHERE scope_key = ? AND key = ?", ["global", "job"]],
        )
      )[0]?.n,
    ),
    await storedRows(store, "global"),
  ];
  const expired = (ops) => later(HOUR + MINUTE, () => batch(store, ops, run));
  const present = await expired([
    { op: "set", key: "other", value: { n: 2 } },
    { op: "set", key: "job", value: { n: 2 }, condition: "present" },
  ]);
  const afterPresent = await stored();
  const versioned = await expired([{ op: "set", key: "job", value: { n: 3 }, ifVersion: claimed }]);
  const afterVersioned = await stored();
  const deleted = await expired([
    { op: "set", key: "other", value: { n: 4 } },
    { op: "delete", key: "job", ifVersion: claimed },
  ]);
  const afterDeleted = await stored();
  const declinedLast = await sequenceLast(store);
  const retaken = await expired([
    { op: "set", key: "job", value: { n: 5 }, condition: "absent", ttl: HOUR },
    { op: "set", key: "other", value: { n: 6 } },
  ]);
  const afterRetaken = await stored();
  // 500 fillers and `job`; each batch's sweep takes a hundred fillers, and
  // only the last batch, which applied, leaves `other` beside them.
  answer.a_batch_entrys_condition_declines_an_expired_key_a_sweep_has_not_reached =
    claim.applied === true &&
    claimed === FILLERS + 1 &&
    present.applied === false &&
    present.failed === 1 &&
    same(afterPresent, [1, 401]) &&
    versioned.applied === false &&
    versioned.failed === 0 &&
    same(afterVersioned, [1, 301]) &&
    deleted.applied === false &&
    deleted.failed === 1 &&
    same(afterDeleted, [1, 201]) &&
    declinedLast === claimed;
  answer.a_batch_entry_with_if_absent_takes_an_expired_key_a_sweep_has_not_reached =
    retaken.applied === true &&
    same(retaken.results, [
      { key: "job", version: claimed + 1 },
      { key: "other", version: claimed + 2 },
    ]) &&
    same(afterRetaken, [1, 102]) &&
    (await later(HOUR + MINUTE, () => get(store, "job", run))).value?.n === 5;
  answer.batch_lease_trace = {
    claim,
    present,
    afterPresent,
    versioned,
    afterVersioned,
    deleted,
    afterDeleted,
    declinedLast: declinedLast ?? null,
    retaken,
    afterRetaken,
  };
}

{
  // **The bound is the run's too**: a caller that skipped the compiler is
  // refused by name, before anything is read, rather than applied in part.
  const store = kv("batch_bound", "global", URL_ENV, BATCHED);
  const run = ctx(execution("batch_bound"));
  const many = Array.from({ length: 101 }, (_, index) => ({ op: "delete", key: `k${index}` }));
  answer.batch_bound_refusal = (await refusal(batch(store, many, run, `${RUN}/batch_bound`))) ?? null;
  answer.a_batch_of_101_entries_is_refused_by_name =
    typeof answer.batch_bound_refusal === "string" &&
    answer.batch_bound_refusal.includes("was handed 101 entries") &&
    (await refusal(batch(store, [], run, `${RUN}/batch_bound/empty`)))?.includes("was handed 0 entries") === true;
}

/**
 * Whether some other transaction holds an uncommitted write of `key` in
 * `store`'s global partition — asked by trying to write it from a connection of
 * this file's own, under a short lock timeout, and rolling the attempt back.
 *
 * A row another transaction inserted is invisible to every read but a dirty
 * one, and Postgres has none; an insert of the same key waits on it on both
 * servers, which is the one question both can be asked alike. A key nobody
 * holds is inserted and rolled back at once, which is the `false` answer.
 */
async function heldUncommitted(store, key) {
  const probe = await openServer(process.env[URL_ENV]);
  try {
    await begin(probe);
    try {
      if (provider === "postgres") {
        await rows(probe, "SET LOCAL lock_timeout = '500ms'");
        await rows(
          probe,
          'INSERT INTO store_entries (store, scope_key, "key", value) VALUES ($1, $2, $3, $4) ON CONFLICT DO NOTHING',
          [store.name, "global", key, "{}"],
        );
      } else {
        await rows(probe, "SET SESSION innodb_lock_wait_timeout = 1");
        await rows(
          probe,
          "INSERT IGNORE INTO store_entries (store, scope_key, `key`, value) VALUES (?, ?, ?, ?)",
          [store.name, "global", key, "{}"],
        );
      }
      return false;
    } catch (error) {
      const message = String(error?.message ?? error);
      return message.includes("lock timeout") || message.includes("Lock wait timeout");
    } finally {
      await rollback(probe).catch(() => undefined);
    }
  } finally {
    await probe.end().catch(() => undefined);
  }
}

{
  // **A batch killed between two entry writes leaves none of it** — on every
  // arm, by killing the writing process outright. `store-batch-crash.mjs` seeds
  // a key, then runs a batch whose second entry's value holds its own
  // serialization open for ever, after its first entry was written inside the
  // transaction; this process waits for the marker that says so, proves the
  // first entry was really written (SQLite's rollback journal is on disk; a
  // server holds an uncommitted insert of the key), and sends `SIGKILL`. What
  // is left is the seed and nothing of the batch: no key, no version, no
  // ledger row — so a retry of the same effect applies, once.
  //
  // On SQLite the store is one this process has never opened: the killed
  // writer's lock directory is still there, and the open below is what breaks
  // it once `LOCK_WAIT_MS` has passed and rolls the hot journal back, which is
  // the recovery a real crash gets.
  const name = `${RUN}_batch_killed`;
  const store = kv("batch_killed", "global", URL_ENV, BATCHED);
  const key = `${execution("batch_killed")}/move/0`;
  const marker = path.join(data, `${RUN}-batch-crash.marker`);
  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "store-batch-crash.mjs");
  try {
    fs.mkdirSync(data, { recursive: true });
    const child = spawn(process.execPath, [script, project, provider, data, name, key, marker], {
      env: process.env,
      stdio: ["ignore", "pipe", "pipe"],
    });
    let err = "";
    child.stderr.on("data", (chunk) => (err += chunk));
    const exited = new Promise((resolve) => child.on("close", (code, signal) => resolve({ code, signal })));
    const deadline = Date.now() + 30_000;
    let open = false;
    while (Date.now() < deadline && child.exitCode === null) {
      if (fs.existsSync(marker)) {
        open = true;
        break;
      }
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
    answer.the_batch_was_mid_transaction_when_it_was_killed = open;
    let written = false;
    if (open) {
      written =
        provider === "sqlite"
          ? fs.existsSync(path.join(data, "stores", `${stores.encodeKey(name)}.sqlite-journal`))
          : await heldUncommitted(store, "first");
    }
    answer.the_killed_batch_had_written_its_first_entry = written;
    child.kill("SIGKILL");
    const ended = await exited;
    answer.batch_crash_exit = { ...ended, stderr: err.slice(0, 2000) };
    const run = ctx(execution("batch_killed"));
    const [first, second, seeded] = await Promise.all(
      ["first", "second", "seeded"].map((entry) => get(store, entry, run)),
    );
    const left = {
      first: first.found,
      second: second.found,
      seeded,
      last: await sequenceLast(store),
      ledger: await ledgerRows(store, key),
    };
    answer.batch_crash_left = left;
    answer.a_batch_killed_mid_transaction_leaves_none_of_it =
      open &&
      written &&
      ended.signal === "SIGKILL" &&
      left.first === false &&
      left.second === false &&
      seeded.value?.n === 0 &&
      left.last === seeded.version &&
      left.ledger === 0;
    const retried = await batch(
      store,
      [
        { op: "set", key: "first", value: { n: 1 } },
        { op: "set", key: "second", value: { n: 2 } },
        { op: "delete", key: "seeded" },
      ],
      run,
      key,
    );
    const again = await batch(store, [{ op: "delete", key: "first" }], run, key);
    answer.a_batch_retried_after_its_crash_applies_once =
      retried.applied === true &&
      retried.deduped === false &&
      same(retried.results, [
        { key: "first", version: seeded.version + 1 },
        { key: "second", version: seeded.version + 2 },
        { key: "seeded" },
      ]) &&
      again.deduped === true &&
      same(again.results, retried.results) &&
      (await get(store, "first", run)).found === true;
  } catch (error) {
    answer.batch_crash_error = String(error?.message ?? error);
  }
}

if (provider !== "sqlite") {
  // **…and one whose connection the server ends between two entry writes.**
  // The dialled arm's own failure, driven the way the redial case below
  // drives it: a connection of this file's holds an uncommitted insert of the
  // batch's second key, so the batch — its first entry written inside the
  // transaction — waits on the server for it, and the server's own `KILL` /
  // `pg_terminate_backend` ends that session mid-transaction. The op is refused
  // by name, the server rolls the transaction back, and a retry applies once.
  const BATCH_URL_ENV = "AGENT_COMPOSE_STORE_URL_BATCH";
  process.env[BATCH_URL_ENV] = process.env[URL_ENV];
  const store = kv("batch_ended", "global", BATCH_URL_ENV, BATCHED);
  const run = ctx(execution("batch_ended"));
  const key = `${execution("batch_ended")}/move/0`;
  const ops = [
    { op: "set", key: "first", value: { n: 1 } },
    { op: "set", key: "second", value: { n: 2 } },
    { op: "delete", key: "seeded" },
  ];
  const killer = await openServer(process.env[URL_ENV]);
  let blocked;
  try {
    const seeded = await set(store, "seeded", { n: 0 }, run, `${RUN}/batch_ended/seed`);
    await begin(killer);
    await holdTheKey(killer, store.name, "second");
    blocked = batch(store, ops, run, key).then(
      () => undefined,
      (error) => String(error?.message ?? error),
    );
    const victim = await waitingOn(killer, store.name);
    answer.the_batch_was_caught_between_two_entries = victim !== undefined;
    if (victim !== undefined) {
      answer.the_ended_batch_had_written_its_first_entry = await heldUncommitted(store, "first");
      await kill(killer, victim);
      const refused = (await blocked) ?? "";
      answer.batch_ended_refusal = refused;
      await rollback(killer).catch(() => undefined);
      const [first, second, still] = await Promise.all(
        ["first", "second", "seeded"].map((entry) => get(store, entry, run)),
      );
      answer.a_batch_whose_connection_the_server_ends_mid_transaction_leaves_none_of_it =
        answer.the_ended_batch_had_written_its_first_entry === true &&
        refused.includes(`lost its connection to the \`${provider}\` store backend`) &&
        first.found === false &&
        second.found === false &&
        still.version === seeded.version &&
        (await sequenceLast(store)) === seeded.version &&
        (await ledgerRows(store, key)) === 0;
      const retried = await batch(store, ops, run, key);
      answer.a_batch_whose_connection_ended_is_retried_once =
        retried.applied === true &&
        retried.deduped === false &&
        same(retried.results, [
          { key: "first", version: seeded.version + 1 },
          { key: "second", version: seeded.version + 2 },
          { key: "seeded" },
        ]);
    }
  } catch (error) {
    answer.batch_ended_error = String(error?.message ?? error);
  } finally {
    await rollback(killer).catch(() => undefined);
    if (blocked !== undefined) await blocked;
    await killer.end().catch(() => undefined);
  }
}

/**
 * Run one statement against a SQLite store's own file, over a handle of this
 * file's — [`storage`]'s, for a statement that changes the file's schema
 * rather than reads its rows. The store's own handle stays open beside it and
 * sees the change at its next statement, as SQLite has every connection to one
 * file see another's.
 */
async function inStoreFile(store, statement) {
  const module = await import("node-sqlite3-wasm");
  const { Database } = module.default ?? module;
  const database = new Database(path.join(data, "stores", `${stores.encodeKey(store.name)}.sqlite`));
  try {
    database.exec("PRAGMA busy_timeout = 5000;");
    database.exec(statement);
  } finally {
    database.close();
  }
}

{
  // **A batch that fails at its ledger row leaves none of it** — the window
  // the two cases above cannot reach, because both end a batch before its
  // ledger insert. PRD resolved q71: "its whole answer is the ledger row, so a
  // retry answers what the first attempt did" — which holds only if that row
  // commits in the batch's own transaction. Written after the `COMMIT`, it
  // leaves a window in which every entry has landed and nothing records it,
  // and a retry under the same §9.4 key runs the batch again: its `set`s take
  // a second run of versions and its `if: absent` entries decline what the
  // first attempt wrote. Each arm is made to fail at exactly that insert, the
  // only way a test can reach into it:
  //
  // - **on a server**, a connection of this file's holds an uncommitted ledger
  //   row under the batch's own key ([`holdTheLedgerRow`]). The batch's ledger
  //   check does not see it, so it writes every entry and then waits on its
  //   own ledger insert — every entry still uncommitted, which an insert of
  //   each key proves — and the server ends that session;
  // - **on SQLite**, a trigger in the store's file refuses that one ledger
  //   row, and says in its refusal that every entry was visible to it — which,
  //   in a file one writer holds, means written by this batch.
  //
  // What is left either way is the seed and nothing of the batch: no key, no
  // version, no ledger row — so a retry applies once, and its own ledger row
  // answers the one after it. With the ledger insert moved past the `COMMIT`
  // on either arm, the entries are there and this case is `false`.
  const LEDGER_URL_ENV = "AGENT_COMPOSE_STORE_URL_BATCH_LEDGER";
  if (provider !== "sqlite") process.env[LEDGER_URL_ENV] = process.env[URL_ENV];
  const store = kv("batch_unledgered", "global", LEDGER_URL_ENV, BATCHED);
  const run = ctx(execution("batch_unledgered"));
  const key = `${execution("batch_unledgered")}/move/0`;
  const ops = [
    { op: "set", key: "first", value: { n: 1 } },
    { op: "set", key: "second", value: { n: 2 } },
    { op: "delete", key: "seeded" },
  ];
  // The trigger's two refusals: the first only where the batch's three
  // entries are all in the file as it reaches its ledger row.
  const WRITTEN = "the probe refuses this ledger row with every entry of its batch written";
  const UNWRITTEN = "the probe refuses this ledger row before every entry of its batch was written";
  const TRIGGER = "refuse_the_batch_ledger_row";
  const killer = provider === "sqlite" ? undefined : await openServer(process.env[URL_ENV]);
  let blocked;
  try {
    const seeded = await set(store, "seeded", { n: 0 }, run, `${RUN}/batch_unledgered/seed`);
    // What the batch failed with, and whether it failed where this case says:
    // at its ledger row, with every entry written inside its transaction.
    let refused;
    let atTheLedgerRow;
    if (killer === undefined) {
      const literal = (text) => `'${text.replaceAll("'", "''")}'`;
      await inStoreFile(
        store,
        `CREATE TRIGGER ${TRIGGER} BEFORE INSERT ON applied WHEN NEW.idempotency_key = ${literal(key)} ` +
          `BEGIN SELECT RAISE(ABORT, ${literal(WRITTEN)}) WHERE ` +
          "(SELECT COUNT(*) FROM entries WHERE scope_key = 'global' AND key IN ('first', 'second')) = 2 " +
          "AND NOT EXISTS (SELECT 1 FROM entries WHERE scope_key = 'global' AND key = 'seeded'); " +
          `SELECT RAISE(ABORT, ${literal(UNWRITTEN)}); END`,
      );
      try {
        refused = (await refusal(batch(store, ops, run, key))) ?? "";
      } finally {
        await inStoreFile(store, `DROP TRIGGER IF EXISTS ${TRIGGER}`);
      }
      atTheLedgerRow = refused.includes(WRITTEN);
    } else {
      await begin(killer);
      await holdTheLedgerRow(killer, store.name, key);
      blocked = batch(store, ops, run, key).then(
        () => undefined,
        (error) => String(error?.message ?? error),
      );
      const victim = await waitingOn(killer, store.name, "store_applied");
      answer.the_batch_was_caught_waiting_on_its_ledger_row = victim !== undefined;
      // Read only once the kill is in, as the redial case below says: a batch
      // still waiting on a lock nothing has released would hang this runner.
      if (victim === undefined) throw new Error("the batch was never seen waiting on its ledger row");
      const held = await Promise.all(["first", "second", "seeded"].map((entry) => heldUncommitted(store, entry)));
      answer.batch_unledgered_held = held;
      answer.the_batch_caught_at_its_ledger_row_held_every_entry_uncommitted = held.every((one) => one === true);
      await kill(killer, victim);
      refused = (await blocked) ?? "";
      // Before anything dials again — the redial's schema waits on this
      // transaction's lock — and after the kill, so the batch never finds the
      // row gone and claims it.
      await rollback(killer).catch(() => undefined);
      atTheLedgerRow =
        answer.the_batch_caught_at_its_ledger_row_held_every_entry_uncommitted &&
        refused.includes(`lost its connection to the \`${provider}\` store backend`);
    }
    answer.batch_unledgered_refusal = refused;
    const [first, second, still] = await Promise.all(
      ["first", "second", "seeded"].map((entry) => get(store, entry, run)),
    );
    const left = {
      first: first.found,
      second: second.found,
      seeded: still,
      last: await sequenceLast(store),
      ledger: await ledgerRows(store, key),
    };
    answer.batch_unledgered_left = left;
    answer.a_batch_that_fails_at_its_ledger_row_leaves_none_of_it =
      atTheLedgerRow &&
      left.first === false &&
      left.second === false &&
      still.value?.n === 0 &&
      still.version === seeded.version &&
      left.last === seeded.version &&
      left.ledger === 0;
    const retried = await batch(store, ops, run, key);
    const again = await batch(store, [{ op: "delete", key: "first" }], run, key);
    answer.a_batch_that_failed_at_its_ledger_row_is_retried_once =
      retried.applied === true &&
      retried.deduped === false &&
      same(retried.results, [
        { key: "first", version: seeded.version + 1 },
        { key: "second", version: seeded.version + 2 },
        { key: "seeded" },
      ]) &&
      again.deduped === true &&
      same(again.results, retried.results) &&
      (await get(store, "first", run)).found === true &&
      (await ledgerRows(store, key)) === 1;
  } catch (error) {
    answer.batch_unledgered_error = String(error?.message ?? error);
  } finally {
    if (killer !== undefined) {
      await rollback(killer).catch(() => undefined);
      if (blocked !== undefined) await blocked;
      await killer.end().catch(() => undefined);
    }
  }
}

{
  // **Two processes racing batches over the same keys in opposite orders**
  // (`store-batch-race.mjs`): each batch locks the partition's sequence first
  // and its keys after, so the second of two waits for the first to commit
  // before it reads anything — never on a key the first is about to reach for
  // while holding one it needs — and they apply one after the other. So every
  // batch of both writers applies with no error (a deadlock the server broke
  // would be one failing); every version a `set` entry took is the partition's
  // own, no two alike and each batch's a run of consecutive ones in entry
  // order, as one batch alone takes them; a reader asking for all six keys at
  // once, in a third process, never finds two batches' tags among them; and
  // the six end holding the one batch that committed last, at its versions. A
  // second store races batches of fresh keys into the two writers' own
  // adjacent partitions, where a server's gap locks would be the circle.
  const script = path.join(path.dirname(fileURLToPath(import.meta.url)), "store-batch-race.mjs");
  const done = path.join(data, `${RUN}-batch-race.done`);
  const ROUNDS = 100;
  const KEYS = ["a", "b", "c", "d", "e", "z"];
  const startAt = Date.now() + 2_500;
  fs.mkdirSync(data, { recursive: true });
  const racer = (role) =>
    new Promise((resolve, reject) => {
      const child = spawn(process.execPath, [script, project, provider, data, RUN, role, String(startAt), done], {
        env: process.env,
        stdio: ["ignore", "pipe", "pipe"],
      });
      let out = "";
      let err = "";
      child.stdout.on("data", (chunk) => (out += chunk));
      child.stderr.on("data", (chunk) => (err += chunk));
      child.on("error", reject);
      child.on("close", (code) =>
        code === 0 ? resolve(JSON.parse(out)) : reject(new Error(`the batch racer ${role} exited ${code}: ${err}`)),
      );
    });
  const reader = racer("reader");
  // A rejection nothing is listening to yet would end this process.
  const read = reader.then(
    (answered) => ({ answered }),
    (error) => ({ error }),
  );
  try {
    const writers = await Promise.allSettled([racer("forward"), racer("reverse")]);
    fs.writeFileSync(done, "done");
    const outcome = await read;
    for (const settled of writers) if (settled.status === "rejected") throw settled.reason;
    if (outcome.error !== undefined) throw outcome.error;
    const [forward, reverse] = writers.map((settled) => settled.value);
    const sides = { forward, reverse };

    /** Every batch of one phase, tagged with its writer and round. */
    const batches = (phase) =>
      Object.entries(sides).flatMap(([role, side]) => side[phase].map((answered, round) => ({ role, round, answered })));
    const failures = (phase) =>
      batches(phase).filter(({ answered }) => answered.error !== undefined || answered.applied !== true);
    /** A batch's `set` versions, in entry order. */
    const versionsOf = (answered) => (answered.results ?? []).filter((row) => "version" in row).map((row) => row.version);
    /** Whether a batch's versions are a run of consecutive ones, as one batch takes them. */
    const consecutive = (versions) => versions.every((version, index) => index === 0 || version === versions[index - 1] + 1);
    /** Every batch of the phase applied, each one's versions a consecutive run of one per `set` entry. */
    const applied = (phase) => {
      const all = batches(phase);
      return (
        all.length === 2 * ROUNDS &&
        failures(phase).length === 0 &&
        all.every(({ answered }) => versionsOf(answered).length === KEYS.length && consecutive(versionsOf(answered)))
      );
    };

    const global_ = batches("global");
    // A batch that waited on another overlaps it in time; with none
    // overlapping, the case would have raced nothing.
    const overlapping = forward.global.filter((mine) =>
      reverse.global.some((theirs) => mine.started < theirs.ended && theirs.started < mine.ended),
    ).length;
    answer.two_processes_racing_batches_really_overlapped = overlapping > 0;
    answer.two_processes_racing_batches_over_shared_keys_in_opposite_orders_all_apply =
      global_.length === 2 * ROUNDS && failures("global").length === 0;
    const versions = global_.flatMap(({ answered }) => versionsOf(answered));
    answer.two_processes_racing_batches_never_take_one_version =
      applied("global") && new Set(versions).size === versions.length;

    // The batch that committed last is the one holding the highest versions,
    // and the six keys hold its tag at the versions it answered.
    const last = global_.reduce((best, entry) =>
      Math.max(...versionsOf(entry.answered)) > Math.max(...versionsOf(best.answered)) ? entry : best,
    );
    const store = kv("batch_race", "global");
    const run = ctx(execution("batch_race_read"));
    const held = await stores.runStoreOp(store, "get_many", { keys: KEYS }, run, node());
    const lastVersions = Object.fromEntries((last.answered.results ?? []).filter((row) => "version" in row).map((row) => [row.key, row.version]));
    answer.every_key_two_racing_batches_wrote_holds_the_last_batchs_value =
      held.entries.every(
        (entry) =>
          entry.found === true &&
          entry.value?.by === last.role &&
          entry.value?.n === last.round &&
          entry.version === lastVersions[entry.key],
      ) && (await sequenceLast(store)) === Math.max(...Object.values(lastVersions));

    /** A read of the six keys that found more than one batch's tag among them, or a key missing. */
    const torn = (answered) => {
      if (answered.error !== undefined) return true;
      const found = answered.entries.filter((entry) => entry.found);
      if (found.length === 0) return false;
      if (found.length !== KEYS.length) return true;
      return new Set(found.map((entry) => `${entry.value?.by}/${entry.value?.n}`)).size !== 1;
    };
    const reads = outcome.answered.reads;
    const tornReads = reads.filter(torn);
    // A read between the first commit and the last is one the race could have torn.
    const midRace = reads.filter(
      (answered) =>
        answered.error === undefined &&
        answered.entries.some((entry) => entry.found) &&
        !answered.entries.every((entry) => entry.value?.by === last.role && entry.value?.n === last.round),
    ).length;
    answer.a_reader_beside_two_racing_batches_never_finds_one_half_landed =
      reads.length > 0 && tornReads.length === 0 && midRace > 0;

    // Fresh keys into the two writers' own adjacent partitions: every batch
    // applies, and each partition's versions are its own sequence's, 1
    // upwards with none twice — the two partitions' sequences are two.
    const partitionVersions = (role) => sides[role].session.flatMap(versionsOf);
    answer.two_processes_racing_batches_into_adjacent_partitions_all_apply =
      applied("session") &&
      ["forward", "reverse"].every((role) =>
        same(
          [...partitionVersions(role)].sort((left, right) => left - right),
          Array.from({ length: ROUNDS * KEYS.length }, (_, index) => index + 1),
        ),
      );
    answer.batch_race_trace = {
      partitions: Object.fromEntries(
        ["forward", "reverse"].map((role) => {
          const versions = [...partitionVersions(role)].sort((left, right) => left - right);
          return [role, { count: versions.length, first: versions[0] ?? null, last: versions.at(-1) ?? null }];
        }),
      ),
      overlapping,
      failures: [...failures("global"), ...failures("session")].slice(0, 3),
      reads: reads.length,
      midRace,
      torn: tornReads.slice(0, 3),
      last: { role: last.role, round: last.round, results: last.answered.results },
    };
  } catch (error) {
    answer.batch_race_error = String(error?.message ?? error);
  } finally {
    // A reader still reading when a writer failed stops at the marker.
    fs.writeFileSync(done, "done");
    await read;
  }
}

// --- Ledger retention (PRD resolved q69) --------------------------------------
//
// Driven through the runtime's own lifecycle — `openExecution`, then
// `settleExecution` and `closeExecution`, exactly as `runFlow` calls them — so
// what prunes the ledger is the hook a settled execution really reaches, over
// this arm's store. Each store is registered as `src/graph.ts` registers every
// store a composition binds (`stores.bindsStores`), because that set — not the
// writes this process happened to make — is what a settlement prunes.

/** Begin one execution's journal row, as `runFlow` does before its graph. */
const begin_ = (id, extra = {}) =>
  runtime.openExecution({
    execution: id,
    flow: "flow.probe",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    ...extra,
  });

/**
 * Settle and close one execution, as `runFlow` does on its way out — and then
 * wait for the ledger prune the settlement started, which nothing in the
 * settlement itself waits for (`stores.pruneLedger`): a case reading what the
 * prune left reads it after the prune, as a command's `releaseStores` does.
 */
const settle = async (id, error = undefined) => {
  try {
    await runtime.settleExecution(id, error);
  } finally {
    runtime.closeExecution(id);
  }
  await stores.ledgerPrunesSettled();
};

{
  // **A store taking writes across N executions holds a bounded ledger**: each
  // keyed write leaves a row while its execution is open, and the row goes when
  // the execution settles — completed or failed — while the data it wrote
  // stays.
  const store = kv("ledger", "global");
  stores.bindsStores([store]);
  const ids = Array.from({ length: 6 }, (_, index) => execution(`ledger_${index}`));
  const held = [];
  for (const [index, id] of ids.entries()) {
    await begin_(id);
    for (let write_ = 0; write_ < 3; write_ += 1) {
      await set(store, `k-${index}-${write_}`, { n: write_ }, ctx(id), `${id}/save/${write_}`);
    }
    held.push(await ledgerRows(store, `${id}/`));
    await settle(id, index % 2 === 0 ? undefined : new Error("this execution failed"));
  }
  const left = await ledgerRows(store, `exec_${RUN}_ledger_`);
  const data = (await list(store, ctx(execution("ledger_read")), "k-", 1000)).keys.length;
  answer.an_open_executions_writes_hold_their_ledger_rows = held.every((count) => count === 3);
  answer.settled_executions_leave_no_ledger_rows = left === 0;
  answer.a_settled_executions_data_stays = data === 18;
  answer.ledger_trace = { held, left, data };
}

{
  // **An execution the journal still holds open keeps every row**, so its
  // resume still writes nothing twice; and a prune is the settled execution's
  // own prefix, never another's whose id merely begins with it.
  const store = kv("ledger_open", "global");
  stores.bindsStores([store]);
  const parked = execution("ledger_open_1");
  const neighbour = execution("ledger_open_10");
  await begin_(parked);
  await set(store, "a", { n: 1 }, ctx(parked), `${parked}/save/0`);
  // A generation that stops without settling — a crash, a parked pause.
  runtime.closeExecution(parked);
  await begin_(neighbour);
  await set(store, "b", { n: 1 }, ctx(neighbour), `${neighbour}/save/0`);
  const kept = await ledgerRows(store, `${parked}/`);
  // The resume: the same write at the same site is answered from the ledger.
  await begin_(parked, { resuming: true });
  const resumed = await set(store, "a", { n: 2 }, ctx(parked), `${parked}/save/0`);
  await settle(parked);
  const pruned = await ledgerRows(store, `${parked}/`);
  const neighbourKept = await ledgerRows(store, `${neighbour}/`);
  await settle(neighbour);
  answer.an_unsettled_executions_ledger_rows_are_kept = kept === 1;
  answer.a_resume_of_an_unsettled_execution_still_dedupes =
    resumed.deduped === true && resumed.applied === true &&
    (await get(store, "a", ctx(execution("ledger_open_read")))).value?.n === 1;
  answer.a_prune_reaches_only_its_own_executions_prefix = pruned === 0 && neighbourKept === 1;
  answer.ledger_open_trace = { kept, resumed, pruned, neighbourKept };
}

{
  // **A child execution's rows go when the child settles**, under the child's
  // own id, while its parent — still open — keeps its own.
  const store = kv("ledger_child", "global");
  stores.bindsStores([store]);
  const parent = execution("ledger_parent");
  await begin_(parent);
  await set(store, "p", { n: 1 }, ctx(parent), `${parent}/dispatch/0/0/save/0`);
  const dispatch = `${parent}/dispatch/0/0`;
  const child = runtime.childExecutionId(parent, dispatch);
  await begin_(child, { lineage: { parent, idempotencyKey: dispatch } });
  await set(store, "c", { n: 1 }, ctx(child), `${child}/save/0`);
  const before = [await ledgerRows(store, `${parent}/`), await ledgerRows(store, `${child}/`)];
  await settle(child);
  const childSettled = [await ledgerRows(store, `${parent}/`), await ledgerRows(store, `${child}/`)];
  await settle(parent);
  const parentSettled = await ledgerRows(store, `${parent}/`);
  answer.a_childs_ledger_rows_go_when_the_child_settles =
    same(before, [1, 1]) && same(childSettled, [1, 0]);
  answer.a_parents_ledger_rows_go_when_the_parent_settles = parentSettled === 0;
  answer.ledger_child_trace = { before, childSettled, parentSettled };
}

{
  // **The two outcomes a settlement leaves open keep every row through it**: a
  // run that reached a `human` pause nobody could answer, and a resume whose
  // record this build disagrees with (`runtime.settleExecution`, PRD resolved
  // q28). Each is settled exactly as `runFlow` settles it — through
  // `settleExecution`, with the error it really carries, wrapped the way a
  // node failure wraps it — and both are what a later resume replays, so a
  // prune of either would make that resume write a second time.
  const store = kv("ledger_unsettled", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const parked = execution("ledger_parked");
  const diverged = execution("ledger_diverged");
  const bump = (id) => write(store, "incr", { key: id, field: "count" }, ctx(id), `${id}/count/0`);
  const outcomes = {
    [parked]: new Error("flow.probe node `ask` failed", {
      cause: new runtime.HumanInterrupt("flow.probe", "ask", `${parked}/ask`, { prompt: "?" }),
    }),
    [diverged]: new journal.ReplayDivergence(
      { key: `${diverged}/count/0`, site: "flow.probe/count", kind: "store", ordinal: 0 },
      "the record and this build's graph disagree",
    ),
  };
  const kept = {};
  const open = {};
  const resumed = {};
  for (const id of [parked, diverged]) {
    await begin_(id);
    await bump(id);
    await settle(id, outcomes[id]);
    kept[id] = await ledgerRows(store, `${id}/`);
    open[id] = (await runtime.journaledExecution(id))?.status;
    // The resume: the same write at the same site, answered from the row the
    // settlement kept rather than applied a second time.
    await begin_(id, { resuming: true });
    resumed[id] = await bump(id);
    await settle(id);
  }
  const counted = async (id) => (await get(store, id, ctx(execution("ledger_unsettled_read")))).value?.count;
  const keeps = async (id) =>
    kept[id] === 1 &&
    open[id] === "open" &&
    resumed[id].deduped === true &&
    (await counted(id)) === 1 &&
    (await ledgerRows(store, `${id}/`)) === 0;
  answer.a_parked_executions_settlement_keeps_its_ledger_rows = await keeps(parked);
  answer.a_diverged_executions_settlement_keeps_its_ledger_rows = await keeps(diverged);
  answer.ledger_unsettled_trace = { kept, open, resumed };
}

{
  // **A keyed write another process made is pruned by the process that
  // settles**: the hub settles an execution whose placed node wrote from its
  // worker, and never saw that write (PRD resolved q69 (d), "in every store
  // partition the execution wrote"). Here the write is a second process's and
  // this one has not written the store under that execution at all.
  const store = kv("ledger_placed", "global");
  stores.bindsStores([store]);
  const id = execution("ledger_placed");
  try {
    await begin_(id);
    const written = await helper("store-placed-write.mjs", [
      project,
      provider,
      data,
      JSON.stringify(store),
      id,
      `${id}/placed/0`,
    ]);
    const before = await ledgerRows(store, `${id}/`);
    await settle(id);
    const after = await ledgerRows(store, `${id}/`);
    answer.a_keyed_write_another_process_made_is_pruned_at_settlement =
      written.answer.applied === true && before === 1 && after === 0;
    answer.ledger_placed_trace = { written: written.answer, before, after };
  } catch (error) {
    answer.ledger_placed_error = String(error?.message ?? error);
  }
}

/**
 * Dispatch one detached `agent.worker` delivery under execution `id` through
 * the real `runtime.runMap` — a `map` of one item with a `detach: true` route —
 * whose body is `deliver`. Answers once the `map` has returned, which a
 * detached route does without waiting for its delivery (grammar 8.6 rule 7).
 */
const dispatchDetached = (id, deliver) => {
  const map = {
    node: "fan",
    as: "item",
    source: { path: "state.items", shape: "any" },
    maxConcurrency: 1,
    onItemError: "fail",
    routes: [
      {
        target: "agent.worker",
        maxConcurrency: 1,
        detach: true,
        itemShape: "any",
        input: (roots) => runtime.toJson(runtime.evaluate("item", roots)),
        writes: [],
        run: deliver,
      },
    ],
  };
  const run = { ...runtime.emptyRun(), execution: { id, session_key: "" } };
  const view = {
    state: { items: [{ at: 0 }] },
    run,
    roots: {
      input: runtime.bind({}, { properties: {} }),
      state: runtime.bind({ items: [{ at: 0 }] }, { properties: { items: { items: "any" } } }),
      execution: runtime.bind(run.execution, {
        properties: { id: "string", session_key: "string", item_index: "int" },
      }),
    },
  };
  return runtime.runMap(map, runtime.mapPlan(map, view), {
    execution: { id, session_key: "" },
    signal: new AbortController().signal,
    node: "fan",
  });
};

/** A promise and the function that settles it. */
const gate = () => {
  let open = () => undefined;
  const opened = new Promise((resolve) => {
    open = resolve;
  });
  return { open, opened };
};

{
  // **A detached delivery still running when its execution settles keeps the
  // execution's ledger rows until it finishes.** A detached `map` delivery
  // outlives the run that dispatched it (grammar 8.6 rule 7), and every effect
  // it makes is that execution's, under its id — a flow an agent calls as a
  // tool, `store:` nodes and all. Here it is the real `runtime.runMap` with a
  // detached route whose instance makes a keyed `incr`, then fails after it as
  // a model call that follows a tool call would, and is retried: the retry
  // re-derives the same key and must be answered from the row the first
  // attempt left, even though the run has settled in between.
  const store = kv("ledger_detached", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const id = execution("ledger_detached");
  const { open: releaseRetry, opened: retry } = gate();
  const { open: firstLanded, opened: landed } = gate();
  const attempts = [];
  const bump = () => write(store, "incr", { key: "delivered", field: "count" }, ctx(id), `${id}/fan/0#agent/0/count/0`);
  try {
    await begin_(id);
    await dispatchDetached(id, async () => {
      attempts.push(await bump());
      firstLanded();
      await retry;
      attempts.push(await bump());
      return { output: {} };
    });
    await landed;
    // The run is over: its graph returned, and it settles with its delivery's
    // first attempt applied and its retry still to come.
    await settle(id);
    const during = await ledgerRows(store, `${id}/`);
    releaseRetry();
    await runtime.settleDetached(id);
    // The prune that waited for the delivery runs once it has finished, on its
    // own and unawaited by anyone, so it is waited for here by its effect.
    let after = await ledgerRows(store, `${id}/`);
    for (let polls = 0; after !== 0 && polls < 50; polls += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
      after = await ledgerRows(store, `${id}/`);
    }
    const held = await get(store, "delivered", ctx(execution("ledger_detached_read")));
    answer.a_detached_delivery_running_at_settlement_keeps_its_ledger_rows = during === 1;
    answer.a_retry_inside_a_detached_delivery_after_settlement_still_dedupes =
      attempts.length === 2 &&
      attempts[0].deduped === false &&
      attempts[1].deduped === true &&
      held.value?.count === 1;
    answer.a_detached_deliverys_rows_go_once_it_has_finished = after === 0;
    answer.ledger_detached_trace = { during, after, attempts, held };
  } catch (error) {
    answer.ledger_detached_error = String(error?.stack ?? error);
  }
}

{
  // **A command letting go of its stores prunes what a running delivery was
  // holding back, and the delivery may not write without its ledger from
  // then on.** `agent-compose run` exits without waiting for a detached
  // `agent.*` or `tool.*` delivery, so a prune chained behind one would never
  // run and every such run would leave its execution's rows for ever; the exit
  // starts it instead (`stores.releaseStores`). The delivery is still running
  // in this process until the exit, though, and a retry it makes after the
  // prune would find no row to answer it — so its keyed write is refused
  // rather than applied a second time.
  const store = kv("ledger_cut_off", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const id = execution("ledger_cut_off");
  const { open: releaseRetry, opened: retry } = gate();
  const { open: firstLanded, opened: landed } = gate();
  const attempts = [];
  let refused;
  const bump = () => write(store, "incr", { key: "delivered", field: "count" }, ctx(id), `${id}/fan/0#agent/0/count/0`);
  try {
    await begin_(id);
    await dispatchDetached(id, async () => {
      attempts.push(await bump());
      firstLanded();
      await retry;
      try {
        attempts.push(await bump());
      } catch (error) {
        refused = String(error?.message ?? error);
      }
      return { output: {} };
    });
    await landed;
    await settle(id);
    const during = await ledgerRows(store, `${id}/`);
    // The command's way out, with the delivery still holding its retry.
    await stores.releaseStores();
    const released = await ledgerRows(store, `${id}/`);
    releaseRetry();
    await runtime.settleDetached(id);
    const held = await get(store, "delivered", ctx(execution("ledger_cut_off_read")));
    answer.a_command_letting_go_prunes_what_a_running_delivery_held_back =
      during === 1 && released === 0;
    answer.a_delivery_cut_off_by_its_command_is_refused_a_keyed_write =
      attempts.length === 1 &&
      (refused ?? "").includes("has already deleted its idempotency ledger") &&
      held.value?.count === 1;
    answer.ledger_cut_off_trace = { during, released, attempts, refused, held };
  } catch (error) {
    answer.ledger_cut_off_error = String(error?.stack ?? error);
  }
}

{
  // **A stopped `serve` drains its stores the same way, and closes nothing
  // under the executions it leaves running.** A `serve` stopped on a deploy
  // exits with deliveries still in flight, and recovery never comes back for a
  // settled execution, so its stop starts the prune a running delivery was
  // holding back exactly as a command letting go does (`stores.drainStores`) —
  // and refuses that delivery's keyed write from then on. What it does not do
  // is close a connection: the executions still open keep running while it
  // stops, and the next start recovers them, so one that writes meanwhile is
  // answered and keeps the ledger rows its resume will need.
  const store = kv("ledger_drained", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const id = execution("ledger_drained");
  const open = execution("ledger_drained_open");
  const { open: releaseRetry, opened: retry } = gate();
  const { open: firstLanded, opened: landed } = gate();
  const attempts = [];
  let refused;
  const bump = () => write(store, "incr", { key: "delivered", field: "count" }, ctx(id), `${id}/fan/0#agent/0/count/0`);
  const running = (n) =>
    write(store, "incr", { key: "running", field: "count" }, ctx(open), `${open}/step/${n}`);
  try {
    await begin_(id);
    await begin_(open);
    await dispatchDetached(id, async () => {
      attempts.push(await bump());
      firstLanded();
      await retry;
      try {
        attempts.push(await bump());
      } catch (error) {
        refused = String(error?.message ?? error);
      }
      return { output: {} };
    });
    await landed;
    await settle(id);
    const before = await running(0);
    const during = await ledgerRows(store, `${id}/`);
    // The stop, with the delivery still holding its retry and the other
    // execution still open.
    await stores.drainStores();
    const drained = await ledgerRows(store, `${id}/`);
    const after = await running(1);
    const kept = await ledgerRows(store, `${open}/`);
    releaseRetry();
    await runtime.settleDetached(id);
    const held = await get(store, "delivered", ctx(execution("ledger_drained_read")));
    answer.a_stopped_serve_prunes_what_a_running_delivery_held_back =
      during === 1 && drained === 0;
    answer.a_delivery_cut_off_by_a_stopped_serve_is_refused_a_keyed_write =
      attempts.length === 1 &&
      (refused ?? "").includes("has already deleted its idempotency ledger") &&
      held.value?.count === 1;
    answer.an_open_execution_still_writes_while_a_serve_stops =
      before.applied === true &&
      after.applied === true &&
      after.value?.count === 2 &&
      kept === 2;
    answer.ledger_drained_trace = { during, drained, before, after, kept, attempts, refused, held };
  } catch (error) {
    answer.ledger_drained_error = String(error?.stack ?? error);
  } finally {
    runtime.closeExecution(open);
  }
}

if (provider !== "sqlite") {
  // **A bound store whose server never answers delays no settlement and no
  // other store's prune, and holds a command's exit for its dial deadline
  // alone.** A settlement prunes every store the composition binds, the ones
  // the execution never reached included — so a server that accepts a
  // connection and then says nothing is one every settled execution meets.
  // Here it is a socket this file listens on and never writes to, bound as a
  // store of this arm's provider ahead of one the execution really wrote.
  const net = await import("node:net");
  const sockets = new Set();
  const listener = net.createServer((socket) => {
    sockets.add(socket);
    socket.on("error", () => undefined);
  });
  await new Promise((resolve) => listener.listen(0, "127.0.0.1", resolve));
  const SILENT_URL_ENV = "AGENT_COMPOSE_STORE_URL_SILENT";
  process.env[SILENT_URL_ENV] = `${provider}://silent:silent@127.0.0.1:${listener.address().port}/silent`;
  const silent = kv("ledger_silent", "global", SILENT_URL_ENV);
  const answered = kv("ledger_answered", "global");
  stores.bindsStores([silent, answered]);
  const id = execution("ledger_silent");
  try {
    await begin_(id);
    await set(answered, "k", { n: 1 }, ctx(id), `${id}/save/0`);
    const settling = Date.now();
    await runtime.settleExecution(id);
    runtime.closeExecution(id);
    const settledIn = Date.now() - settling;
    const status = (await runtime.journaledExecution(id))?.status;
    // The store that answers is pruned while the one that does not is still
    // being dialled: each store is pruned on its own.
    let pruned = await ledgerRows(answered, `${id}/`);
    for (let polls = 0; pruned !== 0 && polls < 30; polls += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
      pruned = await ledgerRows(answered, `${id}/`);
    }
    const prunedIn = Date.now() - settling;
    const releasing = Date.now();
    await stores.releaseStores();
    const releasedIn = Date.now() - releasing;
    answer.a_store_that_never_answers_delays_no_settlement =
      settledIn < 2_000 && status === "completed";
    answer.a_store_that_never_answers_delays_no_other_stores_prune =
      pruned === 0 && prunedIn < 5_000;
    answer.a_store_that_never_answers_holds_a_command_for_its_dial_deadline_alone =
      sockets.size > 0 && releasedIn < 15_000;
    answer.ledger_silent_trace = { settledIn, status, pruned, prunedIn, releasedIn, dials: sockets.size };
  } catch (error) {
    answer.ledger_silent_error = String(error?.stack ?? error);
  } finally {
    // Unset, so no later settlement in this file prunes it again.
    delete process.env[SILENT_URL_ENV];
    for (const socket of sockets) socket.destroy();
    await new Promise((resolve) => listener.close(resolve));
  }
}

// --- A placed node a worker may still be running (PRD resolved q69, q63) ------
//
// The hub superseding a placed node's dispatch — the session holding it gone
// quiet past the liveness window, the artifact it was issued under replaced,
// its node's deadline spent — ends the *attempt*, not the worker's node runner
// (`docs/distributed.md` §6.3): the runner goes on, writing a dialled store
// directly under the same grammar 9.4 keys as the retry that replaced it, and
// its late write is answered from the retry's ledger row (PRD resolved q63) for
// exactly as long as that row is there. So a settlement leaves every row of an
// execution a worker was handed a node of that no result has ended. Driven
// through the hub's own dispatch rows in the journal — parked, claimed by a
// session, then superseded, still held, or settled by a result — and the
// runtime's own settle.

/**
 * Lay down one placed dispatch of `id` at `wait` and take it as far as `end`:
 * `parked` (no worker took it), `superseded` before any worker took it
 * (`dropped`), or claimed by a session and then left `held`, `superseded` or
 * `settled` by a result.
 */
const placedDispatch = async (handle, id, wait, end) => {
  const row = await handle.park({
    execution: id,
    wait,
    id: `dsp_${id}_${wait.replaceAll("/", "_")}`,
    placement: "laptop",
    node: "flow.probe.count",
    site: "count",
    inputs: {},
    parkedAt: new Date().toISOString(),
  });
  if (end === "dropped") {
    await handle.supersedeDispatch(row.id, "the hub stopped waiting for this dispatch: the node execution ended");
    return;
  }
  if (end === "parked") return;
  await handle.claimDispatch(row.id, `ses_${id}`);
  if (end === "superseded") {
    await handle.supersedeDispatch(row.id, "the session holding it stopped making requests");
  } else if (end === "settled") {
    await handle.settleDispatch(row.id, { kind: "value", value: {} });
  }
};

{
  const store = kv("ledger_placed_runner", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const handle = await journal.openJournal();
  const bump = (id) => write(store, "incr", { key: id, field: "count" }, ctx(id), `${id}/count/0`);
  const counted = async (id) => (await get(store, id, ctx(execution("ledger_placed_runner_read")))).value?.count;
  const left = {};
  try {
    // The finding's own sequence: the first worker's dispatch is superseded,
    // the retry's — a second dispatch — lands the `incr` and is settled by its
    // result, the execution settles, and then the first worker's runner, still
    // going, issues the same `incr` under the same key.
    const superseded = execution("ledger_runner_superseded");
    await begin_(superseded);
    await placedDispatch(handle, superseded, "count/0", "superseded");
    await placedDispatch(handle, superseded, "count/1", "settled");
    const retried = await bump(superseded);
    await settle(superseded);
    left.superseded = await ledgerRows(store, `${superseded}/`);
    const late = await bump(superseded);
    answer.a_superseded_placed_dispatch_keeps_its_executions_ledger_rows = left.superseded === 1;
    answer.a_superseded_runners_late_write_after_settlement_still_dedupes =
      retried.applied === true &&
      retried.deduped === false &&
      late.deduped === true &&
      (await counted(superseded)) === 1;

    // A dispatch a session still holds as its execution settles is a runner
    // the hub has not ended at all — the execution failing around a placed
    // node that is still running, its supersession not yet written.
    const held = execution("ledger_runner_held");
    await begin_(held);
    await placedDispatch(handle, held, "count/0", "held");
    await bump(held);
    await settle(held, new Error("another branch failed"));
    left.held = await ledgerRows(store, `${held}/`);
    answer.a_placed_dispatch_still_held_at_settlement_keeps_its_executions_ledger_rows = left.held === 1;

    // …while a dispatch a result settled had a runner that has finished, and
    // one no worker took had none: neither holds anything back.
    const settledByResult = execution("ledger_runner_settled");
    await begin_(settledByResult);
    await placedDispatch(handle, settledByResult, "count/0", "settled");
    await bump(settledByResult);
    await settle(settledByResult);
    left.settled = await ledgerRows(store, `${settledByResult}/`);
    const untaken = execution("ledger_runner_untaken");
    await begin_(untaken);
    await placedDispatch(handle, untaken, "count/0", "dropped");
    await placedDispatch(handle, untaken, "count/1", "parked");
    await bump(untaken);
    await settle(untaken);
    left.untaken = await ledgerRows(store, `${untaken}/`);
    answer.a_placed_dispatch_a_result_settled_does_not_hold_the_prune = left.settled === 0;
    answer.a_placed_dispatch_no_worker_took_does_not_hold_the_prune = left.untaken === 0;
    answer.ledger_placed_runner_trace = { left, retried, late };
  } catch (error) {
    answer.ledger_placed_runner_error = String(error?.stack ?? error);
  }
}

{
  // **…and in a hub, a delivery still in flight at settlement can hand a node
  // out afterwards**: a detached `map` delivery outlives its run (grammar 8.6
  // rule 7) and may reach a placed component, so a process that hands placed
  // nodes to workers asks the journal again as the prune it was holding back
  // starts. Two executions, each settling under a delivery that then hands a
  // node of it to a worker: one dispatch is superseded and its execution keeps
  // its rows; the other is settled by its result and its rows go — which is
  // also the proof that both deferred prunes ran.
  const store = kv("ledger_handed_out", "global", URL_ENV, z.object({ count: z.number().int() }).strict());
  stores.bindsStores([store]);
  const handle = await journal.openJournal();
  const bump = (id) => write(store, "incr", { key: id, field: "count" }, ctx(id), `${id}/fan/0#agent/0/count/0`);
  // What `./mesh.ts` marks as it mounts the worker routes of a `serve`.
  runtime.handsOutPlacedNodes();
  const ends = {
    [execution("ledger_handed_superseded")]: "superseded",
    [execution("ledger_handed_settled")]: "settled",
  };
  const { open: handOut, opened: handing } = gate();
  const handed = [];
  try {
    for (const [id, end] of Object.entries(ends)) {
      await begin_(id);
      await dispatchDetached(id, async () => {
        await handing;
        await placedDispatch(handle, id, "fan/0/count/0", end);
        handed.push(await bump(id));
        return { output: {} };
      });
      await settle(id);
    }
    handOut();
    for (const id of Object.keys(ends)) await runtime.settleDetached(id);
    const [superseded, settledByResult] = Object.keys(ends);
    let gone = await ledgerRows(store, `${settledByResult}/`);
    for (let polls = 0; gone !== 0 && polls < 50; polls += 1) {
      await new Promise((resolve) => setTimeout(resolve, 100));
      gone = await ledgerRows(store, `${settledByResult}/`);
    }
    await stores.ledgerPrunesSettled();
    const kept = await ledgerRows(store, `${superseded}/`);
    answer.a_node_a_delivery_hands_out_after_settlement_holds_the_prune_when_superseded =
      handed.length === 2 && handed.every((written) => written.applied === true) && kept === 1;
    answer.a_node_a_delivery_hands_out_after_settlement_is_pruned_behind_its_result = gone === 0;
    answer.ledger_handed_out_trace = { kept, gone, handed };
  } catch (error) {
    answer.ledger_handed_out_error = String(error?.stack ?? error);
  }
}

// --- Two writers, one store, last write wins ---------------------------------

{
  // Two bindings naming two variables that hold one address, which is two
  // connections to one server — the multi-writer posture PRD resolved q63
  // states, and the property grammar 14.1 rule 5 admits a dialled backend for.
  // On the local backend the two are one process by design, and the case still
  // says what it says about the key.
  const mine = kv("shared", "global", URL_ENV);
  const theirs = kv("shared", "global", OTHER_URL_ENV);
  const run = ctx(execution("shared"));
  await set(mine, "k", { writer: "a" }, run, "w/1");
  answer.a_second_writer_reads_what_the_first_wrote =
    (await get(theirs, "k", run)).value?.writer === "a";
  await set(theirs, "k", { writer: "b" }, run, "w/2");
  const afterB = await get(mine, "k", run);
  await set(mine, "k", { writer: "a-again" }, run, "w/3");
  const afterA = await get(theirs, "k", run);
  answer.two_writers_share_one_store_and_the_last_write_wins =
    afterB.value?.writer === "b" && afterA.value?.writer === "a-again";
}

// --- A second open finds what the first wrote --------------------------------

{
  // The DDL runs on every open and has to be idempotent: a store this build
  // already created is opened again here, against tables that already exist.
  const store = kv("reopened", "global");
  await set(store, "k", { held: true }, ctx(execution("reopen_1")), "x/1");
  await stores.releaseStores();
  answer.a_second_open_finds_what_the_first_wrote =
    (await get(store, "k", ctx(execution("reopen_2")))).value?.held === true;
}

// --- Replay discipline: reads recorded, writes not applied twice -------------

{
  // The arms slot under the recording layer unchanged (PRD 5.8): a resumed
  // generation answers a read out of the record and does not apply a write a
  // second time. Driven here because it is the one claim whose failure would be
  // per-backend — the record is the journal's, and what it holds is whatever the
  // store answered.
  const handle = await journal.openJournal();
  const store = kv("replayed", "global");
  const id = execution("replay");
  await handle.begin({
    id,
    flow: "flow.probe",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });

  // Generation one: a read and a write, both recorded.
  journal.openSession(id, handle, false);
  const live = ctx(id);
  await set(store, "k", { generation: "first" }, live, "seed/0");
  const recorded = ctx(id, "", journal.recorderFor(id, "flow.probe/0#read"));
  const firstRead = await get(store, "k", recorded);
  const writing = ctx(id, "", journal.recorderFor(id, "flow.probe/0#write"));
  await set(store, "k", { generation: "second" }, writing, `${id}/write/0`);
  journal.closeSession(id);

  // The world moves under it, out of band and with nothing recording.
  await set(store, "k", { generation: "moved" }, live, "seed/1");

  // Generation two, resuming: the same sites in the same order.
  journal.openSession(id, handle, true);
  const replayRead = await get(store, "k", ctx(id, "", journal.recorderFor(id, "flow.probe/0#read")));
  // The **same** request, because a replayed generation runs the composition it
  // recorded: a different one is the divergence of resolved q29 rather than
  // this case. What is under test is that the write is not applied a second
  // time — the store still holds what moved under it out of band.
  await set(
    store,
    "k",
    { generation: "second" },
    ctx(id, "", journal.recorderFor(id, "flow.probe/0#write")),
    `${id}/write/0`,
  );
  journal.closeSession(id);

  answer.a_replayed_read_answers_out_of_the_record =
    firstRead.value?.generation === "first" && same(replayRead, firstRead);
  answer.a_replayed_write_is_not_applied_again =
    (await get(store, "k", live)).value?.generation === "moved";
  await journal.releaseJournal();
}

// --- A store an earlier release created --------------------------------------

/**
 * The schema the release before PRD resolved q68 created, on each backend —
 * `entries`/`store_entries` with no `version` column and no sequence table —
 * copied from that release's `src/stores.ts`, `stores-postgres.ts` and
 * `stores-mysql.ts` rather than derived from this one's, because what is under
 * test is that this release opens *that* one.
 */
const EARLIER_SCHEMA = {
  sqlite: [
    `CREATE TABLE entries (
  scope_key TEXT NOT NULL,
  key       TEXT NOT NULL,
  value     TEXT NOT NULL,
  metadata  TEXT,
  embedding TEXT,
  PRIMARY KEY (scope_key, key)
)`,
    `CREATE TABLE applied (
  idempotency_key TEXT PRIMARY KEY,
  op              TEXT NOT NULL,
  result          TEXT NOT NULL
)`,
  ],
  postgres: [
    `CREATE TABLE store_entries (
  store     TEXT COLLATE "C" NOT NULL,
  scope_key TEXT COLLATE "C" NOT NULL,
  "key"     TEXT COLLATE "C" NOT NULL,
  value     TEXT NOT NULL,
  PRIMARY KEY (store, scope_key, "key")
)`,
    `CREATE TABLE store_applied (
  store           TEXT COLLATE "C" NOT NULL,
  scope_key       TEXT COLLATE "C" NOT NULL,
  idempotency_key TEXT COLLATE "C" NOT NULL,
  op              TEXT NOT NULL,
  result          TEXT NOT NULL,
  PRIMARY KEY (store, idempotency_key)
)`,
    "CREATE INDEX store_applied_partition ON store_applied (store, scope_key)",
  ],
  mysql: [
    `CREATE TABLE store_entries (
  store      VARCHAR(128)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key  VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  \`key\`      VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_bin NOT NULL,
  value      LONGTEXT NOT NULL,
  scope_hash BINARY(32) AS (UNHEX(SHA2(scope_key, 256))) STORED NOT NULL,
  key_hash   BINARY(32) AS (UNHEX(SHA2(\`key\`, 256))) STORED NOT NULL,
  PRIMARY KEY (store, scope_hash, key_hash),
  KEY store_entries_partition (store, scope_key(255), \`key\`(191))
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
    `CREATE TABLE store_applied (
  store           VARCHAR(128)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key       VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  idempotency_key VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  op              VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  result          LONGTEXT NOT NULL,
  PRIMARY KEY (store, idempotency_key),
  KEY store_applied_partition (store, scope_key(255))
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  ],
};

{
  // **A deployment upgrading in place.** PRD resolved q68 gave every `kv` key a
  // `version`, so a store file or a database the release before it created
  // has an entries table without the column — and every op of this release
  // reads it. Each arm brings that table forward at its open (SQLite's
  // `migrateStoreFile`, the Postgres arm's probed `ALTER`, the MySQL arm's
  // `MYSQL_STORE_VERSION_COLUMN` with its duplicate-column race), and its rows
  // take version `0`, which no write of this release hands out. Left
  // unchecked, a migration that did nothing would pass every case above —
  // they all run against a schema this release created — and fail every op of
  // every deployment that upgraded, with `no such column: version`.
  //
  // So the earlier schema is laid down by hand, holding one row and the ledger
  // row its write left — on SQLite the store's own file, on a server a
  // database of this run's own, since the tables every case above wrote to are
  // this release's already — and this release then opens it from four
  // connections at once, which on MySQL is four openers racing the probe and
  // the `ALTER`.
  const LEGACY_URL_ENVS = [
    "AGENT_COMPOSE_STORE_URL_LEGACY",
    "AGENT_COMPOSE_STORE_URL_LEGACY_B",
    "AGENT_COMPOSE_STORE_URL_LEGACY_C",
    "AGENT_COMPOSE_STORE_URL_LEGACY_D",
  ];
  const openers = LEGACY_URL_ENVS.map((variable) => kv("legacy", "global", variable));
  const name = openers[0].name;
  const LEDGER_KEY = `${execution("legacy")}/write/0`;
  const row = JSON.stringify({ text: "old" });
  // What the earlier release's ledger held for a `set`: its whole answer then,
  // which carried neither `applied` nor `version`.
  const recorded = JSON.stringify({ key: "old" });
  const database = `${RUN}_legacy`;
  let created = false;
  try {
    if (provider === "sqlite") {
      const module = await import("node-sqlite3-wasm");
      const { Database } = module.default ?? module;
      const directory = path.join(data, "stores");
      fs.mkdirSync(directory, { recursive: true });
      const file = new Database(path.join(directory, `${stores.encodeKey(name)}.sqlite`));
      try {
        for (const statement of EARLIER_SCHEMA.sqlite) file.exec(statement);
        file.run("INSERT INTO entries (scope_key, key, value) VALUES (?, ?, ?)", ["global", "old", row]);
        file.run("INSERT INTO applied (idempotency_key, op, result) VALUES (?, ?, ?)", [
          LEDGER_KEY,
          "set",
          recorded,
        ]);
      } finally {
        file.close();
      }
    } else {
      const admin = await openServer(process.env[URL_ENV]);
      try {
        await rows(admin, `CREATE DATABASE ${database}`);
        created = true;
      } finally {
        await admin.end().catch(() => undefined);
      }
      const address = new URL(process.env[URL_ENV]);
      address.pathname = `/${database}`;
      for (const variable of LEGACY_URL_ENVS) process.env[variable] = address.href;
      const seeder = await openServer(address.href);
      try {
        for (const statement of EARLIER_SCHEMA[provider]) await rows(seeder, statement);
        const [entry, ledger] =
          provider === "postgres"
            ? [
                'INSERT INTO store_entries (store, scope_key, "key", value) VALUES ($1, $2, $3, $4)',
                "INSERT INTO store_applied (store, scope_key, idempotency_key, op, result) VALUES ($1, $2, $3, $4, $5)",
              ]
            : [
                "INSERT INTO store_entries (store, scope_key, `key`, value) VALUES (?, ?, ?, ?)",
                "INSERT INTO store_applied (store, scope_key, idempotency_key, op, result) VALUES (?, ?, ?, ?, ?)",
              ];
        await rows(seeder, entry, [name, "global", "old", row]);
        await rows(seeder, ledger, [name, "global", LEDGER_KEY, "set", recorded]);
      } finally {
        await seeder.end().catch(() => undefined);
      }
    }

    const run = ctx(execution("legacy"));
    const opened = await Promise.allSettled(openers.map((store) => get(store, "old", run)));
    answer.an_earlier_releases_store_takes_every_opener_at_once = opened.every(
      (outcome) => outcome.status === "fulfilled",
    );
    answer.legacy_opening_errors = opened
      .filter((outcome) => outcome.status === "rejected")
      .map((outcome) => String(outcome.reason?.message ?? outcome.reason));
    const [store, other] = openers;
    const held = await get(store, "old", run);
    answer.a_key_an_earlier_release_wrote_answers_version_zero =
      held.found === true && same(held.value, { text: "old" }) && held.version === 0;
    // The earlier release's ledger row still answers its write's redelivery,
    // as a write that applied — every write that release made was
    // unconditional and landed — and the stored value does not move.
    const redelivered = await set(store, "old", { text: "REWRITTEN" }, run, LEDGER_KEY);
    answer.a_write_an_earlier_release_recorded_still_dedupes =
      redelivered.applied === true &&
      redelivered.deduped === true &&
      (await get(other, "old", run)).value?.text === "old";
    // `if_version: 0` names exactly a key nothing has written since the
    // upgrade, and the partition's sequence starts past what it holds.
    const compared = await write(store, "set", { key: "old", value: { text: "new" }, ifVersion: 0 }, run, "lg/1");
    const next = await set(store, "fresh", { text: "fresh" }, run, "lg/2");
    answer.if_version_zero_names_a_key_an_earlier_release_wrote =
      compared.applied === true && compared.version === 1;
    answer.an_upgraded_partitions_sequence_starts_past_what_it_holds =
      next.applied === true &&
      next.version === 2 &&
      (await get(other, "old", run)).version === 1 &&
      (await get(other, "fresh", run)).version === 2;
    answer.legacy_trace = {
      held: held.version ?? null,
      compared: compared.version ?? null,
      next: next.version ?? null,
    };
  } catch (error) {
    answer.legacy_error = String(error?.message ?? error);
  } finally {
    // Every connection this run holds is closed before the database goes —
    // which the case above already does mid-run, so the cases below redial —
    // and a dialled run's database is its own, so it is dropped rather than
    // left on a server a developer keeps.
    await stores.releaseStores();
    if (created) {
      const admin = await openServer(process.env[URL_ENV]);
      await rows(
        admin,
        provider === "postgres"
          ? `DROP DATABASE IF EXISTS ${database} WITH (FORCE)`
          : `DROP DATABASE IF EXISTS ${database}`,
      ).catch(() => undefined);
      await admin.end().catch(() => undefined);
    }
  }
}

// --- A connection the server took away, and the redial that follows ----------

if (provider !== "sqlite") {
  // **The failure a `serve` has and a `run` does not.** A dialled store's
  // connection outlives every execution that uses it, so a managed failover, a
  // proxy's idle reaper or an operator's `KILL` ends it *under* whichever op was
  // holding it — and a store that treated that as permanent would refuse every
  // later op of every later execution until the process restarted, which on a
  // `serve` is for ever. What the arms promise instead is in the sentence
  // `storeConnectionLost` raises: this op fails, the node's own `retry:` decides
  // what happens next, and the retry "runs over a connection dialled again
  // rather than over this one".
  //
  // **The kill lands while a statement is in flight**, and that is the whole
  // design of this case rather than an incidental detail. Killing an *idle*
  // connection is the easy half: both drivers emit `error` on the connection,
  // the arms' listeners hear it, and the entry is dropped. With a statement in
  // flight `mysql2` does the opposite — `_notifyError` hands the error to that
  // statement and emits nothing — so a case that killed an idle socket would
  // pass over a store that is permanently unreachable the moment a real server
  // fails over. The statement is put in flight deterministically: a second
  // connection this file opens with the pinned driver holds an uncommitted row
  // lock on the key the store is about to write, so the store's `set` is
  // provably waiting on the server when the kill arrives.
  const store = kv("redial", "global", KILLED_URL_ENV);
  const run = ctx(execution("redial"));
  const HELD = "held-by-the-killer";
  const killer = await openKiller();
  let blocked;
  try {
    // First, so that the connection — and the DDL its open runs — is there
    // before the killer takes a lock on the table that DDL would need.
    await set(store, "before", { held: "first" }, run, "rd/1");

    await begin(killer);
    await holdTheKey(killer, store.name, HELD);
    // Not awaited, and carrying no idempotency key so that the statement in
    // flight is the write itself: it is now waiting on the killer's uncommitted
    // row, which is what makes the connection's death arrive mid-statement.
    //
    // Its outcome is captured **the moment the promise exists** rather than
    // awaited where it is wanted: a rejection nothing is listening to yet ends
    // the process on this runtime's floor, and what this one rejects with is the
    // whole point of the case.
    const writing = set(store, HELD, { held: "never" }, run);
    blocked = writing.then(
      () => undefined,
      (error) => String(error?.message ?? error),
    );
    const victim = await waitingOn(killer, store.name);
    answer.the_store_was_caught_mid_statement = victim !== undefined;
    answer.redial_victim = victim ?? null;
    // Read only once the kill is in: a write still waiting on a lock nothing has
    // released yet would hang this runner rather than fail it. Where the victim
    // never turned up, the `finally` releases the lock instead and the case
    // above is what says so.
    if (victim !== undefined) {
      await kill(killer, victim);
      const refusal = (await blocked) ?? "";
      answer.redial_refusal = refusal;
      // The op that met the kill is refused by name rather than with whatever
      // the driver said, which is what names the variable an operator can
      // change — and on MySQL it is the only sign the loss was noticed at all.
      answer.a_lost_connection_is_refused_by_name = refusal.includes(
        `lost its connection to the \`${provider}\` store backend`,
      );
    }
  } catch (error) {
    // A store that never came back throws here rather than answering, the two
    // cases below stay absent, and the Rust side prints this beside them.
    answer.redial_error = String(error?.message ?? error);
  } finally {
    // Before anything dials again: the lock is what the next write would queue
    // behind, and an open transaction holds a metadata lock a reopen's DDL wants.
    await rollback(killer).catch(() => undefined);
    if (blocked !== undefined) await blocked;
    await killer.end().catch(() => undefined);
  }
  try {
    await set(store, "after", { held: "second" }, run, "rd/3");
    answer.a_lost_connection_is_redialled =
      (await get(store, "after", run)).value?.held === "second";
    // …and what the lost connection had already committed is still there, which
    // is what makes the redial a reconnection rather than a new store.
    answer.a_redialled_store_still_holds_what_it_wrote =
      (await get(store, "before", run)).value?.held === "first";
  } catch (error) {
    answer.redial_error = String(error?.message ?? error);
  }
}

// **Every other row this run wrote**, on a server, for [`forget`]'s reason at
// a smaller scale: a few hundred rows a run, one of them past 64 KiB, that no
// later run reads and nothing else deletes. Every store this file names starts
// with [`RUN`], which is hex after a letter and so holds no `LIKE` wildcard.
// The SQLite arm's files are in the data directory its caller throws away.
if (provider !== "sqlite") {
  try {
    answer.the_run_gives_its_rows_back = await sweep("LIKE", `${RUN}%`);
  } catch (error) {
    answer.run_cleanup_error = String(error?.message ?? error);
  }
}

await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
