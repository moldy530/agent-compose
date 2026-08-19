// Drives a generated project's terminal answer surface over the **real**
// `process.stdin`, and reports what it did (grammar 8.7, PRD 5.11).
//
// `interactive-pause.mjs` beside it drives the same loop over a `node:stream`
// `PassThrough`, which is what makes its eight sections schedulable — a pause
// parked at an exact moment, an expiry raced against a prompt. What it cannot
// ask is the question this file exists for: `src/cli.ts` reads
// `process.stdin`, and a process's own standard input is not a `PassThrough`.
// It is a pipe the operating system owns, wired into the runtime's event loop,
// and three of its properties are the **engine's** rather than the reader's:
//
//   * whether a `data` listener attached *late* — the loop attaches at the
//     first question, never before, so a run with no `human` node never drains
//     a stream it was not given — still sees the bytes that arrived first;
//   * whether `end` fires on a pipe whose EOF arrived **before** that listener
//     existed, which is a script that piped nothing into a flow that pauses;
//   * whether `Lines.stop()` — `removeListener` and `pause()` — lets go of the
//     handle, so the process **exits** instead of sitting on a released stream
//     with its answer already printed.
//
// The third is why this runner ends the way it does. A run that will not exit
// is the one failure shape a passing test cannot be told from a slow one, so it
// is not left to a harness timeout: the work is guarded by a timer while it
// runs, and once the loop has stopped the runner arms an **unref'd** timer and
// lets the event loop decide. Nothing holding standard input means nothing to
// keep the loop alive, and the process is gone before that timer can fire; a
// handle still attached keeps the loop alive, the timer fires on a live loop,
// and the runner exits non-zero saying which of the two happened.
//
// `src/cli.ts` and `src/runtime.ts` are compiler constants, byte-identical in
// every project this release builds, so driving them directly is driving what
// every project runs.
//
// Usage: node interactive-stdin.mjs <generated project directory> <case>
//   answers   one answer arrives on the pipe; the caller keeps it open
//   eof       the pipe is closed before the loop ever attaches to it
// Output: one JSON object of observations; the expectations live in the Rust
// test that reads it (`generated_code_gates.rs`).

import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, which] = process.argv;
if (project === undefined || which === undefined) {
  throw new Error("usage: node interactive-stdin.mjs <generated project directory> <case>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const cli = await import(pathToFileURL(path.resolve(project, "src/cli.ts")).href);

/** A sign-off's `output:`, as `interactive-pause.mjs` declares the same one. */
const SCHEMA = {
  type: "object",
  properties: { decision: { enum: ["approve", "reject"] } },
  required: ["decision"],
  additionalProperties: false,
};

function parse(payload) {
  if (typeof payload !== "object" || payload === null) {
    throw new Error("the answer is an object");
  }
  if (!["approve", "reject"].includes(payload.decision)) {
    throw new Error("`decision` is `approve` or `reject`");
  }
  return { decision: payload.decision };
}

/** A `NodeView` at `instancePath`, which is what a wait id is derived from. */
function viewAt(instancePath, id) {
  const run = {
    ...runtime.emptyRun(),
    execution: { id, session_key: "" },
    path: instancePath,
  };
  return { state: {}, run, roots: { input: runtime.bind({}, { properties: {} }) } };
}

/** What a parked task ended as: `"pending"`, `"resolved"`, or the error's name. */
function outcomeOf(promise) {
  const held = { state: "pending", value: undefined };
  promise.then(
    (value) => {
      held.state = "resolved";
      held.value = value;
    },
    (error) => {
      held.state = error?.name ?? "Error";
    },
  );
  return held;
}

/** Everything the prompt loop writes, kept off stdout so the report stays JSON. */
const written = [];
const output = {
  write(text) {
    written.push(String(text));
  },
};

// The guard over the work itself. Refd, because a runner that hung with nothing
// keeping the loop alive would exit `0` reporting nothing at all — and cleared
// before the exit check below, which needs the opposite.
const working = setTimeout(() => {
  process.stderr.write(
    `the prompt loop never finished the \`${which}\` case: it is holding a question nothing on ` +
      `standard input can answer\n`,
  );
  process.exit(9);
}, 20_000);

const execution = `exec_${which}`;
runtime.openHumanWaits(execution, true);
const context = { execution: { id: execution, session_key: "" }, node: "sign" };
const held = outcomeOf(
  runtime.runHuman(
    { flow: "flow.sign_off", node: "sign", schema: SCHEMA, parse },
    { question: "ship it?" },
    context,
    viewAt(["review", "0"], execution),
  ),
);
await new Promise((resolve) => setTimeout(resolve, 0));

let stop;
const finished = new Promise((resolve) => {
  stop = resolve;
});
const prompting = cli.answerPauses(execution, finished, { input: process.stdin, output });

/** Wait until something has happened, rather than for a number of milliseconds. */
async function until(ready) {
  while (!ready()) {
    await new Promise((resolve) => setTimeout(resolve, 5));
  }
}

if (which === "answers") {
  // The answer is on the pipe already, or arrives while this waits. The loop
  // goes on running after it — a run keeps asking until it ends — so what stops
  // it is the run being over, which is `finished`.
  await until(() => held.state !== "pending");
  stop();
} else if (which !== "eof") {
  throw new Error(`\`${which}\` is not a case of this runner: \`answers\` or \`eof\``);
}

// `eof` ends on its own: standard input ending is the surface going away, and
// the loop returns having closed the board.
await prompting;
await until(() => held.state !== "pending");
clearTimeout(working);
runtime.releaseHumanWaits(execution);

process.stdout.write(
  JSON.stringify({
    said: written.join(""),
    settled: held.state,
    output: held.value?.output,
    pause: held.value?.human?.settled,
  }),
);

// The exit check. Unref'd on purpose: if the prompt loop let go of standard
// input there is nothing left to keep the event loop alive, the process ends
// here, and this never runs. If it did not, the loop is alive, this fires on
// it, and the gate reads a code rather than waiting out a harness timeout on a
// run that printed its whole answer and then sat there.
const exiting = setTimeout(() => {
  process.stderr.write(
    "the process did not exit after the prompt loop stopped: something is still holding " +
      "standard input open\n",
  );
  process.exit(9);
}, 5_000);
exiting.unref?.();
