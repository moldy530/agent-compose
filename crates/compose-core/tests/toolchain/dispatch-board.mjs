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
journal.begin({
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
  journal.park({
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

const order = journal
  .unsettledDispatches()
  .filter((row) => row.execution === execution)
  .map((row) => row.wait);

// The same question of the per-execution read, which `dispatchesOf`'s own
// contract calls park order too.
const ofExecution = journal.dispatchesOf(execution).map((row) => row.wait);

// §3.4's two verbs, through the answer the contract gives each.
const first = journal.settleDispatch("dsp_board_0", { kind: "value", value: { signature: "s" } });
const again = journal.settleDispatch("dsp_board_0", { kind: "value", value: { signature: "t" } });
const held = journal.dispatchOf("dsp_board_0");

journal.supersedeDispatch("dsp_board_1", "the session holding it went quiet");
const afterSupersede = journal.settleDispatch("dsp_board_1", {
  kind: "value",
  value: { signature: "u" },
});

const unknown = journal.settleDispatch("dsp_board_nothing", { kind: "value", value: {} });

// §3.2's OPTIONAL payload fields, journaled beside `inputs` and read back.
journal.park({
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
const payload = journal.dispatchOf("dsp_board_payload");

process.stdout.write(
  `${JSON.stringify({
    parkOrder: { wanted, order, ofExecution },
    settlement: {
      first,
      again,
      outcome: held?.outcome ?? null,
      afterSupersede,
      supersededStatus: journal.dispatchOf("dsp_board_1")?.status ?? null,
      unknown,
    },
    payload: {
      itemIndex: payload?.itemIndex ?? null,
      history: payload?.history ?? null,
      policy: payload?.policy ?? null,
      // A row parked without them carries none, which is what lets `taken()`
      // omit the keys §3.2 makes optional rather than send `null`.
      absent: {
        itemIndex: journal.dispatchOf("dsp_board_2")?.itemIndex ?? "absent",
        history: journal.dispatchOf("dsp_board_2")?.history ?? "absent",
        policy: journal.dispatchOf("dsp_board_2")?.policy ?? "absent",
      },
    },
  })}\n`,
);
