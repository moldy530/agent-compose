// Runs one SQL script against a built project's journal file.
//
// The acceptance suite's only writer of a journal it did not produce by running
// something (see `harness::journal_sql`). It exists for one claim:
// `docs/durability.md` §11.2 says a journal written **before** the delivery
// ledger existed opens unchanged under a build that has one. There is no earlier
// build in the tree to write such a file, so a test makes one — it runs the real
// project, then drops the `deliveries` table and rebuilds `executions` without
// its `callback` column — and restarts `serve` over the result.
//
// The driver is copied *into* the project before it is run, so the bare
// `node-sqlite3-wasm` specifier below resolves exactly as the emitted
// `src/journal.ts` resolves it: the same pinned driver, the same virtual file
// system, the same file locking. A second SQLite reached from this repository
// would be a different implementation answering a question about this one —
// and, under `--query`, one contending with a live `serve` for a file whose
// locking it did not share.
//
// It reads as well as writes, under `--query`: what a delivery row *says* is
// the only place some claims can be read from — a build that does not declare an
// execution's trigger journals the row and refuses to serve that execution's
// status route, so the journal is the surface, exactly as it is for a reader
// debugging one.
//
// # The lock a killed app leaves behind
//
// Sharing the emitted module's locking means sharing its **stale-lock rule**,
// and this is the half that is easy to leave out. The driver's lock is a
// `mkdir` — `<journal>.lock` — which an `rmdir` gives back and which a process
// killed inside a write never reaches; nothing else ever removes it. So a lock
// outlives the process that took it, and every open of that journal afterwards
// answers `database is locked` for ever rather than for a moment.
//
// That is not a hypothetical here: the tests that use this file are the ones
// about a *crash*, and the way they make one is `harness::Served`'s `SIGKILL`
// to the process group. Waiting longer cannot help — there is no owner left to
// give it back — so this does exactly what `src/journal.ts`'s `migrated` does
// under the same conditions: waits `LOCK_WAIT_MS` for a lock a live writer
// might be holding for a statement, and then, once, treats what is still there
// as the corpse it is and removes it. Sound under the same rule the emitted
// module names: one process at a time writes a project's journal, every write
// here is a single statement, and the harness has already waited for the killed
// app's process group to be empty before it calls this.
//
// Usage: bun journal-sql.mjs <journal.sqlite> <script.sql> [--query]

import { existsSync, readFileSync, rmSync } from "node:fs";
import process from "node:process";

const [, , journal, script, mode] = process.argv;
if (journal === undefined || script === undefined) {
  process.stderr.write("usage: journal-sql.mjs <journal.sqlite> <script.sql> [--query]\n");
  process.exit(2);
}

// The shape `src/journal.ts` loads it in, including the interop dance: the
// package publishes its `Database` under `default` in one runtime and at the top
// level in another.
const loaded = await import("node-sqlite3-wasm");
const { Database } = loaded.default ?? loaded;

/** `src/journal.ts`'s own `LOCK_WAIT_MS`, so this is as patient as the writer. */
const LOCK_WAIT_MS = 5_000;

/** Remove a lock no live process is giving back (`src/journal.ts`'s `breakStaleLock`). */
function breakStaleLock() {
  const lock = `${journal}.lock`;
  if (!existsSync(lock)) return false;
  try {
    rmSync(lock, { recursive: true, force: true });
    return true;
  } catch {
    return false;
  }
}

const sql = readFileSync(script, "utf8");
const deadline = Date.now() + LOCK_WAIT_MS;
let broke = false;
let delay = 10;
for (;;) {
  const database = new Database(journal);
  try {
    // **First**, exactly as `src/journal.ts` does it and for its reason: a
    // `--query` runs against a journal a live `serve` is still writing, and a
    // statement that arrives while that process holds the file fails outright
    // unless this has already told SQLite to wait.
    database.exec(`PRAGMA busy_timeout = ${LOCK_WAIT_MS};`);
    const answered = mode === "--query" ? `${JSON.stringify(database.all(sql))}\n` : undefined;
    if (answered === undefined) database.exec(sql);
    database.close();
    // Written after the close so a statement that threw cannot have half of its
    // answer on stdout ahead of the retry that replaces it.
    if (answered !== undefined) process.stdout.write(answered);
    break;
  } catch (error) {
    database.close();
    if (Date.now() >= deadline) {
      if (broke || !breakStaleLock()) throw error;
      broke = true;
      continue;
    }
    await new Promise((resolve) => setTimeout(resolve, delay));
    delay = Math.min(delay * 2, 200);
  }
}
