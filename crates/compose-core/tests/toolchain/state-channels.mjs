// Constructs a generated project's graph under the pinned LangGraph release and
// reports the channels its state model declares.
//
// Type-checking a state model says it is well typed; constructing it says
// LangGraph accepts it, which is the stronger claim CLAUDE.md's *Generated-code
// checks* asks for. The channel names go back to the Rust side, which compares
// them against the composition's own `state:` section — so a channel silently
// dropped between the IR and the emitted module fails here rather than at the
// first run.
//
// Usage: node state-channels.mjs <generated project directory>

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , project] = process.argv;
if (project === undefined) {
  throw new Error("usage: node state-channels.mjs <generated project directory>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;

const graph = await import(at("src/graph.ts"));
const state = await import(at("src/state.ts"));

const builder = graph.createBuilder();
if (builder === undefined || builder === null) {
  throw new Error("`createBuilder()` answered nothing");
}

process.stdout.write(JSON.stringify(Object.keys(state.channels)));
