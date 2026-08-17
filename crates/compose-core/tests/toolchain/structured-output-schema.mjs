// Converts named exports of an emitted `src/schemas.ts` the way PRD 5.2's
// delivery mechanism would, and prints the result.
//
// `withStructuredOutput` does not send a Zod schema anywhere: `@langchain/core`
// converts it to JSON Schema first, and that conversion is what a provider is
// actually handed. This runs the library's own converter — not a re-implementation
// of it — so the Rust side can compare what would reach a model against what this
// compiler lowers the same surface to.
//
// Usage: node structured-output-schema.mjs <generated project directory> <export> …
//
// Output: { "<export>": <json schema>, … }

import { pathToFileURL } from "node:url";
import path from "node:path";
import process from "node:process";

import { toJsonSchema } from "@langchain/core/utils/json_schema";

const [, , project, ...exports] = process.argv;
if (project === undefined || exports.length === 0) {
  throw new Error("usage: node structured-output-schema.mjs <generated project directory> <export> …");
}

const schemas = await import(pathToFileURL(path.resolve(project, "src/schemas.ts")).href);

const converted = Object.fromEntries(
  exports.map((name) => {
    const schema = schemas[name];
    if (schema === undefined) {
      throw new Error(`the emitted module exports no \`${name}\``);
    }
    return [name, toJsonSchema(schema)];
  }),
);

process.stdout.write(JSON.stringify(converted));
