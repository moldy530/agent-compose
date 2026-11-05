// Runs the shared cron conformance corpus against the evaluator a generated
// project embeds (grammar 13.4, PRD resolved q78, Decision D162).
//
// The cron grammar has two readers, exactly as CEL does: `validate` holds a
// `schedule` trigger's `cron:` to `compose_core::cron`'s grammar, and `serve`
// evaluates it with the emitted `src/cron.ts`. One corpus, run against both, is
// what keeps them one reading — `crates/compose-core/tests/cron_conformance.rs`
// is the Rust column, and this is the other, over the same files with no port
// of anything. It imports only `src/cron.ts`, which has no imports of its own,
// so neither runtime needs an installed dependency set; timezones are the
// engine's own `Intl`, which is why both runtimes answer it.
//
// Usage: <bun|node> cron-conformance.mjs <generated project directory> <corpus directory>
// Output: a JSON array of divergences, empty when the two agree.

import { readdirSync, readFileSync } from "node:fs";
import path from "node:path";
import process from "node:process";
import { pathToFileURL } from "node:url";

const [, , project, corpus] = process.argv;
if (project === undefined || corpus === undefined) {
  throw new Error("usage: cron-conformance.mjs <project directory> <corpus directory>");
}

const cron = await import(pathToFileURL(path.resolve(project, "src/cron.ts")).href);

/** An instant, as the corpus writes one: UTC, to the second, `Z`-suffixed. */
function utc(instant) {
  return new Date(instant).toISOString().replace(/\.\d{3}Z$/, "Z");
}

const divergences = [];
for (const file of readdirSync(corpus).filter((name) => name.endsWith(".json")).sort()) {
  const cases = JSON.parse(readFileSync(path.join(corpus, file), "utf8"));
  for (const entry of cases) {
    const where = `${file}: ${entry.name}`;
    let parsed;
    try {
      parsed = cron.parseCron(entry.cron);
    } catch (error) {
      if (entry.error === undefined) {
        divergences.push({ case: where, expected: "an expression that parses", found: String(error.message) });
      } else if (error.message !== entry.error) {
        divergences.push({ case: where, expected: entry.error, found: error.message });
      }
      continue;
    }
    if (entry.error !== undefined) {
      divergences.push({ case: where, expected: entry.error, found: "an expression that parses" });
      continue;
    }
    const zone = cron.zoneOf(entry.timezone);
    let at = Date.parse(entry.from);
    const next = [];
    const scheduled = [];
    for (let index = 0; index < entry.next.length; index += 1) {
      const fires = cron.nextAfter(parsed, at, zone);
      if (fires === undefined) break;
      next.push(utc(fires));
      scheduled.push(cron.render(fires, zone));
      at = fires;
    }
    if (JSON.stringify(next) !== JSON.stringify(entry.next)) {
      divergences.push({ case: where, expected: entry.next, found: next });
    }
    if (JSON.stringify(scheduled) !== JSON.stringify(entry.scheduled_at)) {
      divergences.push({ case: where, expected: entry.scheduled_at, found: scheduled });
    }
  }
}
process.stdout.write(JSON.stringify(divergences));
