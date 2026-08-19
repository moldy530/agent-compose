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
// A run that fails writes its trace too: `runFlow` raises a `FlowFailure`
// carrying every routing decision the run made before it stopped (PRD 5.3), and
// a driver that dropped it would make a failed run's trace unreadable from a
// test — the runs whose trace a test most wants to assert about.
//
// The session identity is passed too, because `runFlow` refuses a run whose flow
// reaches a `scope: session` store without one (grammar 11.3) — the same rule
// `agent-compose run --session` satisfies. A constant is enough here: what these
// tests assert about a session-scoped store is that a write survives the
// execution, which needs an identity rather than a particular one.
//
// Usage: node invoke-flow.mjs <project> <flow> <inputs.json> <trace.json> [session]
// Output: the flow's outputs as one JSON object on stdout; the trace is written
// to <trace.json>, so stdout stays exactly what `agent-compose run` prints.
//
// What lands in <trace.json> is the bare array of entries — `FlowRun.trace`, as
// an in-process caller of `runFlow` receives it — and deliberately not the
// versioned envelope `agent-compose run` writes to the project's data directory
// (`docs/trace.md`). This file is the harness's own scratch output rather than a
// delivery surface of the compiler, and a driver that wrapped it would be a
// second, unversioned imitation of the one the product defines. `Run::trace`
// reads the real file; `Invocation::trace` reads this one.

import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, flow, inputsPath, tracePath, session] = process.argv;
if (project === undefined || flow === undefined || inputsPath === undefined) {
  throw new Error("usage: node invoke-flow.mjs <project> <flow> <inputs.json> [trace.json]");
}

const inputs = JSON.parse(readFileSync(inputsPath, "utf8"));

// The host's preamble: whatever a host does before the composition loads, from
// a module of its own. Two things in this suite — registering the implementation
// of a grammar 6.1 `function:` binding, without which a project using the escape
// hatch does not run at all, and redirecting egress for a composition whose
// provider does not parameterise its `base_url:`. A test that wants the
// *unregistered* failure simply does not write this file.
const host = path.resolve(project, "host-functions.mjs");
if (existsSync(host)) await import(pathToFileURL(host).href);

const { runFlow } = await import(pathToFileURL(path.resolve(project, "src/index.ts")).href);

let run;
try {
  run = await runFlow(flow, inputs, session === undefined ? {} : { sessionKey: session });
} catch (error) {
  if (tracePath !== undefined) {
    const trace = Array.isArray(error?.trace) ? error.trace : [];
    writeFileSync(tracePath, JSON.stringify(trace, null, 1));
  }
  // The message and its cause chain, which is what a failing test reads.
  process.stderr.write(`${error?.stack ?? String(error)}\n`);
  for (let cause = error?.cause; cause !== undefined; cause = cause?.cause) {
    process.stderr.write(`  cause: ${cause?.stack ?? String(cause)}\n`);
  }
  process.exit(1);
}

if (tracePath !== undefined) writeFileSync(tracePath, JSON.stringify(run.trace, null, 1));
process.stdout.write(JSON.stringify(run.outputs));
