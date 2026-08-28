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
// system, the same locking (none). A second SQLite reached from this repository
// would be a different implementation answering a question about this one.
//
// It reads as well as writes, under `--query`: what a delivery row *says* is
// the only place some claims can be read from — a build that does not declare an
// execution's trigger journals the row and refuses to serve that execution's
// status route, so the journal is the surface, exactly as it is for a reader
// debugging one.
//
// Usage: bun journal-sql.mjs <journal.sqlite> <script.sql> [--query]

import { readFileSync } from "node:fs";
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

const database = new Database(journal);
try {
  const sql = readFileSync(script, "utf8");
  if (mode === "--query") {
    process.stdout.write(`${JSON.stringify(database.all(sql))}\n`);
  } else {
    database.exec(sql);
  }
} finally {
  database.close();
}
