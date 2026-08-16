// Runs the schema-lowering corpus against the *emitted Zod*, and reports one
// verdict per document.
//
// The Rust side runs the same documents against the JSON Schema its own lowering
// produces (grammar 3.8's middle column) and compares. Neither side is trusted
// to be right on its own: the corpus also carries the expected verdict, so two
// implementations agreeing on a wrong answer is still a failure.
//
// Usage: node zod-conformance.mjs <cases.json> <generated project directory>
//
// Input:  [{ "export": "agentReviewerOutput", "documents": [ … ] }, … ]
// Output: [[true, false, …], …] — one array of verdicts per case, in order.

import { readFile } from "node:fs/promises";
import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

const [, , casesPath, project] = process.argv;
if (casesPath === undefined || project === undefined) {
  throw new Error("usage: node zod-conformance.mjs <cases.json> <generated project directory>");
}

const cases = JSON.parse(await readFile(casesPath, "utf8"));
const schemas = await import(pathToFileURL(path.resolve(project, "src/schemas.ts")).href);

const verdicts = cases.map((entry) => {
  const schema = schemas[entry.export];
  if (schema === undefined) {
    throw new Error(`the emitted module exports no \`${entry.export}\``);
  }
  return entry.documents.map((document) => schema.safeParse(document).success);
});

process.stdout.write(JSON.stringify(verdicts));
