// What a contract refusing a **live** answer leaves on that answer's record, and
// what the next generation reads off it (PRD resolved q29's second divergence,
// `docs/durability.md` §3, §7).
//
// The seam under test spans two emitted modules and is the one place in this
// runtime where a **synchronous** function starts a journal **write**:
// `runtime.parseResult` refuses an answer, `journal.refuseRecorded` marks that
// answer's record, and a later `resume` reads the mark to tell "the composition
// already had this mismatch and retried past it" from "this build is the first
// to refuse it" — a `ResultMismatch` a `retry:` absorbs against a
// `ReplayDivergence` no policy may absorb.
//
// None of it is reachable from a run's answer, and each half fails silently in a
// way the other hides:
//
//   * a mark that is **issued and never awaited** leaves the record unmarked
//     exactly as long as the process lives, so every assertion made inside one
//     run still passes — the failure is a resume, of an execution whose
//     composition nobody touched, months later;
//   * a mark that lands **after** the retried attempt's record is the same
//     failure for a process that dies in between, which is why the order the
//     journal saw its own writes in is a case here rather than a comment;
//   * a mark the journal **refuses** is a rejected promise: swallowed, it costs
//     nothing here and the same resume later; raised, it fails the node that was
//     about to write past it while the cause is still on the screen.
//
// So the journal the session is opened with is a recording wrapper around the
// project's real one: every `append` and `refuse` that lands is logged in the
// order the journal finished it, and one case's wrapper refuses to mark at all.
// Everything else is the emitted modules' own.
//
// Usage: node refused-answers.mjs <generated project directory>
// Prints one JSON object, read by `generated_code_gates.rs`.

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node refused-answers.mjs <generated project directory>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const journal = await import(at("src/journal.ts"));
const runtime = await import(at("src/runtime.ts"));

const results = {};

/** A contract that takes the answer a second attempt gives and no other. */
const CONTRACT = {
  safeParse: (value) =>
    value !== null && typeof value === "object" && value.verdict === "approve"
      ? { success: true, data: value }
      : {
          success: false,
          error: { issues: [{ path: ["verdict"], message: "expected `approve`" }] },
        },
};

/** The subject a refusal is reported under, quoted back by the Rust side. */
const SUBJECT = "the answer of `review`";

/** A fresh execution id, so a re-run meets none of its own rows. */
function freshExecution() {
  return `exec_${Math.random().toString(16).slice(2)}${Date.now().toString(16)}`;
}

/**
 * The project's own journal, with every write it finishes written down — and the
 * mark held back behind a round trip.
 *
 * `refuse` is the method under test and `append` is the one it has to be ordered
 * against, so both are logged **after** the real write answers: what the cases
 * below assert is the order the journal really settled them in, not the order
 * this file asked for them.
 *
 * The delay is what makes that order evidence of anything. The SQLite arm runs
 * every method of one journal on one queue, so a mark issued before an append is
 * finished before it whether or not anything waited for it — and a case run
 * against that alone would pass with the waiting deleted. A `postgres` or
 * `mysql` journal is a socket, which is what this stands in for: the mark takes
 * visibly longer than the append that follows it, so only a seam that really
 * waits for what the execution owes can put them in order.
 */
function recording(handle, order, refusal) {
  return {
    lookup: (execution, key) => handle.lookup(execution, key),
    append: async (record) => {
      await handle.append(record);
      order.push(`append ${record.key}`);
    },
    refuse: async (execution, key) => {
      await new Promise((resume) => setTimeout(resume, 25));
      if (refusal !== undefined) throw refusal;
      await handle.refuse(execution, key);
      order.push(`refuse ${key}`);
    },
  };
}

/** What one attempt at the site does: answer `value`, then parse it. */
async function attempt(execution, site, value) {
  const recorder = journal.recorderFor(execution, site);
  const answered = await journal.journaled(recorder, "model", { model: "model.smart" }, () =>
    Promise.resolve(value),
  );
  return runtime.parseResult(CONTRACT, answered, SUBJECT);
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

const handle = await journal.openJournal();

// 1. **A live answer this contract refuses** — the mismatch is the ordinary one
//    a `retry:` absorbs, and the mark for it is down before the record of the
//    attempt that mismatch set off (§3, §7).
{
  const execution = freshExecution();
  const order = [];
  await handle.begin({
    id: execution,
    flow: "flow.review",
    trigger: "manual",
    inputs: {},
    sessionKey: "",
    status: "open",
    journalVersion: journal.JOURNAL_VERSION,
    startedAt: new Date().toISOString(),
  });
  journal.openSession(execution, recording(handle, order), false);

  const refused = await raised(() => attempt(execution, "review/0", { verdict: "revise" }));
  results.a_live_refusal_is_a_mismatch = refused?.name === "ResultMismatch";
  results.the_mismatch_names_its_subject = refused?.message?.startsWith(SUBJECT) === true;

  // The ladder's second attempt, which is what the mark has to be ordered
  // against: its record is the one a resume reads *after* the refused one.
  const took = await attempt(execution, "review/0", { verdict: "approve" });
  results.the_retried_attempt_answers = took?.verdict === "approve";

  const record = await handle.lookup(execution, "review/0#model/0");
  results.the_mark_is_on_the_record = record?.refused === true;
  results.the_mark_lands_before_the_retried_attempts_record = order;

  journal.closeSession(execution);

  // 2. **What the next generation reads.** A resume replays the refused record,
  //    meets the same contract, and raises the mismatch its own ladder is
  //    allowed to absorb — because the generation that recorded the answer said
  //    on the record that it refused it too.
  journal.openSession(execution, recording(handle, []), true);
  const replayed = await raised(() => attempt(execution, "review/0", { verdict: "revise" }));
  results.a_marked_answer_replays_as_a_mismatch = replayed?.name === "ResultMismatch";
  journal.closeSession(execution);
}

// 3. **The control**, without which nothing above is evidence: the *same*
//    replay of an answer whose record carries no mark is the divergence resolved
//    q29 makes un-absorbable. This is the failure a dropped mark turns case 2
//    into, so it is driven rather than described.
{
  const execution = freshExecution();
  await handle.append({
    execution,
    key: "review/0#model/0",
    site: "review/0",
    kind: "model",
    ordinal: 0,
    request: journal.canonical({ model: "model.smart" }),
    outcome: { kind: "value", value: { verdict: "revise" } },
    refused: false,
    recordedAt: new Date().toISOString(),
  });
  journal.openSession(execution, recording(handle, []), true);
  const replayed = await raised(() => attempt(execution, "review/0", { verdict: "revise" }));
  results.an_unmarked_answer_replays_as_a_divergence = replayed?.name === "ReplayDivergence";
  results.the_divergence_names_the_step = replayed?.message?.includes("review/0#model/0") === true;
  journal.closeSession(execution);
}

// 4. **A mark the journal refuses** fails the node that was about to write past
//    it, rather than being swallowed with the record left unmarked (§3). The
//    failure is raised once: the attempt after it writes its record and the
//    execution goes on under the conservative half — an unmarked refusal a later
//    resume reports as a divergence naming the step.
{
  const execution = freshExecution();
  const order = [];
  const unwritable = new Error("the journal is unavailable");
  journal.openSession(execution, recording(handle, order, unwritable), false);

  const refused = await raised(() => attempt(execution, "review/1", { verdict: "revise" }));
  results.a_refused_mark_does_not_fail_the_parse = refused?.name === "ResultMismatch";

  const failed = await raised(() => attempt(execution, "review/1", { verdict: "approve" }));
  results.a_mark_that_cannot_be_written_fails_the_node = failed?.message ?? "";

  const after = await raised(() => attempt(execution, "review/1", { verdict: "approve" }));
  results.a_failed_mark_is_raised_once = after === undefined;
  results.the_attempt_after_it_still_records = order;

  journal.closeSession(execution);
}

await journal.releaseJournal();
process.stdout.write(`${JSON.stringify(results, null, 2)}\n`);
