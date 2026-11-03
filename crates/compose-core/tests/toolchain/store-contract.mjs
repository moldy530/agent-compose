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
 * The session id of the store's connection, once the server can see it waiting.
 *
 * Polled rather than assumed, because "the statement is in flight" is the whole
 * premise of the case that calls this: a kill that landed on an idle connection
 * would exercise the driver's `error` event instead, which is the half that was
 * never in doubt. `undefined` when it never turns up, which the caller reports
 * rather than hiding.
 *
 * MySQL is asked by the statement text — `mysql2` interpolates parameters on the
 * client, so the blocked `INSERT` carries this run's unique store name — and
 * Postgres by state, since `pg` binds parameters on the server and its
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
 */
async function waitingOn(killer, store) {
  const found = async () => {
    if (provider === "postgres") {
      await rows(killer, "SELECT pg_stat_clear_snapshot()");
      return await rows(
        killer,
        "SELECT pid AS id FROM pg_stat_activity WHERE datname = current_database() " +
          "AND pid <> pg_backend_pid() AND backend_type = 'client backend' " +
          "AND state = 'active' AND wait_event_type = 'Lock' AND query LIKE '%store_entries%'",
      );
    }
    return await rows(
      killer,
      "SELECT ID AS id FROM information_schema.processlist WHERE DB = DATABASE() " +
        "AND ID <> CONNECTION_ID() AND INFO LIKE ?",
      [`%${store}%`],
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
  answer.expiry_trace = { before, after, listedBefore, listedAfter };
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
  // A clock offset nobody can read fails the op by name rather than being
  // read as no offset — a test that believed it moved the clock and did not
  // would pass for the wrong reason.
  const store = kv("bad_clock", "global");
  const run = ctx(execution("bad_clock"));
  const refused = await later("an hour", () => refusal(get(store, "k", run)));
  answer.a_malformed_clock_offset_is_refused_by_name = (refused ?? "").includes(CLOCK);
}

if (provider !== "sqlite") {
  // **A dialled store's expiry is the server's clock's**, so two writers whose
  // own clocks disagree still agree about which keys are alive. This process's
  // clock is moved ten days ahead for the write; what the row holds is the
  // server's instant plus the `ttl`, and the key is alive to a reader on the
  // server's clock.
  const store = kv("server_clock", "global");
  const run = ctx(execution("server_clock"));
  const realNow = Date.now;
  Date.now = () => realNow() + 10 * 24 * HOUR;
  let written;
  try {
    written = await write(store, "set", { key: "k", value: { n: 1 }, ttl: HOUR }, run);
  } finally {
    Date.now = realNow;
  }
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
  answer.server_clock_trace = { ahead };
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

await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
