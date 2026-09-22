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

import { randomUUID } from "node:crypto";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

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

/** One binding, on the backend this run is driving. */
const kv = (name, scope, variable = URL_ENV) => ({
  address: `store.${RUN}_${name}`,
  name: `${RUN}_${name}`,
  kind: "kv",
  scope,
  metadata: false,
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
  const address = process.env[KILLED_URL_ENV];
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
 * `pg_stat_activity.query` holds `$1` where the name would be.
 */
async function waitingOn(killer, store) {
  const found = async () => {
    if (provider === "postgres") {
      return await rows(
        killer,
        "SELECT pid AS id FROM pg_stat_activity WHERE datname = current_database() " +
          "AND pid <> pg_backend_pid() AND state = 'active' AND query LIKE '%store_entries%'",
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
  // carried. It must apply **once** — the answer is the first attempt's and the
  // stored value has not moved.
  const second = await set(store, "k", { attempt: "REWRITTEN" }, run, key);
  answer.a_double_delivered_write_lands_once =
    same(first, second) && (await get(store, "k", run)).value?.attempt === "first";
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
