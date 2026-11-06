// A finally node that fails once this process has closed its execution's
// record (grammar §7.9, `docs/durability.md` §3.10, PRD resolved q76).
//
// A detached `agent.*` delivery runs on after the run that issued it has
// closed the execution's record (grammar §8.6 rule 7), and an instance its
// model begins from its `tools:` can end after that; a worker can post a placed
// node's `finally_error` after the hub settled (`docs/distributed.md` §3.4).
// Neither failure has a row to go on or an envelope to ride in, and nothing
// will release it once `closeExecution` has run — so the runtime holds nothing
// for it and says it on stderr instead. This runner drives both writers of
// `finally_error` — `runtime.endInstance` over the `every-schema-form`
// golden's `flow.condense` node `note`, and `runtime.holdFinallyError`, the
// hub's — once while the execution's session is open and once after it has
// closed, and reads back:
//
//   * while the record is open, the failure is the execution's
//     `finally_error` and the boundary is on the record;
//   * once it has closed, the finally node still answers a failed entry with
//     its `error`, but `finally_error` holds nothing and no boundary is
//     written — and the same is true of a worker's failure held after the
//     close.
//
// The stderr lines themselves are read by the gate, which knows the ids from
// the object printed here.
//
// Usage: bun finally-past-record.mjs <generated project directory>
// Prints one JSON object, read by `generated_code_gates.rs`.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: bun finally-past-record.mjs <generated project directory>");
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
function ended(execution) {
  return {
    $run: {
      ...runtime.emptyRun(),
      step: 0,
      execution: { id: execution, session_key: "" },
    },
  };
}

/** The golden's own finally descriptor, made to fail with `error`. */
function failing(descriptor, error) {
  return { ...descriptor, run: async () => Promise.reject(error) };
}

const descriptor = graph.flows["flow.condense"]?.finally;
results.the_golden_names_its_finally_node = descriptor?.node === "note";
const prefix = "flow.condense node `note` failed: ";
const onAWorker = `${prefix}the worker's cleanup could not finish`;

const handle = await journal.openJournal();
try {
  // 1. **While the record is open** — the control: the failure is held.
  const open = freshExecution();
  results.open = open;
  journal.openSession(open, handle, false);
  const inside = await runtime.endInstance(
    failing(descriptor, new Error("the cleanup could not finish")),
    ended(open),
    { outcome: "completed" },
    [],
  );
  results.a_failure_while_the_record_is_open_is_its_finally_error =
    inside?.entry?.outcome === "failed" &&
    (runtime.finallyErrorOf(open) ?? "").startsWith(prefix);
  results.an_end_while_the_record_is_open_writes_its_boundary =
    (await handle.lookup(open, "note/0#finally/0"))?.kind === "finally";
  journal.closeSession(open);
  runtime.closeExecution(open);

  // 2. **Once the record has closed.** The session is opened and closed again
  //    first, which is what `closeExecution` leaves behind it.
  const late = freshExecution();
  results.late = late;
  journal.openSession(late, handle, false);
  runtime.closeExecution(late);
  const past = await runtime.endInstance(
    failing(descriptor, new Error("the late cleanup could not finish")),
    ended(late),
    { outcome: "completed" },
    [],
  );
  results.a_finally_node_failing_past_the_record_still_answers_its_entry =
    past?.entry?.node === "note" &&
    past?.entry?.outcome === "failed" &&
    typeof past?.entry?.error === "string" &&
    past?.ending?.outcome === "completed";
  results.a_failure_past_the_record_is_held_nowhere = runtime.finallyErrorOf(late) === undefined;
  results.an_end_past_the_record_writes_no_boundary =
    (await handle.lookup(late, "note/0#finally/0")) === undefined;

  // 3. **A worker's failure, held by the hub**, on either side of the close.
  const placed = freshExecution();
  results.placed = placed;
  journal.openSession(placed, handle, false);
  runtime.holdFinallyError(placed, onAWorker);
  results.a_workers_failure_while_the_record_is_open_is_held =
    runtime.finallyErrorOf(placed) === onAWorker;
  runtime.closeExecution(placed);
  runtime.holdFinallyError(placed, onAWorker);
  results.a_workers_failure_past_the_record_is_held_nowhere =
    runtime.finallyErrorOf(placed) === undefined;
} finally {
  await journal.releaseJournal().catch(() => undefined);
}

process.stdout.write(JSON.stringify(results));
