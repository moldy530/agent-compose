// Runs one flow of a built project and reports what it produced.
//
// The acceptance suite's invocation half (see `harness.rs`): it imports the
// emitted project's own `runFlow` — the surface `agent-compose run` and the
// generated `serve` app are built on (PRD 5.11's `start`) — rather than
// reimplementing anything about how a graph is invoked. What a test then asserts
// is the flow's `outputs:` and the routing trace the run recorded (PRD 5.3).
//
// Importing `./src/index.ts` rather than `./src/graph.ts` is deliberate: the
// barrel is where `readEnvironment()` runs at module scope, so a run started
// with a variable missing fails at process start naming it, exactly as a real
// invocation would (PRD 5.9).
//
// Usage: node invoke-flow.mjs <project> <flow> <inputs.json> <trace.json>
// Output: the flow's outputs as one JSON object on stdout; the trace is written
// to <trace.json>, so stdout stays exactly what `agent-compose run` prints.

import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, flow, inputsPath, tracePath] = process.argv;
if (project === undefined || flow === undefined || inputsPath === undefined) {
  throw new Error("usage: node invoke-flow.mjs <project> <flow> <inputs.json> [trace.json]");
}

const inputs = JSON.parse(readFileSync(inputsPath, "utf8"));
const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);

let run;
try {
  run = await runFlow(flow, inputs);
} catch (error) {
  if (tracePath !== undefined) writeFileSync(tracePath, "[]");
  // The message and its cause chain, which is what a failing test reads.
  process.stderr.write(`${error?.stack ?? String(error)}\n`);
  for (let cause = error?.cause; cause !== undefined; cause = cause?.cause) {
    process.stderr.write(`  cause: ${cause?.stack ?? String(cause)}\n`);
  }
  process.exit(1);
}

if (tracePath !== undefined) writeFileSync(tracePath, JSON.stringify(run.trace, null, 1));
process.stdout.write(JSON.stringify(run.outputs));
