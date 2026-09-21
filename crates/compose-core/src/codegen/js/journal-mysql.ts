// ---------------------------------------------------------------------------
// The MySQL arm (grammar §14.7, PRD resolved q62)
// ---------------------------------------------------------------------------
//
// Emitted only into a project whose target binds `provider: mysql`, which is
// also the only project whose `package.json` pins `mysql2`. A build that binds
// SQLite carries neither the driver nor this code, which is what keeps the
// zero-infra guarantee a property of the artifact rather than of a code path
// nobody takes.
//
// Nothing here is a second implementation of anything. The statements are
// `SqlJournal`'s and the contract is `docs/durability.md`'s; what this file adds
// is a connection, a schema, and the writer guard.

import { createConnection } from "mysql2/promise";
import type { Connection, RowDataPacket } from "mysql2/promise";

/**
 * The schema, created on first open and never migrated.
 *
 * `IF NOT EXISTS` throughout, so a second open of a journal this build already
 * created does nothing — which is the whole of what `docs/durability.md` §11.2
 * asks of a physical schema, and all it can ask of this one: a remote journal is
 * created by this release or a later one, so there is no older database whose
 * columns have to be probed for. The indexes are declared **inside** each table
 * rather than beside it, because MySQL has no `CREATE INDEX IF NOT EXISTS` and a
 * second open must not be an error.
 *
 * **`CHARACTER SET ascii COLLATE ascii_bin` on every column a statement compares
 * or orders by**, and that is a correctness choice twice over. MySQL's own
 * default collation is case-**insensitive**, so two journal keys differing in
 * case would be one row under a primary key that has to tell them apart — a
 * journal key is `<site>#<kind>/<ordinal>` and a site is an instance path
 * (grammar §9.4), every frame of which is an identifier or a decimal, so the
 * values really are ASCII and really do differ by case. Binary ordering is the
 * second half: `ORDER BY "key"` has to be the order `docs/durability.md` §4
 * derives, and `effectsUnder` matches a prefix with `substr`. One byte per
 * character is what then makes a two-column primary key fit inside InnoDB's
 * index limit with room to spare.
 *
 * Everything a *payload* goes in is `LONGTEXT`: a `TEXT` holds 64 KiB and
 * §3.9's harness payload is the largest private thing this journal keeps — a
 * whole agent loop's event stream, subagent transcripts included.
 *
 * `seq` is the insertion order `docs/distributed.md` §6.2's park order breaks
 * ties with, which SQLite gets from its implicit `rowid` and this has to
 * declare. `AUTO_INCREMENT` requires a key of its own, which is what the
 * `UNIQUE KEY` beside the primary key is for. Rows are inserted and never
 * deleted, so it is exactly monotonic.
 */
const MYSQL_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS executions (
  id              VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  flow            TEXT NOT NULL,
  trigger_kind    TEXT NOT NULL,
  inputs          LONGTEXT NOT NULL,
  session_key     TEXT NOT NULL,
  callback        TEXT,
  traceparent     TEXT,
  status          VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  journal_version INT NOT NULL,
  started_at      VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ended_at        VARCHAR(32),
  error           TEXT,
  PRIMARY KEY (id),
  KEY executions_by_status (status, started_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS effects (
  execution   VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  \`key\`       VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  site        VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  kind        VARCHAR(16)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ordinal     INT NOT NULL,
  request     LONGTEXT NOT NULL,
  outcome     VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  payload     LONGTEXT NOT NULL,
  refused     TINYINT NOT NULL DEFAULT 0,
  recorded_at VARCHAR(32) NOT NULL,
  PRIMARY KEY (execution, \`key\`),
  KEY effects_of_execution (execution)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS deliveries (
  execution    VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ordinal      INT NOT NULL,
  kind         VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin,
  trigger_kind TEXT,
  event        VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  url          TEXT NOT NULL,
  body         LONGTEXT NOT NULL,
  pauses       LONGTEXT NOT NULL,
  status       VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  attempts     LONGTEXT NOT NULL,
  intended_at  VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  settled_at   VARCHAR(32),
  detail       TEXT,
  PRIMARY KEY (execution, ordinal),
  KEY deliveries_by_status (status, intended_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS dispatches (
  seq           BIGINT NOT NULL AUTO_INCREMENT,
  execution     VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  wait          VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  id            VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  placement     VARCHAR(255) NOT NULL,
  node          VARCHAR(255) NOT NULL,
  site          VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  inputs        LONGTEXT NOT NULL,
  item_index    INT,
  history       LONGTEXT,
  policy        LONGTEXT,
  status        VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  session       VARCHAR(255),
  outcome       VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin,
  payload       LONGTEXT,
  parked_at     VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  dispatched_at VARCHAR(32),
  settled_at    VARCHAR(32),
  detail        TEXT,
  PRIMARY KEY (execution, wait),
  UNIQUE KEY dispatches_seq (seq),
  KEY dispatches_by_id (id),
  KEY dispatches_by_status (status, parked_at)
) ENGINE=InnoDB DEFAULT CHARSET=utf8mb4`,
];

/**
 * MySQL's no-op upsert: an assignment that changes nothing.
 *
 * `INSERT IGNORE` is deliberately not the spelling — it swallows every error the
 * statement can raise rather than only the duplicate key, so a payload MySQL
 * refused would be a row silently not written rather than a failure this journal
 * reports. Assigning a primary-key column to itself is the narrow form of the
 * same intent.
 */
function duplicateKeyNoop(_target: string, noop: string): string {
  return `ON DUPLICATE KEY UPDATE ${noop}`;
}

/** MySQL takes `?` as written, and orders dispatches by `seq`. */
const MYSQL_DIALECT: Dialect = {
  provider: "mysql",
  conflict: duplicateKeyNoop,
  bind: positionalBind,
  insertionOrder: "seq",
};

/** The journal as a MySQL database, on one connection this process holds. */
class MysqlDriver implements JournalDriver {
  readonly dialect = MYSQL_DIALECT;
  readonly #connection: Connection;

  constructor(connection: Connection) {
    this.#connection = connection;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Row[]> {
    const [rows] = await this.#connection.query<RowDataPacket[]>(sql, [...parameters]);
    return rows as Row[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<void> {
    await this.#connection.query(sql, [...parameters]);
  }

  async close(): Promise<void> {
    // The `GET_LOCK` goes with the connection, so ending it is releasing the
    // guard — there is nothing to unlock and nothing left holding it if this
    // never runs (see [`guardHeld`]).
    await this.#connection.end();
  }
}

/**
 * Open the MySQL journal: one connection, the schema, and the writer guard.
 *
 * **One connection rather than a pool**, and that is the design rather than a
 * simplification. The journal has exactly one writer (`docs/durability.md` §2,
 * PRD resolved q42), every statement it runs is a single short one, and
 * `SqlJournal` already serializes them — so a pool would buy no parallelism this
 * module can use while making the writer guard unholdable: `GET_LOCK` is
 * **connection**-scoped, and a pool hands connections out and takes them back.
 *
 * `ANSI_QUOTES` is set on the session because the shared statements quote the
 * one column whose name is a reserved word — `"key"` — the way SQLite and
 * Postgres both read it. It changes what a double quote means and nothing else:
 * every literal `SqlJournal` spells is single-quoted, and this connection runs
 * no statement but its own.
 *
 * `GET_LOCK(name, 0)` rather than a timeout: a second opener is told what is
 * happening ([`guardHeld`]) rather than left blocking on a connection that may
 * be a `serve` which will hold it for days.
 */
async function openMysql(): Promise<JournalDriver> {
  const connection = await createConnection({
    uri: journalUrl(),
    // A `BIGINT` comes back as a **string** rather than as a JavaScript number,
    // which is exact where a number is not: `dispatches.seq` is one, and so is
    // the `MAX(ordinal) + 1` a delivery's ordinal is allocated from, because
    // MySQL widens an aggregate over an `INT`. Every reader here goes through
    // `Number(…)`, so a string costs nothing and a silently rounded 64-bit value
    // would cost a row its identity.
    supportBigNumbers: true,
    bigNumberStrings: true,
  });
  try {
    // `CONCAT_WS` rather than `CONCAT`, because a server whose `sql_mode` is
    // empty would otherwise be handed a list with a leading comma — an empty
    // mode name, which MySQL refuses. `NULLIF` is what turns the empty string
    // into the `NULL` `CONCAT_WS` skips.
    await connection.query(
      "SET SESSION sql_mode = CONCAT_WS(',', NULLIF(@@sql_mode, ''), 'ANSI_QUOTES')",
    );
    for (const statement of MYSQL_SCHEMA) await connection.query(statement);
    const [rows] = await connection.query<RowDataPacket[]>("SELECT GET_LOCK(?, 0) AS taken", [
      WRITER_GUARD,
    ]);
    // `GET_LOCK` answers `1` when it took the lock, `0` when it timed out, and
    // `NULL` when something went wrong — and only the first is this process
    // holding the journal.
    if (Number(rows[0]?.["taken"] ?? 0) !== 1) throw guardHeld();
  } catch (error) {
    await connection.end().catch(() => undefined);
    throw error;
  }
  return new MysqlDriver(connection);
}

BACKENDS.mysql = openMysql;
