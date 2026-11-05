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

  // …and a **child execution's lineage** (§3.2, §3.5, PRD resolved q65): the
  //  cause a detached `flow.*` dispatch leaves beside the lifecycle row, in a
  //  table of its own. Read back on both of the reads that return a row — the
  //  one-row read a dispatch asks, and the recovery scan a `serve` start walks —
  //  absent on a row nothing dispatched, and written once: `begin` is idempotent
  //  on the id, lineage included, which is what a re-issued dispatch relies on.
  //  `item_index` is the one integer column the journal holds, so it is the one
  //  a driver could hand back as a string. The admission bound the dispatch was
  //  issued under rides with it, whole, because a recovery with no parent to
  //  re-issue the child admits it under exactly that (Decision D28, §6.1).
  const child = `${execution}-child`;
  const childKey = `${execution}/review/0/3`;
  const admission = { node: `${execution}/review`, nodeBound: 2, route: "1", routeBound: 1 };
  await handle.begin({
    id: child,
    flow: "flow.review",
    trigger: "on_request",
    inputs: { goal: UNICODE },
    sessionKey: "",
    lineage: { parent: execution, idempotencyKey: childKey, itemIndex: 3, admission },
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  await handle.begin({
    id: child,
    flow: "flow.review",
    trigger: "on_request",
    inputs: {},
    sessionKey: "",
    lineage: { parent: "exec_other", idempotencyKey: "exec_other/review/0/0" },
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  const childRow = await handle.execution(child);
  const scannedChild = (await handle.openExecutions()).find((held) => held.id === child);
  const sameLineage = (held) =>
    held?.parent === execution &&
    held?.idempotencyKey === childKey &&
    held?.itemIndex === 3 &&
    JSON.stringify(held?.admission) === JSON.stringify(admission);
  results.a_child_executions_lineage_round_trips =
    sameLineage(childRow?.lineage) && sameLineage(scannedChild?.lineage);
  results.an_execution_nothing_dispatched_has_no_lineage =
    lifecycle !== undefined &&
    lifecycle.lineage === undefined &&
    (await handle.execution(execution))?.lineage === undefined;
  await handle.end(child, "completed");

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

  // …and the **end of an instance** (§3.5, grammar §7.9, PRD resolved q76). The
  //  boundary a `finally:` node's start leaves is an ordinary record of its own
  //  kind, claimed under the node and read back with the end it holds; and `finally_error`
  //  is a column of the lifecycle row beside an outcome it does not change —
  //  `LONGTEXT` on MySQL for `error`'s reason, since what a finally node raised
  //  is as long as what any node raises — and absent on a row that has none,
  //  whichever way that row ended.
  const ending = `${execution}-ending`;
  await handle.begin({
    id: ending,
    flow: "flow.review",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  await handle.append({
    ...record(ending, "close/0#finally/0", "close/0", { outcome: "failed", error: UNICODE }),
    kind: "finally",
    request: journal.canonical({ node: "close" }),
  });
  const boundary = await handle.lookup(ending, "close/0#finally/0");
  results.a_finally_boundary_round_trips =
    boundary?.kind === "finally" &&
    boundary?.request === journal.canonical({ node: "close" }) &&
    boundary?.outcome?.value?.outcome === "failed" &&
    boundary?.outcome?.value?.error === UNICODE;
  await handle.end(ending, "completed", undefined, LARGE);
  const tidied = await handle.execution(ending);
  results.a_finally_error_round_trips_beside_its_outcome =
    tidied?.status === "completed" && tidied?.error === undefined && tidied?.finallyError === LARGE;
  results.a_row_with_no_finally_error_reads_none =
    ended?.finallyError === undefined && (await handle.execution(child))?.finallyError === undefined;

  // …and **a cancel** (§3.5, §6.1, PRD resolved q75). The request is a row of
  //  its own, written before anything is stopped, so a process that dies
  //  between the `202` and the settled row leaves an open row carrying it: the
  //  recovery scan reads it back on the open row, a second request of the same
  //  execution is a no-op that keeps the first one's instant, and the row
  //  settles `cancelled` with no `error` — nothing went wrong with an execution
  //  somebody stopped — and is gone from the scan like any closed row.
  const cancelling = `${execution}-cancelling`;
  await handle.begin({
    id: cancelling,
    flow: "flow.review",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  results.an_execution_nobody_cancelled_carries_no_request =
    (await handle.execution(cancelling))?.cancelRequestedAt === undefined;
  await handle.requestCancel(cancelling);
  const requested = await handle.execution(cancelling);
  const scanned = (await handle.openExecutions()).find((row) => row.id === cancelling);
  results.a_cancel_request_is_read_back_on_the_open_row =
    typeof requested?.cancelRequestedAt === "string" &&
    !Number.isNaN(Date.parse(requested.cancelRequestedAt)) &&
    requested?.status === "open" &&
    scanned?.cancelRequestedAt === requested.cancelRequestedAt;
  await new Promise((resolve) => setTimeout(resolve, 5));
  await handle.requestCancel(cancelling);
  results.a_second_cancel_request_keeps_the_first =
    (await handle.execution(cancelling))?.cancelRequestedAt === requested?.cancelRequestedAt;
  await handle.end(cancelling, "cancelled");
  const cancelledRow = await handle.execution(cancelling);
  results.a_cancelled_row_settles_with_no_error =
    cancelledRow?.status === "cancelled" &&
    cancelledRow?.error === undefined &&
    typeof cancelledRow?.endedAt === "string" &&
    cancelledRow?.cancelRequestedAt === requested?.cancelRequestedAt &&
    !(await handle.openExecutions()).some((row) => row.id === cancelling);

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
  // …and a settled **failure** keeps what a `finally:` node raised beside it on
  // the worker (`docs/distributed.md` §3.4, `docs/durability.md` §3.8): the one
  // field a failure carries beyond its name and message, kept on the row so a
  // hub restarted before the execution settles reads it back. The value is the
  // long one, for `error`'s reason: a finally node's failure is as long as its
  // subprocess made it.
  await handle.claimDispatch(dispatch(11), "session-d");
  await handle.settleDispatch(dispatch(11), {
    kind: "error",
    name: "NodeFailure",
    message: UNICODE,
    finallyError: LARGE,
  });
  const failedHere = (await handle.dispatchOf(dispatch(11)))?.outcome;
  results.a_settled_failure_keeps_its_finally_error =
    failedHere?.kind === "error" &&
    failedHere.name === "NodeFailure" &&
    failedHere.message === UNICODE &&
    failedHere.finallyError === LARGE;
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

/**
 * Take the lifecycle row's `finally_error` column away, from outside the
 * journal, so the file or database is the shape a build before PRD resolved q76
 * wrote (`docs/durability.md` §11.2).
 *
 * The one column a release after the schema's creation added to all three
 * backends, so the one whose probe is driven on each: SQLite's `PRAGMA
 * table_info`, Postgres' `pg_attribute` and MySQL's `information_schema` each
 * have to find it gone and put it back. Answers `true` once it is dropped.
 */
async function dropTheFinallyErrorColumn(provider, url) {
  const statement = "ALTER TABLE executions DROP COLUMN finally_error";
  if (provider === "sqlite") {
    const { Database } = await import("node-sqlite3-wasm");
    const database = new Database(journal.journalPath());
    try {
      database.exec(`${statement};`);
      return !database
        .all("PRAGMA table_info(executions)")
        .some((column) => column.name === "finally_error");
    } finally {
      database.close();
    }
  }
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      await client.query(statement);
      return true;
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    await connection.query(statement);
    return true;
  } finally {
    await connection.end().catch(() => undefined);
  }
}

/**
 * Take the `cancellations` table away, from outside the journal, so the file or
 * database is the shape a build before PRD resolved q75 wrote
 * (`docs/durability.md` §11.2). Answers `true` once it is gone.
 */
async function dropTheCancellationsTable(provider, url) {
  const statement = "DROP TABLE cancellations";
  if (provider === "sqlite") {
    const { Database } = await import("node-sqlite3-wasm");
    const database = new Database(journal.journalPath());
    try {
      database.exec(`${statement};`);
      return !database
        .all("SELECT name FROM sqlite_master WHERE type = 'table'")
        .some((table) => table.name === "cancellations");
    } finally {
      database.close();
    }
  }
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      await client.query(statement);
      return true;
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    await connection.query(statement);
    return true;
  } finally {
    await connection.end().catch(() => undefined);
  }
}

// 8. **A journal written before the end of an instance was recorded opens, and
//    takes one** (§3.5, §11.2, PRD resolved q76). `CREATE TABLE IF NOT EXISTS`
//    leaves an older table exactly as it is, so a column the lifecycle row grew
//    later exists only where a probe on open adds it — and an `UPDATE` naming a
//    column the table lacks would fail every row's close in that journal.
{
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const dropped = await dropTheFinallyErrorColumn(journal.journalBinding.provider, url).catch(
    (error) => (error instanceof Error ? error.message : String(error)),
  );
  try {
    handle = await opened(journal);
    const later = `exec_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}-later`;
    await handle.begin({
      id: later,
      flow: "flow.review",
      trigger: "manual",
      inputs: {},
      sessionKey: "",
      status: "open",
      journalVersion: journal.JOURNAL_VERSION,
      startedAt: new Date().toISOString(),
    });
    await handle.end(later, "failed", "the body failed", UNSERIALIZED);
    const row = await handle.execution(later);
    results.a_journal_without_the_finally_error_column_gains_it_on_open =
      dropped === true &&
      row?.status === "failed" &&
      row?.error === "the body failed" &&
      row?.finallyError === UNSERIALIZED;
  } finally {
    await journal.releaseJournal().catch(() => undefined);
  }
}

// 8b. **A journal written before a cancel was recorded opens, and takes one**
//     (§3.5, §11.2, PRD resolved q75). The request is a table of its own rather
//     than a column, so what an older journal lacks is the whole table — which
//     `CREATE TABLE IF NOT EXISTS` on open adds, with no probe — and a cancel of
//     an execution that journal holds open is then recorded like any other.
{
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const dropped = await dropTheCancellationsTable(journal.journalBinding.provider, url).catch(
    (error) => (error instanceof Error ? error.message : String(error)),
  );
  try {
    handle = await opened(journal);
    const older = `exec_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}-older`;
    await handle.begin({
      id: older,
      flow: "flow.review",
      trigger: "manual",
      inputs: {},
      sessionKey: "",
      status: "open",
      journalVersion: journal.JOURNAL_VERSION,
      startedAt: new Date().toISOString(),
    });
    const before = await handle.execution(older);
    await handle.requestCancel(older);
    const after = await handle.execution(older);
    results.a_journal_without_the_cancellations_table_gains_it_on_open =
      dropped === true &&
      before?.status === "open" &&
      before?.cancelRequestedAt === undefined &&
      typeof after?.cancelRequestedAt === "string";
    await handle.end(older, "cancelled");
  } finally {
    await journal.releaseJournal().catch(() => undefined);
  }
}

/**
 * Take the four intake tables away, so the file or database is the shape a
 * build before PRD resolved q78 wrote (`docs/durability.md` §11.2). Answers
 * `true` once they are gone.
 */
async function dropTheIntakeTables(provider, url) {
  const tables = ["fired", "dedupe", "checkpoints", "restarts"];
  if (provider === "sqlite") {
    const { Database } = await import("node-sqlite3-wasm");
    const database = new Database(journal.journalPath());
    try {
      for (const table of tables) database.exec(`DROP TABLE IF EXISTS ${table};`);
      return !database
        .all("SELECT name FROM sqlite_master WHERE type = 'table'")
        .some((table) => tables.includes(table.name));
    } finally {
      database.close();
    }
  }
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      for (const table of tables) await client.query(`DROP TABLE IF EXISTS ${table}`);
      return true;
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    for (const table of tables) await connection.query(`DROP TABLE IF EXISTS ${table}`);
    return true;
  } finally {
    await connection.end().catch(() => undefined);
  }
}

/**
 * Claim one tick **from outside this process's journal handle** — a second
 * connection writing the `fired` row the way another hub sharing the journal
 * would — so the case below is a race between two writers, not two calls one
 * serialized handle orders. Answers `true` once the row is there.
 */
async function claimTickElsewhere(provider, url, trigger, scheduledAt, execution) {
  const firedAt = new Date().toISOString();
  if (provider === "sqlite") {
    const { Database } = await import("node-sqlite3-wasm");
    const database = new Database(journal.journalPath());
    try {
      database.run(
        "INSERT INTO fired (trigger_name, scheduled_at, execution, fired_at) VALUES (?, ?, ?, ?)",
        [trigger, scheduledAt, execution, firedAt],
      );
      return true;
    } finally {
      database.close();
    }
  }
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      await client.query(
        "INSERT INTO fired (trigger_name, scheduled_at, execution, fired_at) VALUES ($1, $2, $3, $4)",
        [trigger, scheduledAt, execution, firedAt],
      );
      return true;
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    await connection.query(
      "INSERT INTO fired (trigger_name, scheduled_at, execution, fired_at) VALUES (?, ?, ?, ?)",
      [trigger, scheduledAt, execution, firedAt],
    );
    return true;
  } finally {
    await connection.end().catch(() => undefined);
  }
}

// 8c. **A journal written before the intake opens, and takes its four tables**
//     (§3.11-§3.14, §11.2, PRD resolved q78). Each is a table of its own, so
//     `CREATE TABLE IF NOT EXISTS` on open is the whole of the migration — and
//     then every verb of the intake answers on it.
{
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const dropped = await dropTheIntakeTables(journal.journalBinding.provider, url).catch((error) =>
    error instanceof Error ? error.message : String(error),
  );
  try {
    handle = await opened(journal);
    const suffix = `${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
    const fired = await handle.fire({
      trigger: `older_${suffix}`,
      scheduledAt: "2026-11-04T03:00:00.000Z",
      execution: `exec_older_${suffix}`,
      firedAt: new Date().toISOString(),
    });
    await handle.recordCheckpoint(`older_${suffix}`, "7");
    results.a_journal_without_the_intake_tables_gains_them_on_open =
      dropped === true &&
      fired.execution === `exec_older_${suffix}` &&
      (await handle.checkpointOf(`older_${suffix}`)) === "7";
  } finally {
    await journal.releaseJournal().catch(() => undefined);
  }
}

/**
 * Run one statement against the journal **from outside this process's
 * handle** — the way a build before PRD resolved q77 left a row, or a reader
 * counts what a sweep removed — on whichever backend this project bound.
 * Answers the rows a query returned.
 */
async function outside(provider, url, statement, parameters = []) {
  if (provider === "sqlite") {
    const { Database } = await import("node-sqlite3-wasm");
    const database = new Database(journal.journalPath());
    try {
      if (/^\s*(select|explain)/i.test(statement)) return database.all(statement, parameters);
      database.run(statement, parameters);
      return [];
    } finally {
      database.close();
    }
  }
  if (provider === "postgres") {
    const { Client } = await import("pg");
    const client = new Client({ connectionString: url });
    await client.connect();
    try {
      let next = 0;
      const numbered = statement.replace(/\?/g, () => `$${++next}`);
      return (await client.query(numbered, parameters)).rows ?? [];
    } finally {
      await client.end().catch(() => undefined);
    }
  }
  const { createConnection } = await import("mysql2/promise");
  const connection = await createConnection({ uri: url });
  try {
    const [rows] = await connection.query(statement, parameters);
    return Array.isArray(rows) ? rows : [];
  } finally {
    await connection.end().catch(() => undefined);
  }
}

/**
 * The columns of the index a dedupe claim's sweep reads down
 * (`DEDUPE_SWEEPS`, `docs/durability.md` §3.12), in index order, as the
 * bound backend's own catalogue lists them — none where it has no such index.
 */
async function sweepIndexColumns(provider, url) {
  const statement =
    provider === "sqlite"
      ? "SELECT name FROM pragma_index_info('dedupe_by_expiry') ORDER BY seqno"
      : provider === "postgres"
        ? "SELECT a.attname AS name FROM pg_index i " +
          "JOIN pg_class c ON c.oid = i.indexrelid " +
          "JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = ANY (i.indkey) " +
          "WHERE c.relname = 'dedupe_by_expiry' AND pg_table_is_visible(c.oid) " +
          "ORDER BY array_position(i.indkey::int2[], a.attnum)"
        : "SELECT COLUMN_NAME AS name FROM information_schema.STATISTICS WHERE TABLE_SCHEMA = DATABASE() " +
          "AND TABLE_NAME = 'dedupe' AND INDEX_NAME = 'dedupe_by_expiry' ORDER BY SEQ_IN_INDEX";
  return (await outside(provider, url, statement)).map((row) => String(row["name"]));
}

/** The one shape of that index: the trigger, then the window, then the admission. */
const SWEEP_INDEX = ["trigger_name", "expires_at", "admitted_at"];

// 8d. **A journal written before the intake ledger opens, and takes it**
//     (§3.12, §3.15, §11.2, PRD resolved q77). The dedupe row grew a column —
//     its window's end — which each backend's probe adds on open, and the
//     intake rows are a table of their own, which `CREATE TABLE IF NOT EXISTS`
//     adds. A dedupe row the older build wrote has no window, and is read as
//     expiring one window — the claim's — after it was admitted.
{
  const provider = journal.journalBinding.provider;
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const suffix = `${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  const older = `older_ledger_${suffix}`;
  const dropped = await (async () => {
    await outside(provider, url, "DROP TABLE IF EXISTS intakes");
    // The sweep's index goes first: it names the column, which SQLite will
    // not drop while an index holds it, and MySQL would drop from the index
    // and keep the index under its name — a shape no older build left.
    await outside(
      provider,
      url,
      provider === "mysql"
        ? "ALTER TABLE dedupe DROP INDEX dedupe_by_expiry"
        : "DROP INDEX IF EXISTS dedupe_by_expiry",
    );
    await outside(provider, url, "ALTER TABLE dedupe DROP COLUMN expires_at");
    // A row as the build before the window wrote it: no `expires_at`, admitted
    // two days ago — so a day's window has long passed over it.
    await outside(
      provider,
      url,
      "INSERT INTO dedupe (trigger_name, key_hash, dedupe_key, execution, admitted_at) VALUES (?, ?, ?, ?, ?)",
      [
        older,
        "0".repeat(64),
        JSON.stringify("never-expiring"),
        "exec_before_the_window",
        new Date(Date.now() - 2 * 86_400_000).toISOString(),
      ],
    );
    return true;
  })().catch((error) => (error instanceof Error ? error.message : String(error)));
  try {
    handle = await opened(journal);
    const now = new Date();
    const later = new Date(now.getTime() + 86_400_000);
    const row = await handle.admitIntake({
      id: `int_older_${suffix}`,
      trigger: older,
      state: "queued",
      execution: `exec_older_${suffix}`,
      waited: true,
      admission: { inputs: {}, sessionKey: "" },
      coalesced: 1,
      admittedAt: now.toISOString(),
    });
    const columns = await outside(
      provider,
      url,
      provider === "sqlite"
        ? "SELECT name FROM pragma_table_info('dedupe') WHERE name = 'expires_at'"
        : "SELECT column_name AS name FROM information_schema.columns WHERE table_name = 'dedupe' AND column_name = 'expires_at'",
    );
    // The legacy row's key is not this one's digest, so the sweep is what is
    // asked about: a claim of the trigger at `now` with a day's window sweeps a
    // row admitted two days ago with no window of its own.
    await handle.claimDedupe({
      trigger: older,
      key: "a fresh key",
      execution: `exec_fresh_${suffix}`,
      admittedAt: now.toISOString(),
      expiresAt: later.toISOString(),
    });
    const left = await outside(
      provider,
      url,
      "SELECT execution FROM dedupe WHERE trigger_name = ? AND execution = ?",
      [older, "exec_before_the_window"],
    );
    const indexed = await sweepIndexColumns(provider, url);
    results.a_journal_written_before_the_intake_ledger_gains_it_on_open =
      dropped === true &&
      row.id === `int_older_${suffix}` &&
      row.state === "queued" &&
      columns.length >= 1 &&
      JSON.stringify(indexed) === JSON.stringify(SWEEP_INDEX) &&
      left.length === 0;
  } finally {
    await journal.releaseJournal().catch(() => undefined);
  }
}

// 13. **The intake ledger** (PRD resolved q77, `docs/durability.md` §3.12,
//     §3.15): a dedupe key's window, and the rows a trigger's `concurrency:`
//     admits under — each one's transitions, and the lease read off the rows.
try {
  handle = await opened(journal);
  const provider = journal.journalBinding.provider;
  const url = process.env[journal.journalBinding.urlEnv ?? ""] ?? "";
  const suffix = `${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  const base = Date.parse("2026-11-04T03:00:00.000Z");
  const at = (seconds) => new Date(base + seconds * 1000).toISOString();

  // The window is the row's TTL: inside it a repeat is answered with the
  // first; at its end the key is claimed again; and the claim's sweep takes
  // every row of the trigger whose window has passed, not only this key's.
  const hooks = `hooks_${suffix}`;
  const claim = (key, execution, second, window = 10) => ({
    trigger: hooks,
    key,
    execution,
    admittedAt: at(second),
    expiresAt: at(second + window),
  });
  const first = await handle.claimDedupe(claim(UNICODE, "exec_w1", 0));
  const inside = await handle.claimDedupe(claim(UNICODE, "exec_w2", 9));
  const other = await handle.claimDedupe(claim("other key", "exec_other", 1, 2));
  const after = await handle.claimDedupe(claim(UNICODE, "exec_w3", 10));
  const swept = await outside(
    provider,
    url,
    "SELECT execution FROM dedupe WHERE trigger_name = ? AND execution = ?",
    [hooks, "exec_other"],
  );
  results.a_dedupe_key_is_held_for_its_window_and_claimed_again_after_it =
    first.execution === "exec_w1" &&
    first.expiresAt === at(10) &&
    inside.execution === "exec_w1" &&
    other.execution === "exec_other" &&
    after.execution === "exec_w3" &&
    after.expiresAt === at(20);
  results.a_dedupe_claim_sweeps_the_triggers_expired_rows = swept.length === 0;

  // The sweep costs what expired, never what is live: every backend holds the
  // index it reads down, in its one shape, and SQLite's planner — the one
  // whose choice is a rule rather than a statistic — reads each half of it
  // as a range over the window's column, not a walk of the trigger's rows.
  const sweepIndex = await sweepIndexColumns(provider, url);
  let sweepPlans = true;
  if (provider === "sqlite") {
    for (const [statement, parameters] of [
      [journal.DEDUPE_SWEEPS.windowed, [hooks, at(0)]],
      [journal.DEDUPE_SWEEPS.windowless, [hooks, at(0)]],
    ]) {
      const plan = (await outside(provider, url, `EXPLAIN QUERY PLAN ${statement}`, parameters))
        .map((step) => String(step["detail"]))
        .join(" | ");
      sweepPlans &&= plan.includes("USING INDEX dedupe_by_expiry (trigger_name=? AND expires_at");
    }
  }
  results.a_dedupe_sweep_reads_only_expired_rows_down_its_index =
    JSON.stringify(sweepIndex) === JSON.stringify(SWEEP_INDEX) &&
    typeof journal.DEDUPE_SWEEPS?.windowed === "string" &&
    sweepPlans;

  // A takeover restarts the window it moves.
  const retaken = await handle.retakeDedupe(hooks, UNICODE, "exec_w3", "exec_w4", at(99));
  const held = await handle.claimDedupe(claim(UNICODE, "exec_w5", 11));
  results.a_dedupe_takeover_carries_its_new_window =
    retaken === true && held.execution === "exec_w4" && held.expiresAt === at(99);

  // An intake row round-trips, is answered again under its id, and is found
  // by the execution it names.
  const builds = `builds_${suffix}`;
  const intake = (id, state, execution, extra = {}) => ({
    id: `int_${id}_${suffix}`,
    trigger: builds,
    state,
    execution: `exec_${id}_${suffix}`,
    waited: true,
    coalesced: 1,
    admittedAt: new Date().toISOString(),
    ...extra,
  });
  const admission = { inputs: { goal: UNICODE }, sessionKey: "s", callback: "https://hooks.example/x" };
  const queuedA = await handle.admitIntake(
    intake("a", "queued", "a", { group: UNICODE, admission }),
  );
  const again = await handle.admitIntake(intake("a", "dropped", "a", { group: "other" }));
  const queuedB = await handle.admitIntake(intake("b", "queued", "b", { group: UNICODE, admission }));
  const queuedC = await handle.admitIntake(intake("c", "queued", "c", { admission }));
  const read = await handle.intake(queuedA.id);
  const named = await handle.intakeOf(`exec_a_${suffix}`);
  results.an_intake_row_round_trips_and_is_admitted_once =
    queuedA.state === "queued" &&
    again.state === "queued" &&
    again.group === UNICODE &&
    read?.admission?.inputs?.goal === UNICODE &&
    read?.admission?.callback === "https://hooks.example/x" &&
    read?.waited === true &&
    named?.id === queuedA.id &&
    (await handle.intake(`int_nope_${suffix}`)) === undefined;

  // A queue drains in the order its rows went in.
  const queue = await handle.intakesOf(builds, "queued");
  results.a_triggers_queue_reads_in_admission_order =
    queue.map((row) => row.id).join(",") === [queuedA.id, queuedB.id, queuedC.id].join(",") &&
    queuedC.group === undefined;

  // A move is conditional on the state it leaves: the first mover of a row
  // wins, and a second move out of the same state takes nothing.
  const moved = await handle.moveIntake(queuedA.id, "queued", {
    state: "started",
    startedAt: new Date().toISOString(),
  });
  const twice = await handle.moveIntake(queuedA.id, "queued", { state: "started" });
  const startedA = await handle.intake(queuedA.id);
  const droppedC = await handle.moveIntake(queuedC.id, "queued", {
    state: "dropped",
    heldBy: "exec_running",
    detail: UNSERIALIZED,
  });
  const readC = await handle.intake(queuedC.id);
  results.an_intake_move_is_won_once =
    moved === true &&
    twice === false &&
    startedA?.state === "started" &&
    typeof startedA?.startedAt === "string" &&
    droppedC === true &&
    readC?.state === "dropped" &&
    readC?.heldBy === "exec_running" &&
    readC?.detail === UNSERIALIZED;

  // A debounce's row: found as its group's one pending row, extended in place,
  // and started once.
  const pending = await handle.admitIntake(
    intake("p", "pending", "p", { group: "branch", payload: { body: { n: 1 } }, deadline: at(30) }),
  );
  const open = await handle.pendingIntake(builds, "branch");
  const none = await handle.pendingIntake(builds, "another branch");
  const extended = await handle.moveIntake(pending.id, "pending", {
    state: "pending",
    payload: { body: { n: 2 } },
    coalesced: 2,
    deadline: at(40),
  });
  const extendedRow = await handle.intake(pending.id);
  const closed = await handle.moveIntake(pending.id, "pending", { state: "started", admission });
  const reopened = await handle.pendingIntake(builds, "branch");
  results.a_pending_intake_is_extended_and_started_once =
    open?.id === pending.id &&
    none === undefined &&
    extended === true &&
    extendedRow?.coalesced === 2 &&
    extendedRow?.deadline === at(40) &&
    extendedRow?.payload?.body?.n === 2 &&
    closed === true &&
    reopened === undefined;

  // The lease, read off the rows: a `started` row holds a slot while its
  // execution's lifecycle row is open and nobody has asked to cancel it.
  const lifecycle = (id, startedAt) => ({
    id,
    flow: "flow.review",
    trigger: builds,
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt,
  });
  const execA = `exec_a_${suffix}`;
  const execB = `exec_b_${suffix}`;
  const execP = `exec_p_${suffix}`;
  await handle.begin(lifecycle(execA, at(1)));
  await handle.moveIntake(queuedB.id, "queued", { state: "started" });
  await handle.begin(lifecycle(execB, at(2)));
  await handle.begin(lifecycle(execP, at(3)));
  const three = await handle.leasesOf(builds);
  await handle.requestCancel(execB);
  await handle.end(execP, "completed");
  const one = await handle.leasesOf(builds);
  results.a_lease_is_the_open_uncancelled_executions_of_the_rows =
    three.map((lease) => lease.execution).join(",") === [execA, execB, execP].join(",") &&
    three[0]?.group === UNICODE &&
    three[2]?.group === "branch" &&
    three[0]?.startedAt === at(1) &&
    one.map((lease) => lease.execution).join(",") === execA &&
    (await handle.leasesOf(`never_${suffix}`)).length === 0;

  // A start nobody was told of — never waited, its execution never begun —
  // is found and taken back; one that waited is found and kept.
  const quiet = await handle.admitIntake(
    intake("q", "started", "q", { waited: false, admission }),
  );
  const owed = await handle.admitIntake(intake("o", "started", "o", { admission }));
  const unbegun = await handle.unbegunIntakes(builds);
  const tookOwed = await handle.abandonIntake(owed.id);
  const tookQuiet = await handle.abandonIntake(quiet.id);
  results.an_unbegun_start_is_found_and_only_an_untold_one_taken_back =
    unbegun.map((row) => row.id).join(",") === [quiet.id, owed.id].join(",") &&
    tookOwed === false &&
    tookQuiet === true &&
    (await handle.intake(quiet.id)) === undefined &&
    (await handle.intake(owed.id))?.state === "started";

  // A debounce counts an arrival in the move that records it: the row names
  // the id the arrival's gate was claimed under, and is found by that id as
  // well as by its own execution — which is what tells a counted arrival's
  // repeat from a claim no debounce counted.
  const burst = `burst_${suffix}`;
  const burstRow = await handle.admitIntake({
    id: `int_burst_${suffix}`,
    trigger: burst,
    state: "pending",
    execution: `exec_burst_${suffix}`,
    waited: true,
    payload: { body: { n: 1 } },
    coalesced: 1,
    admittedAt: at(0),
    deadline: at(5),
  });
  const counted = await handle.moveIntake(burstRow.id, "pending", {
    state: "pending",
    payload: { body: { n: 2 } },
    coalesced: 2,
    deadline: at(6),
    joined: { execution: `exec_arrival_${suffix}`, gate: { kind: "dedupe", key: UNICODE } },
  });
  const countedRow = await handle.intake(burstRow.id);
  const byArrival = await handle.intakeOf(`exec_arrival_${suffix}`);
  const byOwn = await handle.intakeOf(`exec_burst_${suffix}`);
  results.a_debounced_arrival_is_found_by_the_id_its_gate_was_claimed_under =
    burstRow.joined === undefined &&
    counted === true &&
    countedRow?.coalesced === 2 &&
    countedRow?.joined?.execution === `exec_arrival_${suffix}` &&
    countedRow?.joined?.gate?.kind === "dedupe" &&
    countedRow?.joined?.gate?.key === UNICODE &&
    byArrival?.id === burstRow.id &&
    byArrival?.execution === `exec_burst_${suffix}` &&
    byOwn?.id === burstRow.id &&
    (await handle.intakeOf(`exec_never_counted_${suffix}`)) === undefined;

  // Folding a gate re-points it at the pending row's execution and keeps its
  // window; it moves a row only from the id it still names, for a dedupe key
  // and a tick alike.
  const folds = `folds_${suffix}`;
  const claimedOnce = await handle.claimDedupe({
    trigger: folds,
    key: UNICODE,
    execution: "exec_arrival",
    admittedAt: at(0),
    expiresAt: at(30),
  });
  await handle.foldGate(folds, { kind: "dedupe", key: UNICODE }, "exec_somebody_else", "exec_wrong");
  const unmoved = await handle.claimDedupe({
    trigger: folds,
    key: UNICODE,
    execution: "exec_repeat",
    admittedAt: at(1),
    expiresAt: at(31),
  });
  await handle.foldGate(folds, { kind: "dedupe", key: UNICODE }, "exec_arrival", "exec_pending");
  const folded = await handle.claimDedupe({
    trigger: folds,
    key: UNICODE,
    execution: "exec_repeat",
    admittedAt: at(2),
    expiresAt: at(32),
  });
  const tick = at(0);
  await handle.fire({ trigger: folds, scheduledAt: tick, execution: "exec_tick", firedAt: at(0) });
  await handle.foldGate(folds, { kind: "tick", scheduledAt: tick }, "exec_tick", "exec_pending");
  const firedFolded = await handle.fire({
    trigger: folds,
    scheduledAt: tick,
    execution: "exec_tick_again",
    firedAt: at(1),
  });
  results.a_gate_folds_onto_its_pending_rows_execution_and_keeps_its_window =
    claimedOnce.execution === "exec_arrival" &&
    unmoved.execution === "exec_arrival" &&
    folded.execution === "exec_pending" &&
    folded.admittedAt === at(0) &&
    folded.expiresAt === at(30) &&
    firedFolded.execution === "exec_pending" &&
    firedFolded.firedAt === at(0);

  // The lease is read off the open lifecycle rows of the trigger: a settled
  // execution's `started` row — the history every trigger keeps — holds no
  // slot however many of them there are, and an open execution another
  // trigger started is no slot of this one's.
  const history = `history_${suffix}`;
  const settledIds = [];
  for (let n = 0; n < 25; n += 1) {
    const execution = `exec_history_${n}_${suffix}`;
    settledIds.push(execution);
    await handle.admitIntake({
      id: `int_history_${n}_${suffix}`,
      trigger: history,
      state: "started",
      execution,
      waited: false,
      coalesced: 1,
      admittedAt: at(n),
      startedAt: at(n),
    });
    await handle.begin({ ...lifecycle(execution, at(n)), trigger: history });
    await handle.end(execution, "completed");
  }
  const live = `exec_history_live_${suffix}`;
  await handle.admitIntake({
    id: `int_history_live_${suffix}`,
    trigger: history,
    state: "started",
    execution: live,
    waited: false,
    coalesced: 1,
    admittedAt: at(40),
    startedAt: at(40),
  });
  await handle.begin({ ...lifecycle(live, at(40)), trigger: history });
  const historic = await handle.leasesOf(history);
  results.a_lease_holds_only_what_is_running_however_much_the_trigger_ran =
    historic.length === 1 &&
    historic[0]?.execution === live &&
    historic[0]?.startedAt === at(40) &&
    (await handle.leasesOf(builds)).every((lease) => lease.execution !== live);
} finally {
  await journal.releaseJournal().catch(() => undefined);
}

// 12. **The intake** (PRD resolved q78, `docs/durability.md` §3.11-§3.14): the
//     four ledgers that decide whether an execution exists. Every name below is
//     suffixed with this run's own token, because a dialled journal is shared
//     with every earlier run of this suite and these tables are keyed by trigger
//     and source rather than by execution.
try {
  handle = await opened(journal);
  const suffix = `${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
  const nightly = `nightly_${suffix}`;
  const at = (minute) => `2026-11-04T03:${String(minute).padStart(2, "0")}:00.000Z`;
  const tick = (minute, execution, trigger = nightly) => ({
    trigger,
    scheduledAt: at(minute),
    execution,
    firedAt: new Date().toISOString(),
  });

  // A tick fires once: the insert that wins fires, and every later claim of the
  // same tick is answered with the execution that holds it.
  const won = await handle.fire(tick(0, "exec_first"));
  const lost = await handle.fire(tick(0, "exec_second"));
  results.a_tick_is_claimed_once_and_a_second_claim_is_answered_with_the_first =
    won.execution === "exec_first" && lost.execution === "exec_first";

  // …including claims made together, which is the race two hubs on one journal
  // are when they wake for the same tick.
  const racing = await Promise.all(
    [1, 2, 3, 4, 5].map((n) => handle.fire(tick(1, `exec_racer_${n}`))),
  );
  results.claims_of_one_tick_made_together_agree_on_one_winner =
    new Set(racing.map((row) => row.execution)).size === 1;

  // …and a tick another writer of the same journal claimed first — a second
  // connection, the hub that woke a moment earlier — is that writer's: this
  // handle's claim of it is answered with the other's execution, and fires
  // nothing (`docs/distributed.md` §8 rule 2's stated exception).
  const elsewhere = await claimTickElsewhere(
    journal.journalBinding.provider,
    process.env[journal.journalBinding.urlEnv ?? ""] ?? "",
    nightly,
    at(2),
    "exec_other_hub",
  ).catch((error) => (error instanceof Error ? error.message : String(error)));
  const behind = await handle.fire(tick(2, "exec_this_hub"));
  results.a_tick_another_writer_claimed_first_is_answered_with_its_execution =
    elsewhere === true && behind.execution === "exec_other_hub";

  // A trigger name differing only in case is another trigger.
  const cased = await handle.fire(tick(0, "exec_cased", `NIGHTLY_${suffix}`));
  results.tick_claims_are_case_sensitive_by_trigger = cased.execution === "exec_cased";

  // The newest tick is the greatest instant, whatever order they were claimed in.
  await handle.fire(tick(30, "exec_thirty"));
  await handle.fire(tick(15, "exec_fifteen"));
  const last = await handle.lastFired(nightly);
  const all = await handle.firedOf(nightly);
  results.the_newest_tick_is_the_greatest_instant =
    last?.scheduledAt === at(30) &&
    last?.execution === "exec_thirty" &&
    all.map((row) => row.scheduledAt).join(",") === [at(0), at(1), at(2), at(15), at(30)].join(",");
  results.a_trigger_that_never_fired_has_no_last_tick =
    (await handle.lastFired(`never_${suffix}`)) === undefined;

  // Dedupe: a key is claimed once, by the first execution; the key is stored
  // whole and keyed by its digest, so a key of any length and any character
  // is one row — and two keys differing in case are two.
  const ingest = `ingest_${suffix}`;
  const admit = (key, execution, trigger = ingest) => ({
    trigger,
    key,
    execution,
    admittedAt: new Date().toISOString(),
  });
  const first = await handle.claimDedupe(admit(UNICODE, "exec_a"));
  const again = await handle.claimDedupe(admit(UNICODE, "exec_b"));
  const upper = await handle.claimDedupe(admit(UNICODE.toUpperCase(), "exec_c"));
  const large = await handle.claimDedupe(admit(LARGE, "exec_large"));
  const largeAgain = await handle.claimDedupe(admit(LARGE, "exec_large_again"));
  const otherTrigger = await handle.claimDedupe(admit(UNICODE, "exec_other", `other_${suffix}`));
  results.a_dedupe_key_is_claimed_once_and_a_repeat_is_answered_with_the_first =
    first.execution === "exec_a" && again.execution === "exec_a" && again.key === UNICODE;
  results.dedupe_keys_differing_in_case_are_two_keys = upper.execution === "exec_c";
  results.a_dedupe_key_of_any_length_is_one_row =
    large.execution === "exec_large" && largeAgain.execution === "exec_large" && largeAgain.key === LARGE;
  results.a_dedupe_key_is_scoped_to_its_trigger = otherTrigger.execution === "exec_other";

  // A claim whose execution never began is taken over, conditionally: only
  // from the execution the row still names.
  const moved = await handle.retakeDedupe(ingest, UNICODE, "exec_a", "exec_d");
  const stale = await handle.retakeDedupe(ingest, UNICODE, "exec_a", "exec_e");
  const holder = await handle.claimDedupe(admit(UNICODE, "exec_f"));
  results.a_dedupe_claim_is_taken_over_only_from_its_holder =
    moved === true && stale === false && holder.execution === "exec_d";

  // Checkpoints: absent, then the newest written.
  const feed = `feed_${suffix}`;
  const none = await handle.checkpointOf(feed);
  await handle.recordCheckpoint(feed, "10");
  await handle.recordCheckpoint(feed, "11");
  const eleven = await handle.checkpointOf(feed);
  await handle.recordCheckpoint(feed, UNICODE);
  const unicode = await handle.checkpointOf(feed);
  await handle.recordCheckpoint(feed, LARGE);
  const largest = await handle.checkpointOf(feed);
  results.a_checkpoint_is_absent_until_written_and_then_the_newest =
    none === undefined && eleven === "11" && unicode === UNICODE && largest === LARGE;

  // Restarts: every row kept, the newest first, a page bounded, and how each
  // run ended — a code or a signal — carried as it was.
  const base = Date.parse("2026-11-04T03:00:00.000Z");
  const ended = (n) => new Date(base + n * 1000).toISOString();
  await handle.recordRestart({ source: feed, endedAt: ended(1), attempt: 1, exitCode: 0, ranMs: 1200, backoffMs: 1000 });
  await handle.recordRestart({ source: feed, endedAt: ended(2), attempt: 2, exitCode: 3, ranMs: 50, backoffMs: 2000 });
  await handle.recordRestart({ source: feed, endedAt: ended(3), attempt: 3, signal: "SIGKILL", ranMs: 4_000_000_000, backoffMs: 4000 });
  const page = await handle.restartsOf(feed, 2);
  const empty = await handle.restartsOf(`quiet_${suffix}`, 20);
  results.restart_rows_are_paged_newest_first_with_their_total =
    page.total === 3 &&
    page.rows.length === 2 &&
    page.rows[0]?.endedAt === ended(3) &&
    page.rows[0]?.signal === "SIGKILL" &&
    page.rows[0]?.exitCode === undefined &&
    page.rows[0]?.ranMs === 4_000_000_000 &&
    page.rows[1]?.exitCode === 3 &&
    page.rows[1]?.signal === undefined &&
    page.rows[1]?.attempt === 2 &&
    empty.total === 0 &&
    empty.rows.length === 0;
} finally {
  await journal.releaseJournal().catch(() => undefined);
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
