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
// Environment: AGENT_COMPOSE_STORE_URL and AGENT_COMPOSE_STORE_URL_B on a
// dialled provider — two names for one server, which is how the interleaved
// writers below become two connections.
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

/** The variable each of the two connections reads its address from. */
const URL_ENV = "AGENT_COMPOSE_STORE_URL";
const OTHER_URL_ENV = "AGENT_COMPOSE_STORE_URL_B";

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

const answer = {};

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

await stores.releaseStores();
process.stdout.write(JSON.stringify(answer));
