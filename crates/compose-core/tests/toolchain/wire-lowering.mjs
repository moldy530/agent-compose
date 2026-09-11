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
// Three properties are checked *here* rather than reported, because each is
// about the function rather than about any one schema and none survives the JSON
// round trip out of this process:
//
//   * **determinism** — the same schema and table project to the same JSON, read
//     over two projections of two independently parsed copies of the case;
//   * **key order** — the half of byte-identity the round trip actually loses.
//     The gate compares the two schema columns as `serde_json` values, whose
//     objects are sorted maps, so a projection that rebuilt its nodes in some
//     other order (descriptions first, say, or keys sorted) would change the
//     bytes of every request ever sent and every structural assertion over there
//     would stay green. What is read instead is the property `loweredSchema`
//     claims: a lowered node's keys are the source node's keys, in the source's
//     order, minus whatever came off — with a folded `description` appended at
//     the end where the node had none. Stated as a *subsequence* so it needs no
//     copy of the table and reads a `properties` map, whose keys are field names
//     and one of which may be spelled like a keyword, by the very same rule;
//   * **purity** — the input is not touched, which is load bearing: the emitted
//     schema is what the emitted Zod parses and what the journal replays, so a
//     projection that rewrote it in place would change the contract on its way
//     to describing it.
//
// Usage: node wire-lowering.mjs <generated project directory> <cases.json>
// Output: one JSON object — { tables, lowered, stable, ordered, disorder, pure }
// — on stdout.

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

/** An object a schema node can be, as against an array, a string or a number. */
const plain = (value) => typeof value === "object" && value !== null && !Array.isArray(value);

/** Whether `keys` appear in `order`, in that order, with anything in between. */
function subsequence(keys, order) {
  let at = 0;
  for (const key of keys) {
    at = order.indexOf(key, at);
    if (at === -1) return false;
    at += 1;
  }
  return true;
}

/**
 * Every node whose lowered keys are not its source keys in the source's order —
 * empty when the projection kept them all, which is what byte-identity means
 * here. The one key allowed to be new is a folded `description`, at the end.
 */
function disordered(source, lowered, where) {
  const found = [];
  if (plain(source) && plain(lowered)) {
    const order = Object.keys(source);
    const keys = Object.keys(lowered);
    const last = keys[keys.length - 1];
    const folded = last === "description" && !Object.hasOwn(source, "description");
    if (!subsequence(folded ? keys.slice(0, -1) : keys, order)) {
      found.push({ path: where, source: order, lowered: keys });
    }
    for (const [key, value] of Object.entries(lowered)) {
      if (!Object.hasOwn(source, key)) continue;
      found.push(...disordered(source[key], value, where === "" ? key : `${where}.${key}`));
    }
  } else if (Array.isArray(source) && Array.isArray(lowered) && source.length === lowered.length) {
    for (let at = 0; at < lowered.length; at += 1) {
      found.push(...disordered(source[at], lowered[at], `${where}[${at}]`));
    }
  }
  return found;
}

const lowered = [];
const disorder = [];
let stable = true;
let pure = true;
for (const testcase of input.cases) {
  const before = JSON.stringify(testcase.schema);
  for (const [wire, mechanism] of pairs) {
    const schema = JSON.parse(before);
    const once = runtime.loweredSchema(schema, wire, mechanism);
    // A second projection of a second parse of the same text, so the comparison
    // spans two object graphs rather than one.
    const twice = runtime.loweredSchema(JSON.parse(before), wire, mechanism);
    if (JSON.stringify(once) !== JSON.stringify(twice)) stable = false;
    if (JSON.stringify(schema) !== before) pure = false;
    for (const node of disordered(schema, once, "")) {
      disorder.push({ label: testcase.label, wire, mechanism, ...node });
    }
    lowered.push({ label: testcase.label, wire, mechanism, schema: once });
  }
}

process.stdout.write(
  JSON.stringify({ tables, lowered, stable, ordered: disorder.length === 0, disorder, pure }),
);
