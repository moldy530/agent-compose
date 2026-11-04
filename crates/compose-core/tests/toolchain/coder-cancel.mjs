// A `coder:` node **cancelled** mid-run (grammar 8.9, PRD resolved q75,
// Decision D160).
//
// The field report's case: a run inside a long coding-harness node, with no
// way to stop it short of killing the process. This runs the `patch-pipeline`
// golden's own `runFlow` with a harness driver that does what a real SDK does
// with its run — spawns a long-lived child process and stops it when the run's
// signal fires — and then cancels the execution from outside, the way the
// cancel route does (`runtime.cancelExecution`).
//
// What it reports is what the cancel owes: how long the run took to stop, that
// the child process the harness started is gone, that the run ended with the
// outcome `cancelled` rather than a failure, that the node's trace entry says
// `cancelled` with the harness record it made and no `error` and no `routing`,
// and that the journal's lifecycle row settled `cancelled` with no `error`.
//
// And a second run, for the cancel that arrives **before the run has a row**:
// `serve` answers a trigger's request while the lifecycle row's `begin` is
// still in flight — on a dialled journal, real network I/O — and a cancel sent
// at once lands inside it. The journal's `begin` is slowed here to make that
// window a fact rather than a race, and the cancel is sent the moment the run
// is started. It reports whether that run ended `cancelled` with no harness run
// begun, and how its row settled (`early*`).
//
// **The seam is `registerHarnessDriver`**, as `coder-graph.mjs` explains: a
// harness SDK speaks its own wire, so the driver is the one place a test can
// stand in for it. What the driver reads is `run.signal` — the node's own
// signal, which is chained to the execution's cancel — and nothing else.
//
// Usage: node coder-cancel.mjs <generated project directory> <scratch dir>
// Output: one JSON object, read by `generated_code_gates.rs`.

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, scratch] = process.argv;
if (project === undefined || scratch === undefined) {
  throw new Error("usage: node coder-cancel.mjs <generated project directory> <scratch dir>");
}

fs.mkdirSync(scratch, { recursive: true });
const workspace = path.join(scratch, "checkout");
fs.mkdirSync(workspace, { recursive: true });

// The composition's own `${ENV}` references, as `coder-graph.mjs` sets them.
process.env["REPO_ROOT"] = workspace;
process.env["ANTHROPIC_API_KEY"] = "harness-key-anthropic";
process.env["OPENAI_API_KEY"] = "harness-key-openai";
process.env["LLM_GATEWAY_URL"] = "https://gateway.invalid/v1";
process.env["TEAM_NAME"] = "platform";
process.env["AGENT_COMPOSE_DATA"] = path.join(scratch, "data");

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const harness = await import(pathToFileURL(path.resolve(project, "src/harness.ts")).href);
const journals = await import(pathToFileURL(path.resolve(project, "src/journal.ts")).href);

/** Resolved with the child's pid once the harness run has started it. */
let started;
const running = new Promise((resolve) => {
  started = resolve;
});

/** How many harness runs began — the second run's must be none. */
let implementerRuns = 0;

/** A harness that works for ten minutes, the way a long coding run does. */
const implementer = {
  sdk: "agent-compose:cancel-probe/cc",
  version: "0",
  enforcesTools: true,
  run(run) {
    implementerRuns += 1;
    // A run begun in an execution whose cancel was already requested is the
    // failure the second scenario looks for: it fails at once, so the
    // scenario ends rather than waiting out a ten-minute probe.
    if (implementerRuns > 1) {
      return (async function* refused() {
        throw new Error("a harness run began in an execution whose cancel was already requested");
      })();
    }
    return (async function* yielded() {
      yield {
        source: { type: "assistant" },
        tap: { kind: "turn", usage: { inputTokens: 120, outputTokens: 30 } },
      };
      // What an SDK does under its run: a process of its own, stopped when the
      // run's signal fires — the one thing the driver contract asks of it.
      const child = spawn("sleep", ["600"], { stdio: "ignore" });
      run.signal.addEventListener("abort", () => child.kill("SIGTERM"), { once: true });
      started(child.pid);
      await new Promise((_resolve, reject) => {
        if (run.signal.aborted) reject(run.signal.reason);
        run.signal.addEventListener("abort", () => reject(run.signal.reason), { once: true });
      });
    })();
  },
};
const reviewer = harness.scriptedDriver("codex", [
  { source: { type: "agent-compose.settled" }, tap: { kind: "settled", cost: {} } },
  {
    source: { type: "agent-compose.agent_message" },
    tap: { kind: "output", value: { verdict: "approve", feedback: "" } },
  },
]);
runtime.registerHarnessDriver("cc", implementer);
runtime.registerHarnessDriver("codex", reviewer.driver);

const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);

const ended = runFlow("flow.patch", { goal: "make the failing test pass" }).then(
  (value) => ({ value }),
  (error) => ({ error }),
);
const pid = await running;

const journal = await journals.openJournal();
const open = await journal.openExecutions();
const execution = open.find((row) => row.flow === "flow.patch")?.id;
if (execution === undefined) throw new Error("the run left no open lifecycle row to cancel");

const asked = Date.now();
await runtime.cancelExecution(execution);
const settled = await ended;
const took = Date.now() - asked;

/** Whether `pid` is still a process, for a moment after the run stopped. */
async function alive(held) {
  for (let attempt = 0; attempt < 50; attempt += 1) {
    try {
      process.kill(held, 0);
    } catch {
      return false;
    }
    await new Promise((resolve) => setTimeout(resolve, 20));
  }
  return true;
}

const row = await journal.execution(execution);
const trace = settled.error?.trace ?? settled.value?.trace ?? [];

// The second run: the cancel lands while the row is still being begun.
const begin = journal.begin.bind(journal);
journal.begin = async (begun) => {
  await new Promise((resolve) => setTimeout(resolve, 150));
  return begin(begun);
};
const earlyId = `exec_${globalThis.crypto.randomUUID()}`;
const earlyEnded = runFlow(
  "flow.patch",
  { goal: "make the failing test pass" },
  { executionId: earlyId },
).then(
  (value) => ({ value }),
  (error) => ({ error }),
);
await runtime.cancelExecution(earlyId);
const early = await earlyEnded;
journal.begin = begin;
const earlyRow = await journal.execution(earlyId);
const results = {
  completed: settled.value !== undefined,
  endedByCancel: settled.error !== undefined && runtime.endedByCancel(settled.error),
  tookMs: took,
  childAlive: await alive(pid),
  status: row?.status ?? null,
  rowError: row?.error ?? null,
  cancelRequested: typeof row?.cancelRequestedAt === "string",
  reviewRuns: reviewer.runs.length,
  earlyEndedByCancel: early.error !== undefined && runtime.endedByCancel(early.error),
  earlyHarnessRuns: implementerRuns - 1,
  earlyStatus: earlyRow?.status ?? null,
  earlyRowError: earlyRow?.error ?? null,
  entries: trace.map((entry) => ({
    node: entry.node,
    outcome: entry.outcome,
    hasError: entry.error !== undefined,
    hasRouting: entry.routing !== undefined,
    harness: (entry.harness ?? []).map((record) => ({ harness: record.harness, sdk: record.sdk })),
  })),
};
await journals.releaseJournal();
process.stdout.write(JSON.stringify(results, null, 1));
