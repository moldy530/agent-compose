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
 * Two sites distinct only by case, which a case-insensitive key column merges.
 *
 * A site of their own rather than a second reading of `review/0`: the append
 * below is idempotent on `(execution, key)`, so a key this journal already holds
 * would answer the first record whatever case the second was written in — and
 * the case would pass for the wrong reason.
 */
const CASED = ["cased/0", "CASED/0"];

const results = {};
let handle;

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
  // 1. **Open and create, twice.** The schema is created on first open with
  //    `IF NOT EXISTS`, so a second open of a journal this build already made
  //    does nothing (`docs/durability.md` §10, §11.2).
  handle = await journal.openJournal();
  const again = await journal.openJournal();
  results.open_is_idempotent = again === handle;
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

  // 8. **The dispatch board**, whose park order breaks ties by insertion rather
  //    than by the millisecond two rows share (`docs/distributed.md` §6.2).
  const parkedAt = new Date().toISOString();
  for (const index of [0, 1, 2, 10, 11]) {
    await handle.park({
      execution,
      wait: `map/0/${index}`,
      id: `dsp_${index}`,
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
    id: "dsp_other",
    placement: "gpu",
    node: "flow.review.embed",
    site: "map/0/0",
    inputs: { item: 99 },
    status: "parked",
    parkedAt: new Date().toISOString(),
  });
  results.park_is_idempotent = reparked.id === "dsp_0";
  results.a_claim_hands_the_row_over =
    (await handle.claimDispatch("dsp_1", "session-a"))?.session === "session-a";
  results.a_second_claim_takes_nothing =
    (await handle.claimDispatch("dsp_1", "session-b")) === undefined;
  results.a_settle_answers_once =
    (await handle.settleDispatch("dsp_1", { kind: "value", value: { ok: UNICODE } })) === true &&
    (await handle.settleDispatch("dsp_1", { kind: "value", value: { ok: "again" } })) === false;
  results.a_settled_dispatch_keeps_its_outcome =
    (await handle.dispatchOf("dsp_1"))?.outcome?.value?.ok === UNICODE;
} finally {
  // Always, and before the writer guard's own case below: a connection left open
  // is a lock the next opener meets.
  await journal.releaseJournal().catch(() => undefined);
}

// 9. **The writer guard.** A second opener of a journal a live process holds is
//    refused by name rather than left to interleave (`docs/durability.md` §2).
//    Only the two remote backends have one — SQLite's file lock is broken after
//    a deadline by design, which §2.2 states and §12 repeats — so the case is
//    reported as skipped there rather than asserted away.
if (journal.journalBinding.provider === "sqlite") {
  results.writer_guard_refuses_a_second_opener = "not-applicable";
} else {
  const held = await journal.openJournal();
  // A second module instance, loaded under a query string so the runtime treats
  // it as a different module and it opens a connection of its own — which is
  // what a second *process* would do, at the only level this runner can do it.
  const rival = await import(`${at("src/journal.ts")}?rival`);
  try {
    await rival.openJournal();
    results.writer_guard_refuses_a_second_opener = "not-refused";
  } catch (error) {
    results.writer_guard_refuses_a_second_opener = error instanceof Error ? error.message : String(error);
  } finally {
    await rival.releaseJournal().catch(() => undefined);
    await journal.releaseJournal().catch(() => undefined);
  }
  void held;
}

process.stdout.write(JSON.stringify(results));
