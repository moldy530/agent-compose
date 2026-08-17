// The JavaScript column of the property conformance harness.
//
// `tests/property_conformance.rs` generates compositions, documents and guard
// expressions from a seed, answers them with the Rust column, and hands them
// here to be answered again by the emitted one. What comes back is verdicts, in
// the order the cases were given; the comparison is the Rust side's.
//
// Nothing is generated here. A runner that invented its own cases would be
// testing itself against the other column's cases, which is not the same
// experiment.
//
// Usage: node property-conformance.mjs <generated project> <cases.json>
//
// cases.json:
//   {
//     "schemas":     [ { "export": "stateA", "documents": ["<json text>", …] }, … ],
//     "expressions": [ { "source": "<CEL>", "roots": { "state": { "value": …, "shape": … } } }, … ]
//   }
//
// stdout:
//   { "schemas": [[true, false, …], …], "expressions": [{"value": true} | {"error": "…"}, …] }

import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, casesPath] = process.argv;
if (project === undefined || casesPath === undefined) {
  throw new Error("usage: node property-conformance.mjs <generated project> <cases.json>");
}

const at = (relative) => pathToFileURL(path.resolve(project, relative)).href;
const schemas = await import(at("src/schemas.ts"));
const cel = await import(at("src/cel.ts"));

const cases = JSON.parse(readFileSync(casesPath, "utf8"));

const schemaVerdicts = (cases.schemas ?? []).map((entry) => {
  const schema = schemas[entry.export];
  if (schema === undefined) {
    throw new Error(`the emitted project declares no \`${entry.export}\``);
  }
  return entry.documents.map((text) => schema.safeParse(JSON.parse(text)).success);
});

const expressionVerdicts = (cases.expressions ?? []).map((entry) => {
  const roots = {};
  for (const [name, root] of Object.entries(entry.roots)) {
    roots[name] = cel.bind(root.value, root.shape);
  }
  try {
    const answer = cel.evaluate(entry.source, roots);
    return typeof answer === "boolean" ? { value: answer } : { error: `not a bool: ${answer}` };
  } catch (error) {
    return { error: String(error) };
  }
});

process.stdout.write(JSON.stringify({ schemas: schemaVerdicts, expressions: expressionVerdicts }));
