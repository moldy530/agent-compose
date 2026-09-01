// Runs the shared `traceparent` corpus against the parser a generated project
// embeds.
//
// The sibling of `otlp-conformance.mjs`, for the half of `src/otlp.ts` that
// reads rather than writes. PRD resolved q51's first amendment makes an inbound
// W3C `traceparent` the exported root span's parent, so what that header is
// *allowed to be* is a wire contract: a parser that accepted a reserved version
// would publish spans under a trace id a collector is entitled to drop, and one
// that minted an all-zero id would publish spans nothing renders at all. Neither
// failure is visible from the export side — an ignored header and a refused one
// produce the same well-formed request — so the arms are asked here directly.
//
// It imports the emitted module for `otlp-conformance.mjs`'s reason: the parser
// under test is emitted TypeScript, and a port of it would be a second
// implementation answering a question about the first.
//
// Usage: <bun|node> traceparent-conformance.mjs <generated project directory> <corpus file>
// Output: a JSON array of divergences, empty when the parser answers the corpus.

import { readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, corpus] = process.argv;
if (project === undefined || corpus === undefined) {
  throw new Error("usage: traceparent-conformance.mjs <project directory> <corpus file>");
}

const otlp = await import(pathToFileURL(path.resolve(project, "src/otlp.ts")).href);

const cases = JSON.parse(readFileSync(corpus, "utf8"));
if (!Array.isArray(cases)) {
  throw new Error("the traceparent corpus is a JSON array of cases");
}

const divergences = [];
const seen = new Set();

for (const held of cases) {
  const name = held.name;
  if (typeof name !== "string" || name.length === 0) {
    divergences.push({ kind: "no-name" });
    continue;
  }
  if (seen.has(name)) {
    divergences.push({ name, kind: "duplicate-name" });
    continue;
  }
  seen.add(name);

  let produced;
  try {
    // `header` absent in the fixture is the header being absent on the request,
    // which is its own refusal arm and the commonest one by far.
    produced = otlp.parseTraceparent(held.header);
  } catch (error) {
    divergences.push({ name, kind: "threw", detail: String(error) });
    continue;
  }
  const answered = produced === undefined ? null : produced;
  const wanted = held.expected === undefined ? null : held.expected;
  if (JSON.stringify(answered) !== JSON.stringify(wanted)) {
    divergences.push({ name, kind: "mismatch", expected: wanted, produced: answered });
    continue;
  }
  // A case that says which arm refuses it has to actually be refused, and one
  // that expects a parse has to not be: the two fields are read by
  // `otlp_conformance.rs` to prove every arm is reached, and a case filed under
  // the wrong one would prove it about nothing.
  const refusing = typeof held.refuses === "string";
  if (refusing !== (answered === null)) {
    divergences.push({ name, kind: "arm-mismatch", refuses: held.refuses ?? null, produced: answered });
  }
}

process.stdout.write(`${JSON.stringify(divergences)}\n`);
