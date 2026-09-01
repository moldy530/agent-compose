// Runs the shared OTLP conformance corpus against the exporter a generated
// project embeds.
//
// PRD resolved q51 trades an OpenTelemetry SDK for a **conformance fixture
// corpus** — "the CEL-corpus discipline applied to span output" — and this is the
// half of that corpus which executes. `crates/compose-core/tests/otlp_conformance.rs`
// is the driver; `tests/fixtures/otlp-conformance/` is the data. It lives in the
// shared toolchain fixture beside `cel-conformance.mjs`, and for the same reason:
// the module under test is emitted TypeScript, so the only honest way to answer
// the corpus is to import the emitted module and call it.
//
// It imports `src/otlp.ts`, whose one runtime import is `node:crypto` — the trace
// types it also names arrive through `import type`, which erases — so this run
// needs no installed dependency set.
//
// Usage: <bun|node> otlp-conformance.mjs <generated project directory> <corpus directory> [--write]
// Output: a JSON array of divergences, empty when the exporter answers the corpus.
//
// `--write` rewrites each fixture's `expected` from what the exporter produced.
// It is the `UPDATE_GOLDENS=1` posture the generated projects already have: the
// expectations are *bytes*, and bytes are reviewed in a diff rather than typed by
// hand. What keeps that from being a test that agrees with itself is the driver:
// `otlp_conformance.rs` asserts the structural invariants `docs/trace.md` §12
// states — hex-shaped ids, a parent for every span but the root, one span per
// record, the documented resource attributes — over whatever the corpus holds,
// so a rewritten expectation that broke one of them fails there.

import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, corpus, ...flags] = process.argv;
if (project === undefined || corpus === undefined) {
  throw new Error("usage: otlp-conformance.mjs <project directory> <corpus directory> [--write]");
}
const writing = flags.includes("--write");

const otlp = await import(pathToFileURL(path.resolve(project, "src/otlp.ts")).href);

const files = readdirSync(corpus)
  .filter((name) => name.endsWith(".json"))
  .sort();

const divergences = [];
const seen = new Set();

for (const file of files) {
  const at = path.join(corpus, file);
  const fixture = JSON.parse(readFileSync(at, "utf8"));
  const name = fixture.name;
  if (typeof name !== "string" || name.length === 0) {
    divergences.push({ file, kind: "no-name" });
    continue;
  }
  if (seen.has(name)) {
    divergences.push({ file, name, kind: "duplicate-name" });
    continue;
  }
  seen.add(name);
  if (fixture.document === undefined || fixture.context === undefined) {
    divergences.push({ file, name, kind: "no-input" });
    continue;
  }

  let produced;
  try {
    produced = otlp.exportRequest(fixture.document, fixture.context);
  } catch (error) {
    divergences.push({ file, name, kind: "threw", detail: String(error) });
    continue;
  }

  // **Determinism, asked of the exporter rather than assumed of it.** The ids
  // are hashes and nothing reads a clock, so a second mapping of one input has
  // to be the same bytes — which is what makes a retried at-least-once delivery
  // one export rather than two views of one run (docs/trace.md §12.2).
  const again = otlp.exportRequest(fixture.document, fixture.context);
  if (JSON.stringify(again) !== JSON.stringify(produced)) {
    divergences.push({ file, name, kind: "not-deterministic" });
    continue;
  }

  if (writing) {
    fixture.expected = produced;
    writeFileSync(at, `${JSON.stringify(fixture, null, 2)}\n`);
    continue;
  }

  if (fixture.expected === undefined) {
    divergences.push({ file, name, kind: "no-expectation" });
    continue;
  }
  const wanted = JSON.stringify(fixture.expected);
  const got = JSON.stringify(produced);
  if (wanted !== got) {
    divergences.push({ file, name, kind: "mismatch", expected: fixture.expected, produced });
  }
}

process.stdout.write(`${JSON.stringify(divergences)}\n`);
