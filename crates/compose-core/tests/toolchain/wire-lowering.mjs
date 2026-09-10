// What a compiled graph puts on each wire, projected by the generated runtime's
// own lowering tables (PRD §9 resolved q55).
//
// `src/runtime.ts` is a compiler constant — byte-identical in every project —
// so driving its projection directly is driving what every generated graph
// sends. Two questions are answered here and the expectations for both live in
// the Rust gate that reads this (`generated_code_gates.rs`), which is the same
// division `map-dispatch.mjs` and `raw-decoding.mjs` keep: this script observes,
// the gate decides.
//
// 1. **The tables**, verbatim, so the gate measures the delta between the two
//    schema columns against the table the composers actually project through
//    rather than against a copy of it.
// 2. **The projection**, per (wire, mechanism), over every schema the gate hands
//    in — the goldens' own agent `output:` surfaces, and a corpus of shapes
//    written to reach the corners: nested objects, arrays of objects, `anyOf`
//    branches, a node whose bounds pair up, and a node with no description of
//    its own.
//
// Two properties are checked *here* rather than reported, because both are about
// the function rather than about any one schema and neither survives a JSON
// round trip:
//
//   * **determinism** — the same schema and table project to byte-identical
//     JSON, which is what lets a wire schema be compared as text at all;
//   * **purity** — the input is not touched, which is load bearing: the emitted
//     schema is what the emitted Zod parses and what the journal replays, so a
//     projection that rewrote it in place would change the contract on its way
//     to describing it.
//
// Usage: node wire-lowering.mjs <generated project directory> <cases.json>
// Output: one JSON object — { tables, lowered, stable, pure } — on stdout.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, casesPath] = process.argv;
if (project === undefined || casesPath === undefined) {
  throw new Error("usage: node wire-lowering.mjs <generated project directory> <cases.json>");
}

const runtime = await import(pathToFileURL(path.resolve(project, "src/runtime.ts")).href);

// JSON *text* rather than values, so the key order a case was written in
// survives into the projection — a lowered schema is compared as bytes.
const input = JSON.parse(fs.readFileSync(casesPath, "utf8"));
const pairs = input.pairs;

const tables = {};
for (const [wire, mechanism] of pairs) {
  tables[wire] ??= {};
  tables[wire][mechanism] = runtime.loweredAway(wire, mechanism);
}

const lowered = [];
let stable = true;
let pure = true;
for (const testcase of input.cases) {
  const before = JSON.stringify(testcase.schema);
  for (const [wire, mechanism] of pairs) {
    const schema = JSON.parse(before);
    const once = runtime.loweredSchema(schema, wire, mechanism);
    const twice = runtime.loweredSchema(schema, wire, mechanism);
    if (JSON.stringify(once) !== JSON.stringify(twice)) stable = false;
    if (JSON.stringify(schema) !== before) pure = false;
    lowered.push({ label: testcase.label, wire, mechanism, schema: once });
  }
}

process.stdout.write(JSON.stringify({ tables, lowered, stable, pure }));
