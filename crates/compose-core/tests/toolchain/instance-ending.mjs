// The end-of-instance seam, driven directly with the one outcome nothing in this
// release can reach it with: `cancelled` (grammar §7.9, Decision D159, PRD
// resolved q75, q76).
//
// PRD resolved q75's cancel route is not built, so no `run`, `resume` or
// `serve` request ends an instance `cancelled`, and no acceptance test can make
// one. The enum value, the boundary record and the seam carry it anyway, so
// that q75 adds a **caller** rather than a code path — and this runner is what
// holds them to that. It hands `runtime.endInstance` a `cancelled` ending for a
// real emitted `finally:` node — the `every-schema-form` golden's
// `flow.condense` node `note`, whose input binds `execution.outcome` — and
// reads back everything the seam is responsible for:
//
//   * the node ran, and read `cancelled` out of its own `execution` root, with
//     no `execution.error` beside it;
//   * its entry is the instance's last, one step past the body's, with no
//     `routing`;
//   * the boundary is on the record at the node's own site, claimed under the
//     outcome, holding the outcome;
//   * a later generation that reaches the same end is handed the recorded one
//     and runs the node again only past its record — and one that reaches a
//     **different** end is a `ReplayDivergence` naming the boundary, rather than
//     a finally node run a second time under another story;
//   * a `cancelled` instance whose finally node fails stays `cancelled`: the
//     failure is the execution's `finally_error` and the node's own entry.
//
// Usage: bun instance-ending.mjs <generated project directory>
// Prints one JSON object, read by `generated_code_gates.rs`.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: bun instance-ending.mjs <generated project directory>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const journal = await import(at("src/journal.ts"));
const runtime = await import(at("src/runtime.ts"));
const graph = await import(at("src/graph.ts"));

const results = {};

/** A fresh execution id, so a re-run meets none of its own rows. */
function freshExecution() {
  return `exec_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
}

/** The state an instance of `flow.condense` ended in, at the root. */
function ended(execution, step) {
  return {
    $run: {
      ...runtime.emptyRun(),
      step,
      execution: { id: execution, session_key: "" },
    },
  };
}

/**
 * The golden's own finally descriptor, with what its input builder answered
 * written down — the only place the value the node read is observable, since
 * the activity it runs is `true`.
 */
function observed(descriptor, seen, failing = undefined) {
  return {
    ...descriptor,
    input: (roots, view) => {
      const built = descriptor.input(roots, view);
      seen.push({ built, error: roots.execution.error });
      return built;
    },
    run: failing === undefined ? descriptor.run : async () => Promise.reject(failing),
  };
}

/** The class and message of whatever a promise raised, or `undefined`. */
async function raised(work) {
  try {
    await work();
    return undefined;
  } catch (error) {
    return { name: error?.name ?? "", message: error instanceof Error ? error.message : "" };
  }
}

const descriptor = graph.flows["flow.condense"]?.finally;
results.the_golden_names_its_finally_node = descriptor?.node === "note";

const handle = await journal.openJournal();
try {
  // 1. **A cancelled end, live.** One body entry at step 1, so the finally
  //    entry is at step 2.
  const execution = freshExecution();
  await handle.begin({
    id: execution,
    flow: "flow.condense",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  journal.openSession(execution, handle, false);
  const seen = [];
  const body = [{ step: 1, flow: "flow.condense", node: "reduce", traversal: 0, outcome: "completed", attempts: 1, writes: [] }];
  const ran = await runtime.endInstance(
    observed(descriptor, seen),
    ended(execution, 1),
    { outcome: "cancelled" },
    body,
  );
  journal.closeSession(execution);
  results.the_node_read_cancelled =
    seen.length === 1 && seen[0].built?.outcome === "cancelled" && seen[0].error === undefined;
  results.its_entry_is_the_instances_last =
    ran?.entry?.node === "note" &&
    ran?.entry?.outcome === "completed" &&
    ran?.entry?.traversal === 0 &&
    ran?.entry?.step === 2 &&
    ran?.entry?.routing === undefined;
  const boundary = await handle.lookup(execution, "note/0#finally/0");
  results.the_boundary_records_the_cancellation =
    boundary?.kind === "finally" &&
    boundary?.request === journal.canonical({ node: "note", outcome: "cancelled" }) &&
    boundary?.outcome?.kind === "value" &&
    boundary?.outcome?.value?.outcome === "cancelled" &&
    boundary?.outcome?.value?.error === undefined;
  results.a_cancelled_end_whose_node_completed_has_no_finally_error =
    runtime.finallyErrorOf(execution) === undefined;

  // 2. **The same end, replayed.** A resuming generation that reaches it again
  //    is handed the recorded outcome and runs the node — whose own effects are
  //    what the record answers past the boundary.
  journal.openSession(execution, handle, true);
  const again = [];
  const replayed = await runtime.endInstance(
    observed(descriptor, again),
    ended(execution, 1),
    { outcome: "cancelled" },
    body,
  );
  journal.closeSession(execution);
  results.a_replayed_cancellation_reads_the_record =
    replayed?.entry?.outcome === "completed" && again[0]?.built?.outcome === "cancelled";

  // 3. **Another end, replayed.** The record says `cancelled` and this
  //    generation's body ended `completed`: a divergence at the boundary, and
  //    the node never ran under the second story.
  journal.openSession(execution, handle, true);
  const diverged = [];
  const divergence = await raised(() =>
    runtime.endInstance(
      observed(descriptor, diverged),
      ended(execution, 1),
      { outcome: "completed" },
      body,
    ),
  );
  journal.closeSession(execution);
  results.another_end_over_a_recorded_cancellation_diverges =
    divergence?.name === "ReplayDivergence" &&
    divergence.message.includes("note/0#finally/0") &&
    diverged.length === 0;
  runtime.closeExecution(execution);

  // 4. **A cancelled instance whose finally node failed** stays cancelled: the
  //    failure is the node's entry and the execution's `finally_error`, and the
  //    seam answers rather than throwing it.
  const failing = freshExecution();
  journal.openSession(failing, handle, false);
  const broken = await runtime.endInstance(
    observed(descriptor, [], new Error("the cleanup could not finish")),
    ended(failing, 0),
    { outcome: "cancelled" },
    [],
  );
  journal.closeSession(failing);
  results.a_failed_finally_node_after_a_cancellation_is_recorded =
    broken?.entry?.outcome === "failed" &&
    broken?.entry?.step === 1 &&
    broken?.entry?.routing === undefined &&
    typeof broken?.entry?.error === "string" &&
    (runtime.finallyErrorOf(failing) ?? "").startsWith("flow.condense node `note` failed: ");
  const failedBoundary = await handle.lookup(failing, "note/0#finally/0");
  results.the_failed_nodes_boundary_still_says_cancelled =
    failedBoundary?.outcome?.value?.outcome === "cancelled";
  runtime.closeExecution(failing);
} finally {
  await journal.releaseJournal().catch(() => undefined);
}

process.stdout.write(JSON.stringify(results));
