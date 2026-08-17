// Folds contributions into a generated project's `$run` channel and reports
// what comes out.
//
// The compiler's own state channel is where the runtime keeps what grammar
// requires in graph state and a composition never declares — the per-bounded-edge
// counters of 7.4, the traversal ordinals of 9.4, the step number of 7.6, and the
// routing trace PRD 5.3 asks for. Its reducer is the one place a *concurrent*
// step's contributions meet, so what it does with them out of order is the
// question: grammar 7.6.4 fixes a canonical order, and a reducer that kept
// arrival order instead would make a replay a plausible alternative to the live
// run rather than a reproduction of it.
//
// Usage: node run-channel.mjs <generated project directory> <contributions.json>
//   contributions.json: [ <Partial<RunChannel>>, … ], applied in the order given
// Output: the folded channel, as JSON.

import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, contributionsPath] = process.argv;
if (project === undefined || contributionsPath === undefined) {
  throw new Error("usage: node run-channel.mjs <project directory> <contributions.json>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);
const contributions = JSON.parse(readFileSync(contributionsPath, "utf8"));

let folded = runtime.emptyRun();
for (const contribution of contributions) {
  folded = runtime.mergeRun(folded, contribution);
}
process.stdout.write(JSON.stringify(folded));
