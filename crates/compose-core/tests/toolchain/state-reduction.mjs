// Runs a generated project's state model and reports what it holds afterwards.
//
// Constructing a `StateGraph` says LangGraph accepted the channel specs
// (`state-channels.mjs`); it says nothing about what they *do*. A wrong reducer
// and a missing initial value both type-check, both construct, and both would be
// locked in as a correct golden. So this one invokes: two nodes write in
// sequence, and the state at quiescence goes back to the Rust side, which
// compares it against what the composition's `reduce:` policies and `default:`s
// say it should be (grammar 7.6.4, 10.1, 10.2).
//
// Two writes rather than one, because most of the policies are only visible on
// the second: `append` differs from "assign" only when there is something to
// append to, and `merge` differs from "replace" only when two writes carry
// different keys.
//
// Usage: node state-reduction.mjs <generated project directory> <writes.json>
//   writes.json: [ { "<channel>": <update>, … }, … ] — one object per node, in
//   the order the nodes run.
// Output: the state at quiescence, as JSON.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { StateGraph } from "@langchain/langgraph";

const [, , project, writesPath] = process.argv;
if (project === undefined || writesPath === undefined) {
  throw new Error("usage: node state-reduction.mjs <generated project directory> <writes.json>");
}

const writes = JSON.parse(await readFile(writesPath, "utf8"));
const state = await import(pathToFileURL(path.resolve(project, "src/state.ts")).href);

let builder = new StateGraph(state.State);
let previous = "__start__";
writes.forEach((update, index) => {
  const id = `write_${index}`;
  builder = builder.addNode(id, () => update).addEdge(previous, id);
  previous = id;
});
builder = builder.addEdge(previous, "__end__");

const answer = await builder.compile().invoke({});
process.stdout.write(JSON.stringify(answer));
