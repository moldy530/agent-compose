// Drives `docs/durability.md`'s contract against one emitted project's journal,
// whichever backend that project's target bound (grammar §14.7, PRD resolved
// q62).
//
// The point of running this rather than reading it: the statements in
// `src/journal.ts` are **one** implementation shared by all three backends, so
// what a second or third backend can break is not the logic but the server's
// answer to it — a collation that folds case where a journal key must not, a
// column too small for a harness payload, an `AUTO_INCREMENT` that needs a key
// of its own, an advisory lock that is or is not dropped when a connection ends.
// None of that is visible to `tsc`, and none of it is visible to a fake. So the
// same cases run against each backend a deployment can really bind, and the Rust
// side compares the answers.
//
// Usage: node journal-contract.mjs <generated project directory>
// Prints one JSON object: { provider, cases: { <name>: <result> } }.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";
import { spawnSync } from "node:child_process";
import { writeFileSync, rmSync } from "node:fs";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node journal-contract.mjs <generated project directory>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const journal = await import(at("src/journal.ts"));

/** A payload with the two shapes a `TEXT` column and a folding collation break. */
const UNICODE = "héllo → 世界 \u0000-free 🙂 \\\" ' ; = -- /* not sql */";
const LARGE = "x".repeat(300_000);

/**
 * …and the same shapes for the columns a caller's string reaches **raw**.
 *
 * Every use of `UNICODE` above is serialized on the way in — `canonical` and
 * `JSON.stringify` write a NUL as the six characters of a `\u0000` escape — while
 * a `detail` is the caller's own string put straight into a column. SQLite's
 * driver hands that to the C API as a NUL-terminated string, so a NUL truncates
 * there and on neither server; a case carrying one would be asserting that
 * divergence rather than this document's contract, and what a `detail` actually
 * holds — an HTTP status, a transport error, a reason this codebase wrote — has
 * no NUL in it. Everything else stays, because every other shape here is one a
 * `detail` really does carry: the non-ASCII, the quote, the backslash, the
 * comment openers and the bare `--`.
 */
const UNSERIALIZED = UNICODE.replaceAll("\u0000", "");

/**
 * Two sites distinct only by case, which a case-insensitive key column merges.
 *
 * A site of their own rather than a second reading of `review/0`: the append
 * below is idempotent on `(execution, key)`, so a key this journal already holds
 * would answer the first record whatever case the second was written in — and
 * the case would pass for the wrong reason.
 */
const CASED = ["cased/0", "CASED/0"];

/**
 * The name the MySQL guard is taken under.
 *
 * A literal here and a literal in `src/journal.ts`, held to each other by
 * `the_runner_and_the_module_take_one_writer_guard` in
 * `journal_backend_conformance.rs` — this runner has to name the lock to find
 * the session holding it, and a copy nothing compares is a copy that drifts.
 */
const WRITER_GUARD = "agent-compose:journal";

/**
 * …and how the MySQL arm qualifies it with the schema it is connected to.
 *
 * The same expression `MYSQL_GUARD_NAME` spells in `js/journal-mysql.ts`, for
 * the same reason the name above is duplicated and by the same drift test:
 * MySQL's user-level locks are keyed on the name alone across the whole server,
 * so the arm takes its guard under `<name>:<digest of DATABASE()>` and a reader
 * looking for the holder has to ask for exactly that.
 */
const MYSQL_GUARD_NAME = "CONCAT(?, ':', LEFT(SHA2(DATABASE(), 256), 32))";

const results = {};
let handle;

/**
 * Open the journal, waiting out a guard the last connection has not given back.
 *
 * `close()` answers when **this** side's socket is closed; the server drops the
 * session lock when the backend that held it exits, which is a moment later. So
 * an open that arrives inside that moment can meet its own previous session's
 * guard — a race in the *test*, not in the journal, and waiting is the whole of
 * the answer. Anything that is not the guard is re-thrown at once, so a schema
 * that stopped being idempotent still fails on the first attempt.
 */
async function opened(module, deadline = 4_000) {
  const until = Date.now() + deadline;
  for (;;) {
    try {
      return await module.openJournal();
    } catch (error) {
      const said = error instanceof Error ? error.message : String(error);
      if (!said.includes("another process is already writing") || Date.now() > until) throw error;
      await new Promise((resume) => setTimeout(resume, 50));
    }
  }
}

function record(execution, key, site, value, ordinal = 0) {
  return {
    execution,
    key,
    site,
    kind: "model",
    ordinal,
    request: journal.canonical({ model: "model.smart", turns: [] }),
    outcome: { kind: "value", value },
    refused: false,
    recordedAt: new Date().toISOString(),
  };
}

try {
  // 1. **Open, create, and open again.** The schema is created on first open
  //    with `IF NOT EXISTS`, so a second open of a journal this build already
  //    made runs every statement of that DDL against tables that exist and does
  //    nothing (`docs/durability.md` §10, §11.2).
  //
  //    `releaseJournal()` between the two is what makes this a case at all.
  //    `openJournal` memoizes its promise, so two calls without it are **one**
  //    open that never reaches the server a second time — an assertion that
  //    would hold with `IF NOT EXISTS` deleted from every statement in the
  //    schema. The memoization is worth its own case, and gets one: a second
  //    handle to a remote journal would meet its own writer guard and refuse.
  handle = await journal.openJournal();
  const memoized = await journal.openJournal();
  results.open_memoizes_one_handle = memoized === handle;
  await journal.releaseJournal();
  handle = await opened(journal).catch((error) => {
    throw new Error(
      `open_is_idempotent: a second open of a journal this build already created failed, so \
its schema creation is not idempotent: ${error instanceof Error ? error.message : String(error)}`,
    );
  });
  results.open_is_idempotent = handle !== memoized;
  results.journal_version = journal.JOURNAL_VERSION;
  results.provider = journal.journalBinding.provider;

  const execution = `exec_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  await handle.begin({
    id: execution,
    flow: "flow.review",
    trigger: "manual",
    inputs: { goal: UNICODE },
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  // …and `begin` is idempotent on the id, which is what a recovered execution
  // re-entering `openExecution` relies on.
  await handle.begin({
    id: execution,
    flow: "flow.other",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  const lifecycle = await handle.execution(execution);
  results.begin_is_idempotent = lifecycle?.flow === "flow.review";
  results.lifecycle_round_trips_unicode = lifecycle?.inputs?.goal === UNICODE;

  // 2. **Per-record insert, then read back** — including the two payload shapes
  //    a server can quietly truncate or re-encode (§3, §11.1).
  await handle.append(record(execution, "review/0#model/0", "review/0", { text: UNICODE }));
  await handle.append(record(execution, "review/0#model/1", "review/0", { blob: LARGE }, 1));
  const small = await handle.lookup(execution, "review/0#model/0");
  const large = await handle.lookup(execution, "review/0#model/1");
  results.payload_round_trips_unicode = small?.outcome?.value?.text === UNICODE;
  results.payload_round_trips_a_large_blob = large?.outcome?.value?.blob?.length === LARGE.length;
  results.a_record_reads_back_its_request = small?.request === journal.canonical({
    model: "model.smart",
    turns: [],
  });

  // …and an append is idempotent on `(execution, key)`, which is what keeps a
  // redispatched node's streamed history from writing a row twice (§3.3).
  await handle.append(record(execution, "review/0#model/0", "review/0", { text: "different" }));
  results.append_is_idempotent =
    (await handle.lookup(execution, "review/0#model/0"))?.outcome?.value?.text === UNICODE;

  // 3. **Keys are compared byte-wise.** Two keys differing only in case are two
  //    effects, and a server whose default collation folds them would merge one
  //    execution's record into another effect's slot (§4, §10).
  for (const [index, site] of CASED.entries()) {
    await handle.append(record(execution, `${site}#model/0`, site, { which: index }));
  }
  const cased = await Promise.all(
    CASED.map((site) => handle.lookup(execution, `${site}#model/0`)),
  );
  results.keys_are_case_sensitive =
    cased[0]?.outcome?.value?.which === 0 && cased[1]?.outcome?.value?.which === 1;

  // 4. **The frontier after N records**, which is what a replay reads: a key the
  //    journal holds answers, and the first it does not is where the run goes
  //    live again (§5).
  results.frontier_is_the_first_key_not_held =
    (await handle.lookup(execution, "review/0#model/2")) === undefined;
  const under = await handle.effectsUnder(execution, "review/0");
  results.effects_under_a_site = under.map((held) => held.key);

  // 5. **The recovery scan.** Every execution the journal holds open, which is
  //    what a `serve` start enumerates before it accepts a connection (§6.1).
  const open = await handle.openExecutions();
  results.recovery_scan_finds_the_open_execution = open.some((held) => held.id === execution);
  await handle.end(execution, "completed");
  const closed = await handle.openExecutions();
  results.recovery_scan_drops_a_closed_execution = !closed.some((held) => held.id === execution);
  results.a_closed_row_keeps_its_outcome = (await handle.execution(execution))?.status ===
    "completed";

  // …and the columns that are **not** payloads take what a real failure is.
  //  `executions.error` holds a provider's whole failure body or a harness run's
  //  quoted transcript; MySQL's `TEXT` is 64 KiB and the `STRICT_TRANS_TABLES`
  //  that arm sets on its session makes an over-long value an error rather than
  //  a truncation, so a `failed` outcome this size is a write SQLite and Postgres
  //  take and one backend could refuse — leaving the lifecycle row `open` for
  //  ever, so every later `serve` start re-recovers an execution that has
  //  already finished (§3.6, §10). The blob case above drives the one column
  //  that was always `LONGTEXT`, which is not this one.
  await handle.end(execution, "failed", LARGE);
  const ended = await handle.execution(execution);
  results.a_large_error_round_trips = ended?.error === LARGE && ended?.status === "failed";

  // 6. **A refusal is marked on the record**, which is resolved q29's second
  //    divergence told apart from an ordinary mismatch (§7).
  await handle.refuse(execution, "review/0#model/0");
  results.a_refusal_is_recorded =
    (await handle.lookup(execution, "review/0#model/0"))?.refused === true;

  // 7. **The delivery ledger** allocates its ordinals durably and keeps one
  //    outcome per row (§3.7).
  const first = await handle.intendDelivery({
    execution,
    kind: "callback",
    event: "parked",
    url: "https://receiver.example/hook",
    body: JSON.stringify({ hello: UNICODE }),
    pauses: ["review/0"],
  });
  const second = await handle.intendDelivery({
    execution,
    kind: "trace_sink",
    event: "settled",
    url: "https://collector.example/v1/traces",
    body: "{}",
    pauses: [],
  });
  results.delivery_ordinals_increase = second.ordinal === first.ordinal + 1;
  await handle.recordAttempt(execution, first.ordinal, { at: new Date().toISOString(), outcome: "delivered" }, "delivered");
  await handle.recordAttempt(execution, first.ordinal, { at: new Date().toISOString(), outcome: "failed" }, "pending");
  const ledger = await handle.deliveries(execution);
  results.a_settled_delivery_is_not_reopened =
    ledger[0]?.status === "delivered" && ledger[0]?.attempts?.length === 1;
  results.a_pending_delivery_is_owed = (await handle.undelivered()).some(
    (held) => held.execution === execution && held.ordinal === second.ordinal,
  );
  // …and the ledger's own unbounded column, for the reason the lifecycle row's
  // `error` is checked above: a `detail` is why an attempt failed, which is
  // whatever the far end said.
  await handle.recordAttempt(
    execution,
    second.ordinal,
    { at: new Date().toISOString(), outcome: "failed", detail: LARGE },
    "pending",
  );
  results.a_large_delivery_detail_round_trips =
    (await handle.deliveries(execution)).find((held) => held.ordinal === second.ordinal)
      ?.detail === LARGE;

  // …and the two endings §3.7 gives a delivery beside `delivered`, through the
  // three verbs that reach them — none of which the cases above drive at all.
  //
  //  * `refuseDelivery` opens a row that is already over — a callback URL no
  //    `callback_allow:` entry admitted, recorded rather than raised (resolved
  //    q33, grammar §13.3);
  //  * `refuseRecorded` is the same ending for a row already on the ledger, and
  //    it is the **only** statement anywhere in `SqlJournal` that compares
  //    `deliveries.kind` — `AND (kind IS NULL OR kind = 'callback')`. Nothing
  //    else drives that column, and `docs/durability.md` §10 says the
  //    per-backend collation binding "covers every column a statement compares,
  //    not only the keys". A `trace_sink` address is admitted by no list, so
  //    there is nothing for it to fail to match and its row must stay owed
  //    (grammar §14.5, PRD resolved q50);
  //  * `exhaustRecorded` is the end where the bounded schedule ran out.
  //
  // Both `UPDATE`s also carry `status = 'pending'`, which is what stops a late
  // refusal or a spent schedule reopening a row that already has an outcome —
  // so each is driven twice, once where it must take and once where it must not.
  const BLOCKED = `no \`callback_allow:\` entry admits it: ${UNSERIALIZED}`;
  const refused = await handle.refuseDelivery(
    {
      execution,
      kind: "callback",
      event: "settled",
      url: "https://blocked.example/hook",
      body: JSON.stringify({ blocked: UNICODE }),
      pauses: [],
    },
    BLOCKED,
  );
  const opener = (await handle.deliveries(execution)).find(
    (held) => held.ordinal === refused.ordinal,
  );
  results.a_delivery_refused_at_intent_is_opened_settled =
    refused.status === "refused" &&
    refused.ordinal === second.ordinal + 1 &&
    opener?.status === "refused" &&
    opener?.kind === "callback" &&
    opener?.detail === BLOCKED &&
    opener?.settledAt !== undefined;

  const owed = await handle.intendDelivery({
    execution,
    kind: "callback",
    event: "settled",
    url: "https://receiver.example/late",
    body: "{}",
    pauses: [],
  });
  const REFUSAL = `the allowlist stopped admitting it: ${UNSERIALIZED}`;
  // …the callback row, which the predicate must take…
  await handle.refuseRecorded(execution, owed.ordinal, REFUSAL);
  // …the `trace_sink` row, which it must not, because `kind` is compared…
  await handle.refuseRecorded(execution, second.ordinal, REFUSAL);
  const ledgerAfterRefusal = await handle.deliveries(execution);
  const refusedLate = ledgerAfterRefusal.find((held) => held.ordinal === owed.ordinal);
  results.a_recorded_callback_is_refused_by_ordinal =
    refusedLate?.status === "refused" &&
    refusedLate?.detail === REFUSAL &&
    refusedLate?.settledAt !== undefined;
  results.a_trace_sink_row_is_not_refused =
    ledgerAfterRefusal.find((held) => held.ordinal === second.ordinal)?.status === "pending";

  const SPENT = `every attempt in the schedule failed: ${UNSERIALIZED}`;
  // …the row still owed, which the predicate must take…
  await handle.exhaustRecorded(execution, second.ordinal, SPENT);
  // …and the one that was refused a moment ago, which it must not.
  await handle.exhaustRecorded(execution, owed.ordinal, SPENT);
  const ledgerAfterExhaustion = await handle.deliveries(execution);
  const spent = ledgerAfterExhaustion.find((held) => held.ordinal === second.ordinal);
  results.a_spent_schedule_exhausts_its_delivery =
    spent?.status === "exhausted" && spent?.detail === SPENT && spent?.settledAt !== undefined;
  results.a_settled_delivery_is_not_exhausted =
    ledgerAfterExhaustion.find((held) => held.ordinal === owed.ordinal)?.status === "refused";

  // 8. **The dispatch board**, whose park order breaks ties by insertion rather
  //    than by the millisecond two rows share (`docs/distributed.md` §6.2).
  //
  //    The handles are minted under this run's execution, the way a hub mints
  //    them — the schema calls a dispatch id "a random handle the issuing hub
  //    owns" (§10.1) and every verb that takes one keys on `dispatches.id`
  //    **journal-globally**: `dispatchOf` selects on it alone, `claimDispatch`
  //    and `settleDispatch` update on it alone. A fixed `dsp_1` would therefore
  //    be the *same row set* on every run against a server that keeps its
  //    journal — and the second run's `settleDispatch` would read the first
  //    run's already-settled row and answer `false`, reporting
  //    `a_settle_answers_once` as a backend failure that is this runner's own
  //    bookkeeping. The executions are unique per run for the same reason; the
  //    dispatch ids have to be too.
  const dispatch = (name) => `dsp_${name}_${execution.slice("exec_".length)}`;
  const parkedAt = new Date().toISOString();
  for (const index of [0, 1, 2, 10, 11]) {
    await handle.park({
      execution,
      wait: `map/0/${index}`,
      id: dispatch(index),
      placement: "gpu",
      node: "flow.review.embed",
      site: `map/0/${index}`,
      inputs: { item: index },
      status: "parked",
      parkedAt,
    });
  }
  const board = await handle.unsettledDispatches();
  results.park_order_is_insertion_order = board
    .filter((row) => row.execution === execution)
    .map((row) => row.wait);
  // …and parking is idempotent on `(execution, wait)`, which is what makes a
  // resumed hub re-attach rather than dispatch the work a second time (§6.1).
  const reparked = await handle.park({
    execution,
    wait: "map/0/0",
    id: dispatch("other"),
    placement: "gpu",
    node: "flow.review.embed",
    site: "map/0/0",
    inputs: { item: 99 },
    status: "parked",
    parkedAt: new Date().toISOString(),
  });
  results.park_is_idempotent = reparked.id === dispatch(0);
  // …and the two readers a recovering hub re-attaches through: the row at one
  // wait, which is how a resumed execution finds the dispatch its predecessor
  // left rather than opening a second one for work a worker may already be
  // doing, and every row of one execution, which is what a takeover enumerates
  // (§3.8, §6.1). `dispatchesOf` is ordered like the board — `parked_at` first,
  // insertion order to break the tie a fan-out's five rows share — and scoped to
  // the execution asked for, which on a journal that has held other runs is not
  // the same as "every row".
  results.a_dispatch_is_found_at_its_wait =
    (await handle.dispatchAt(execution, "map/0/0"))?.id === dispatch(0) &&
    (await handle.dispatchAt(execution, "map/0/99")) === undefined;
  const mine = await handle.dispatchesOf(execution);
  results.the_dispatches_of_an_execution_are_its_own_in_park_order =
    mine.every((row) => row.execution === execution) &&
    mine.map((row) => row.wait).join(",") === "map/0/0,map/0/1,map/0/2,map/0/10,map/0/11";
  results.a_claim_hands_the_row_over =
    (await handle.claimDispatch(dispatch(1), "session-a"))?.session === "session-a";
  results.a_second_claim_takes_nothing =
    (await handle.claimDispatch(dispatch(1), "session-b")) === undefined;
  // …and a release is the **holder's** alone. This one is a collation case as
  // much as a predicate case: `releaseDispatch` compares the session **in SQL**,
  // so the column's collation is what decides it, and on a case-insensitive one
  // — MySQL's own table default — a worker whose id differed from the holder's
  // only in case would put another worker's in-flight work back on the board
  // (§3.8, §10).
  await handle.claimDispatch(dispatch(2), "session-a");
  results.a_release_by_another_session_takes_nothing =
    (await handle.releaseDispatch(dispatch(2), "SESSION-A")) === false &&
    (await handle.dispatchOf(dispatch(2)))?.status === "dispatched";
  results.a_release_by_the_holder_parks_it_again =
    (await handle.releaseDispatch(dispatch(2), "session-a")) === true &&
    (await handle.dispatchOf(dispatch(2)))?.status === "parked";
  results.a_settle_answers_once =
    (await handle.settleDispatch(dispatch(1), { kind: "value", value: { ok: UNICODE } })) ===
      true &&
    (await handle.settleDispatch(dispatch(1), { kind: "value", value: { ok: "again" } })) === false;
  results.a_settled_dispatch_keeps_its_outcome =
    (await handle.dispatchOf(dispatch(1)))?.outcome?.value?.ok === UNICODE;
  // …and the *other* ending, which is the hub's rather than a result's: a
  // session that fell outside the liveness window, or a dispatch issued under an
  // artifact that has been replaced (§3.8, `docs/distributed.md` §5, §6.3). The
  // statement is `supersedeDispatch`'s, and nothing above reaches it — while a
  // replaced hub runs it over every row it inherited. Three things have to be
  // true of it: the row ends without an outcome, so a late result meets `409`
  // rather than `204`; the session it was claimed under is let go; and a row
  // that already ended keeps the ending it has, which is the
  // `status IN ('parked', 'dispatched')` predicate.
  const ORPHANED = `the hub stopped waiting for this dispatch: ${UNSERIALIZED}`;
  await handle.claimDispatch(dispatch(10), "session-c");
  await handle.supersedeDispatch(dispatch(10), ORPHANED);
  const orphaned = await handle.dispatchOf(dispatch(10));
  results.a_dispatch_is_superseded_without_an_outcome =
    orphaned?.status === "superseded" &&
    orphaned?.detail === ORPHANED &&
    orphaned?.settledAt !== undefined &&
    orphaned?.outcome === undefined &&
    orphaned?.session === undefined;
  await handle.supersedeDispatch(dispatch(1), ORPHANED);
  const kept = await handle.dispatchOf(dispatch(1));
  results.a_settled_dispatch_is_not_superseded =
    kept?.status === "settled" && kept?.outcome?.value?.ok === UNICODE;
} finally {
  // Always, and before the writer guard's own case below: a connection left open
  // is a lock the next opener meets.
  await journal.releaseJournal().catch(() => undefined);
}

/**
 * End the session holding the writer guard, from outside, without closing it.
 *
 * The case below needs the state `docs/durability.md` §2.3's headline property
 * is about: a hub whose **host** is gone. A process killed on a machine that is
 * still running closes its socket and the server ends the session at once; a
 * host that vanished closes nothing, and the server ends the session only when
 * it notices — which is what each arm's shortened reap window bounds to five
 * minutes. Waiting five minutes is not a test, so the session is ended from
 * another connection instead: the same path the reap takes, at the moment this
 * runner chooses.
 *
 * Answers how many sessions it ended, or a string saying why it could not — a
 * server this runner may not ask to end a session is a case reported as
 * unavailable rather than one asserted away or failed.
 */
async function endTheGuardSession(provider, url) {
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      // The journal's guard is the only advisory lock anything in this project
      // ever takes, so a session holding one here is the journal's writer.
      const holders = await client.query(
        "SELECT pid FROM pg_locks WHERE locktype = 'advisory' AND granted AND pid <> pg_backend_pid()",
      );
      for (const row of holders.rows) {
        await client.query("SELECT pg_terminate_backend($1)", [row.pid]);
      }
      return holders.rows.length;
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    // `IS_USED_LOCK` answers the connection id holding the named lock, or NULL —
    // and the name is the journal's, schema and all. MySQL's user-level locks
    // are server-wide, so the arm qualifies `WRITER_GUARD` with the database it
    // is connected to (`MYSQL_GUARD_NAME` in `js/journal-mysql.ts`); asking for
    // the bare name here would find nothing holding it and report the
    // dead-owner case unavailable against a server that is working perfectly.
    // The two spell the expression identically, held so by
    // `the_runner_and_the_module_take_one_writer_guard`.
    const [rows] = await connection.query(
      `SELECT IS_USED_LOCK(${MYSQL_GUARD_NAME}) AS holder`,
      [WRITER_GUARD],
    );
    const holder = rows[0]?.holder;
    if (holder === null || holder === undefined) return 0;
    // `KILL` takes no placeholder, so the id goes in as the number it is.
    await connection.query(`KILL CONNECTION ${Number(holder)}`);
    return 1;
  } finally {
    await connection.end().catch(() => undefined);
  }
}

/**
 * Open this project's journal from a child process, and say how that went.
 *
 * The one honest way to meet the writer guard: `openJournal()` caches its
 * promise per module instance and this runtime deduplicates module instances
 * more aggressively than a query-stringed re-import assumes, so any in-process
 * "second opener" risks holding the first opener's handle and vouching for a
 * guard it never touched. A child process shares nothing, which is exactly the
 * claim §2.3 makes.
 *
 * Answers `{ outcome: "opened" }` — the child released what it opened before
 * reporting — or `{ outcome: "refused", message }` with the arm's own text.
 */
function secondOpener() {
  const script = path.resolve(project, ".journal-second-opener.mjs");
  writeFileSync(
    script,
    `const journal = await import(${JSON.stringify(at("src/journal.ts"))});\n` +
      `try {\n` +
      `  await journal.openJournal();\n` +
      `  await journal.releaseJournal().catch(() => undefined);\n` +
      `  process.stdout.write(JSON.stringify({ outcome: "opened" }));\n` +
      `} catch (error) {\n` +
      `  process.stdout.write(JSON.stringify({ outcome: "refused", message: error instanceof Error ? error.message : String(error) }));\n` +
      `}\n`,
  );
  try {
    const ran = spawnSync(process.execPath, [script], {
      cwd: project,
      env: process.env,
      encoding: "utf8",
      timeout: 30_000,
    });
    if (ran.status !== 0 || ran.stdout === "") {
      return {
        outcome: "refused",
        message: `the second opener did not report: exit ${ran.status}, stderr: ${ran.stderr}`,
      };
    }
    return JSON.parse(ran.stdout);
  } finally {
    rmSync(script, { force: true });
  }
}

// 9. **The writer guard.** A second opener of a journal a live process holds is
//    refused by name rather than left to interleave (`docs/durability.md` §2).
//    Only the two remote backends have one — SQLite's file lock is broken after
//    a deadline by design, which §2.2 states and §12 repeats — so the case is
//    reported as skipped there rather than asserted away.
if (journal.journalBinding.provider === "sqlite") {
  results.writer_guard_refuses_a_second_opener = "not-applicable";
  results.a_takeover_is_not_locked_out_by_a_dead_session = "not-applicable";
  results.a_lost_connection_is_refused_rather_than_fatal = "not-applicable";
} else {
  const held = await opened(journal);
  // A second **process**, literally: an earlier draft loaded a second module
  // instance under a query string (`src/journal.ts?rival`) and Bun answered
  // that absolute-URL import with the *same* module namespace, so the "rival"
  // resolved the first opener's cached promise, no second connection ever
  // existed, and the case could not fail against any server. A contract case
  // that cannot fail is not a case, and the guard it vouched for is the one
  // §2.3 leans on — so the second opener is now a child process running this
  // project's own `openJournal()`, which is also the situation the guard
  // exists for.
  const verdict = secondOpener();
  results.writer_guard_refuses_a_second_opener =
    verdict.outcome === "opened" ? "not-refused" : verdict.message;

  // 10. **…and a guard nobody is holding is not a lock.** The case above is a
  //     *live* second opener, which is the easy half. The half §2.3 leans on
  //     hardest is the dead one: "a `serve` restarted on a fresh machine
  //     recovers every open execution" is a promise about taking over from a
  //     host that is gone, and a guard that outlived its holder would refuse
  //     exactly that takeover — while the refusal text told the operator to go
  //     and find a live `serve` that does not exist.
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const ended = await endTheGuardSession(journal.journalBinding.provider, url).catch((error) =>
    error instanceof Error ? error.message : String(error),
  );
  if (typeof ended !== "number" || ended === 0) {
    results.a_takeover_is_not_locked_out_by_a_dead_session = `not-available: ${
      ended === 0 ? "nothing was holding the guard" : ended
    }`;
  } else {
    // A real process again (see the rival above), because what a takeover is.
    // The kill and the successor race by nature — the server releases the dead
    // session's guard when it finishes terminating it — so a refusal that reads
    // as the guard still held is retried under the same deadline `opened()`
    // gives a live one.
    const until = Date.now() + 4_000;
    let verdict = secondOpener();
    while (
      verdict.outcome === "refused" &&
      verdict.message.includes("another process is already writing") &&
      Date.now() <= until
    ) {
      verdict = secondOpener();
    }
    results.a_takeover_is_not_locked_out_by_a_dead_session =
      verdict.outcome === "opened" ? "admitted" : verdict.message;
  }

  // 11. **…and this process is still here to report it.** Both drivers emit
  //     `error` on a connection the server ended, and an `EventEmitter` that
  //     emits `error` with nothing listening raises `ERR_UNHANDLED_ERROR` and
  //     takes the process with it — so a run that reaches this line at all is
  //     the listener each arm attaches. What the statement after a lost
  //     connection answers is a refusal, never a row: the guard went with the
  //     connection and another hub may already hold this journal, so nothing is
  //     redialled (§2.3).
  if (typeof ended === "number" && ended > 0) {
    results.a_lost_connection_is_refused_rather_than_fatal = await held
      .openExecutions()
      .then(
        () => "answered",
        (error) => (error instanceof Error ? error.message : String(error)),
      );
  } else {
    results.a_lost_connection_is_refused_rather_than_fatal = "not-available: no session was ended";
  }
  await journal.releaseJournal().catch(() => undefined);
}

process.stdout.write(JSON.stringify(results));
