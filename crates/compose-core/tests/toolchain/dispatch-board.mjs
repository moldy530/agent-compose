// Drives a generated project's dispatch board directly, and reports what it did
// (`docs/distributed.md` §3.4, §6.1, §6.2).
//
// The served-hub suite decides "a placed node runs somewhere else"; this is the
// other half, the two properties of the board that a hub answering HTTP cannot
// show:
//
//   * **park order breaks its ties by insertion.** §6.2 says dispatch resumes
//     "in park order", and `parked_at` is an ISO instant with millisecond
//     resolution — a fan-out parks every instance from one synchronous burst, so
//     they all carry the same one. The tiebreak has to be the order they went on
//     the board. Showing that the *old* tiebreak was wrong needs at least ten
//     waits at one instance path, because it was a string compare and `sign/10`
//     sorts before `sign/2`; no fixture a served hub runs fans out that wide, and
//     making one would spend ten dispatches to observe an ordering.
//
//   * **`settleDispatch` answers whether *this* call settled it.** The
//     contract's own words, and the difference is invisible over the wire: the
//     route checks the row's status before it calls, so a second result is
//     `204` either way. A caller that trusted the answer to tell a first settle
//     from a re-post would take the second result's outcome as newly journaled.
//
//   * **`releaseDispatch` is the exact inverse of a claim.** §7 makes dispatch
//     at-least-once, and where the hub can tell an answer never reached the
//     worker it was written for, re-issuing is putting the row back rather than
//     waiting out the node's whole deadline. What has to hold of that is not
//     reachable over HTTP either: the row goes back to its **own place** in the
//     park order rather than to the end of the queue, only the session holding
//     it may hand it back, and a row that has moved on — settled, or superseded
//     — is left exactly as it is.
//
// `src/journal.ts` is a compiler constant, byte-identical in every project this
// release builds, so driving it directly is driving what every project runs.
//
// Usage: node dispatch-board.mjs <generated project directory>
// Output: one JSON object of observations; the expectations live in the Rust
// test that reads it (`generated_code_gates.rs`).

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node dispatch-board.mjs <generated project directory>");
}

const journalModule = await import(pathToFileURL(path.resolve(project, "src/journal.ts")).href);
const journal = await journalModule.openJournal();

const execution = `exec_board_${Date.now()}`;
await journal.begin({
  id: execution,
  flow: "flow.release",
  trigger: "manual",
  inputs: {},
  sessionKey: "",
  status: "open",
  journalVersion: journalModule.JOURNAL_VERSION,
  startedAt: new Date().toISOString(),
});

// One instant for every row, which is what a synchronous burst of
// `dispatchPlaced` calls produces.
const parkedAt = new Date().toISOString();
const wanted = [];
for (let ordinal = 0; ordinal < 12; ordinal += 1) {
  wanted.push(`sign/${ordinal}`);
  // Awaited one at a time, which is what makes the burst below an *insertion*
  // order rather than a race: the board's tiebreak is the order the rows went
  // in, so a harness that started twelve writes at once would be asserting on
  // whichever order the driver happened to finish them in.
  await journal.park({
    execution,
    wait: `sign/${ordinal}`,
    id: `dsp_board_${ordinal}`,
    placement: "mac",
    node: "flow.release.sign",
    site: "sign",
    inputs: { ordinal },
    status: "parked",
    parkedAt,
  });
}

const order = (await journal.unsettledDispatches())
  .filter((row) => row.execution === execution)
  .map((row) => row.wait);

// The same question of the per-execution read, which `dispatchesOf`'s own
// contract calls park order too.
const ofExecution = (await journal.dispatchesOf(execution)).map((row) => row.wait);

// §3.4's two verbs, through the answer the contract gives each. Awaited in turn,
// because "did *this* call settle it" is a question about two calls in sequence:
// two settles started together would be two readers of the same unsettled row.
const first = await journal.settleDispatch("dsp_board_0", {
  kind: "value",
  value: { signature: "s" },
});
const again = await journal.settleDispatch("dsp_board_0", {
  kind: "value",
  value: { signature: "t" },
});
const held = await journal.dispatchOf("dsp_board_0");

await journal.supersedeDispatch("dsp_board_1", "the session holding it went quiet");
const afterSupersede = await journal.settleDispatch("dsp_board_1", {
  kind: "value",
  value: { signature: "u" },
});

const unknown = await journal.settleDispatch("dsp_board_nothing", { kind: "value", value: {} });

// §7's at-least-once, from the journal's side: a claim, and the exact inverse of
// it. `dsp_board_5` is picked out of the middle of the queue so that "back in
// its own place" is a different answer from "back at either end".
const claimed = await journal.claimDispatch("dsp_board_5", "wrk_one");
const releasedByAnother = await journal.releaseDispatch("dsp_board_5", "wrk_two");
const released = await journal.releaseDispatch("dsp_board_5", "wrk_one");
const afterRelease = await journal.dispatchOf("dsp_board_5");
const orderAfterRelease = (await journal.unsettledDispatches())
  .filter((row) => row.execution === execution)
  .map((row) => row.wait);
// …and the two rows that have moved on. Neither may be handed back, whoever
// asks: one holds a worker's result and the other a hub's supersede, and a
// release that took either would put work back on the board that the execution
// has already gone past.
const releasedSettled = await journal.releaseDispatch("dsp_board_0", "wrk_one");
const releasedSuperseded = await journal.releaseDispatch("dsp_board_1", "wrk_one");

// §3.2's OPTIONAL payload fields, journaled beside `inputs` and read back.
await journal.park({
  execution,
  wait: "conversation/0",
  id: "dsp_board_payload",
  placement: "mac",
  node: "flow.conversation.sign",
  site: "conversation",
  inputs: { path: "release.dmg" },
  itemIndex: 3,
  history: [{ role: "assistant", text: "a release of release.dmg" }],
  policy: { timeoutMs: 30_000 },
  status: "parked",
  parkedAt,
});
const payload = await journal.dispatchOf("dsp_board_payload");

// The rows the report below reads twice, read once here: every journal verb
// answers a promise now, and an `await` inside the object literal would be a
// read whose place in the sequence is harder to see than its value is worth.
const settledRow = await journal.dispatchOf("dsp_board_0");
const supersededRow = await journal.dispatchOf("dsp_board_1");
const plainRow = await journal.dispatchOf("dsp_board_2");
const insertion = (await journal.unsettledDispatches())
  .filter((row) => row.execution === execution)
  .map((row) => row.order ?? null);

process.stdout.write(
  `${JSON.stringify({
    parkOrder: {
      wanted,
      order,
      ofExecution,
      // The tiebreak itself, carried on the row: the backend's insertion-order
      // column, which is what `ORDER BY parked_at ASC, insertion_order ASC`
      // breaks a shared instant with and what a reader of these rows has to
      // sort by to agree with the queue it is reporting on.
      insertion,
    },
    release: {
      claimed: claimed?.status ?? null,
      claimedBy: claimed?.session ?? null,
      releasedByAnother,
      released,
      status: afterRelease?.status ?? null,
      session: afterRelease?.session ?? "absent",
      dispatchedAt: afterRelease?.dispatchedAt ?? "absent",
      orderAfterRelease,
      wantedAfterRelease: wanted.slice(2),
      releasedSettled,
      settledStatus: settledRow?.status ?? null,
      releasedSuperseded,
      supersededStatusAfter: supersededRow?.status ?? null,
    },
    settlement: {
      first,
      again,
      outcome: held?.outcome ?? null,
      afterSupersede,
      supersededStatus: supersededRow?.status ?? null,
      unknown,
    },
    payload: {
      itemIndex: payload?.itemIndex ?? null,
      history: payload?.history ?? null,
      policy: payload?.policy ?? null,
      // A row parked without them carries none, which is what lets `taken()`
      // omit the keys §3.2 makes optional rather than send `null`.
      absent: {
        itemIndex: plainRow?.itemIndex ?? "absent",
        history: plainRow?.history ?? "absent",
        policy: plainRow?.policy ?? "absent",
      },
    },
  })}\n`,
);
