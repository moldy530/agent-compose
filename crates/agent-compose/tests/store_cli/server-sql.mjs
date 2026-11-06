// Runs one SQL statement against the server a dialled store is bound to, and
// prints the rows it answered as JSON.
//
// The store command's suite has two claims only a server can answer: that an
// operator's write left **no ledger row** — `store_applied` is where a keyed
// write's idempotency entry lands (PRD resolved q63), and an operator's write
// is not keyed (PRD resolved q72) — and that a case leaves nothing behind it,
// which is the suite deleting the rows its run-unique stores wrote. Neither is
// a question the command itself answers, by design, so the suite asks the
// server directly.
//
// It is copied into the built project before it is run, so `pg` and `mysql2`
// resolve exactly as the emitted `src/stores.ts` resolves them: the pinned
// drivers, from the install the project is built beneath. The address is read
// from the variable the project's deploy file names, never from an argument.
//
// Usage: bun server-sql.mjs <postgres|mysql> <variable> <sql> [<parameters as JSON>]
//
// `?` placeholders on both arms; the Postgres arm numbers them, as
// `src/stores.ts` does.

import process from "node:process";

const [, , provider, variable, sql, parameters] = process.argv;
if (provider === undefined || variable === undefined || sql === undefined) {
  process.stderr.write("usage: server-sql.mjs <postgres|mysql> <variable> <sql> [<parameters>]\n");
  process.exit(2);
}
const address = process.env[variable];
if (address === undefined || address === "") {
  process.stderr.write(`\`${variable}\` is not set\n`);
  process.exit(2);
}
const bound = parameters === undefined ? [] : JSON.parse(parameters);

let rows;
if (provider === "postgres") {
  const loaded = await import("pg");
  const { Client } = loaded.default ?? loaded;
  const client = new Client({ connectionString: address });
  await client.connect();
  try {
    let at = 0;
    const numbered = sql.replace(/\?/g, () => `$${(at += 1)}`);
    rows = (await client.query(numbered, bound)).rows;
  } finally {
    await client.end();
  }
} else if (provider === "mysql") {
  const loaded = await import("mysql2/promise");
  const mysql = loaded.default ?? loaded;
  const connection = await mysql.createConnection(address);
  try {
    await connection.query("SET SESSION sql_mode = CONCAT(@@sql_mode, ',ANSI_QUOTES')");
    const [answered] = await connection.query(sql, bound);
    rows = Array.isArray(answered) ? answered : [];
  } finally {
    await connection.end();
  }
} else {
  process.stderr.write(`\`${provider}\` is not a dialled store provider\n`);
  process.exit(2);
}
process.stdout.write(`${JSON.stringify(rows)}\n`);
