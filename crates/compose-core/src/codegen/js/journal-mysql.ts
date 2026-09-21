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
 * **A column that is unbounded on the other two backends is `LONGTEXT` here**,
 * and that is the rule rather than a list. `TEXT` holds 64 KiB, MySQL 8 ships
 * `STRICT_TRANS_TABLES`, and this arm preserves whatever `sql_mode` the server
 * has — so an over-long value is an *error* rather than a truncation, and an
 * error on a write the other two backends took is "one contract, three
 * backends" untrue on one of them. The columns that make that concrete are not
 * only the payloads: `executions.error` is a provider's whole failure body or a
 * harness run's quoted transcript, and a `journal.end(id, 'failed', …)` the
 * server refuses leaves the lifecycle row `open` for ever — so every later
 * `serve` start re-recovers and re-replays an execution that has already
 * finished.
 *
 * The exceptions are the columns MySQL will not index without a bound, and they
 * are bounded here rather than anywhere else: `VARCHAR(2048)` on the two halves
 * of a compound primary key that are not fixed-shape ids — `effects."key"` and
 * `dispatches.wait` — which with `VARCHAR(255)` beside them and one ASCII byte
 * per character leaves a 2303-byte key inside InnoDB's 3072-byte index limit
 * with room to spare. `ROW_FORMAT=DYNAMIC` is declared rather than inherited
 * because that limit is 767 bytes under the older row formats a server's
 * `innodb_default_row_format` can still name, and a schema that created itself
 * differently on two servers would be the divergence this file exists to avoid.
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
  flow            LONGTEXT NOT NULL,
  trigger_kind    LONGTEXT NOT NULL,
  inputs          LONGTEXT NOT NULL,
  session_key     LONGTEXT NOT NULL,
  callback        LONGTEXT,
  traceparent     LONGTEXT,
  status          VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  journal_version INT NOT NULL,
  started_at      VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ended_at        VARCHAR(32),
  error           LONGTEXT,
  PRIMARY KEY (id),
  KEY executions_by_status (status, started_at)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS effects (
  execution   VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  \`key\`       VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  site        LONGTEXT CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  kind        VARCHAR(16)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ordinal     INT NOT NULL,
  request     LONGTEXT NOT NULL,
  outcome     VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  payload     LONGTEXT NOT NULL,
  refused     TINYINT NOT NULL DEFAULT 0,
  recorded_at VARCHAR(32) NOT NULL,
  PRIMARY KEY (execution, \`key\`),
  KEY effects_of_execution (execution)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS deliveries (
  execution    VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  ordinal      INT NOT NULL,
  kind         VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin,
  trigger_kind LONGTEXT,
  event        VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  url          LONGTEXT NOT NULL,
  body         LONGTEXT NOT NULL,
  pauses       LONGTEXT NOT NULL,
  status       VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  attempts     LONGTEXT NOT NULL,
  intended_at  VARCHAR(32) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  settled_at   VARCHAR(32),
  detail       LONGTEXT,
  PRIMARY KEY (execution, ordinal),
  KEY deliveries_by_status (status, intended_at)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS dispatches (
  seq           BIGINT NOT NULL AUTO_INCREMENT,
  execution     VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  wait          VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  id            VARCHAR(255) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  placement     LONGTEXT NOT NULL,
  node          LONGTEXT NOT NULL,
  site          LONGTEXT CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
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
  detail        LONGTEXT,
  PRIMARY KEY (execution, wait),
  UNIQUE KEY dispatches_seq (seq),
  KEY dispatches_by_id (id),
  KEY dispatches_by_status (status, parked_at)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
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

/**
 * How long MySQL is asked to hold a session that has stopped saying anything.
 *
 * `wait_timeout` is what MySQL has instead of Postgres' per-session TCP
 * keepalives, and its shipped default is eight hours — so a hub whose host
 * vanished goes on holding `GET_LOCK` for most of a working day while the
 * machine that took over is refused. Set for **this session only**, so nothing
 * an operator configured for the rest of the server moves.
 *
 * It measures the application's own silence rather than the peer's reachability,
 * which is why [`GUARD_HEARTBEAT_MS`] is an order of magnitude below it: a
 * `serve` that journals nothing for an afternoon has to stay a live session, and
 * the only thing that can say so on a protocol with no kernel-level probe is a
 * round trip.
 */
const MYSQL_WAIT_TIMEOUT = `SET SESSION wait_timeout = ${GUARD_REAP_SECONDS}`;

/**
 * [`WRITER_GUARD`], qualified with the schema this connection is addressing.
 *
 * **MySQL's user-level locks are server-wide**, and that is the one place this
 * arm cannot read the Postgres one and translate. `pg_try_advisory_lock` is
 * scoped to the database the session connected to, so two deployments in two
 * databases of one cluster take two different locks under one name; `GET_LOCK`
 * is keyed on the name **alone**, so the same two on one MySQL server would take
 * the *same* lock — and a `staging` hub that is up and well would refuse `prod`'s
 * `serve` with [`guardHeld`]'s advice to wait five minutes for a host that never
 * vanished. One database, one journal, one writer (`docs/durability.md` §2.3) is
 * the rule on both backends; here the name has to say so.
 *
 * `DATABASE()` rather than anything parsed out of the URL, because it is what
 * every statement below will really address — a schema the connection was
 * redirected to is one this guard still covers.
 *
 * **A digest rather than the schema's own name**, and that is arithmetic rather
 * than obfuscation: MySQL refuses a lock name longer than 64 characters, a
 * database name may be 64 characters by itself, and a guard that failed to open
 * on a long schema name would be a deployment refused for how it was spelled.
 * The first 128 bits of a SHA-256 are 32 characters, which leaves this name 55
 * and no way to grow. `SHA2` rather than `MD5` because a server in FIPS mode
 * refuses the latter. An operator looking for the holder reads it back the way
 * `tests/toolchain/journal-contract.mjs` does — `IS_USED_LOCK` of this same
 * expression, which the two spell identically under a drift test
 * (`the_runner_and_the_module_take_one_writer_guard`).
 */
const MYSQL_GUARD_NAME = "CONCAT(?, ':', LEFT(SHA2(DATABASE(), 256), 32))";

/** What an address with no database on it is refused with. See [`openMysql`]. */
function noDatabaseNamed(): Error {
  return new Error(
    `this project's journal is ${journalLocation()}, and that address names no database: a \`mysql://\` URL carries the schema as its path (\`mysql://user:pass@host:3306/agent_compose\`), and this one stops at the host. The journal's tables live in a schema and its writer guard is taken under the one this connection is addressing (\`docs/durability.md\` §2.3), so there is nothing to open until the variable names it (grammar §14.7)`,
  );
}

/** The journal as a MySQL database, on one connection this process holds. */
class MysqlDriver implements JournalDriver {
  readonly dialect = MYSQL_DIALECT;
  readonly #connection: Connection;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: ConnectionFault;
  /** What keeps [`MYSQL_WAIT_TIMEOUT`] from reaping an idle hub. */
  readonly #heartbeat: ReturnType<typeof setInterval>;

  constructor(connection: Connection, fault: ConnectionFault) {
    this.#connection = connection;
    this.#fault = fault;
    this.#heartbeat = setInterval(() => {
      // Nothing reads the answer and nothing acts on the failure: the round trip
      // is the whole point, and a connection that could not make one has already
      // told the listener in [`openMysql`]. The rejection is caught so that a
      // lost connection is one refused statement rather than an unhandled
      // rejection.
      void this.#connection.ping().catch(() => undefined);
    }, GUARD_HEARTBEAT_MS);
    this.#heartbeat.unref();
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Row[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const [rows] = await this.#connection.query<RowDataPacket[]>(sql, [...parameters]);
    return rows as Row[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<void> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    await this.#connection.query(sql, [...parameters]);
  }

  async close(): Promise<void> {
    // The `GET_LOCK` goes with the connection, so ending it is releasing the
    // guard — there is nothing to unlock, and a connection this process never
    // got to end is reaped by the server inside [`GUARD_REAP_SECONDS`] (see
    // [`guardHeld`]).
    clearInterval(this.#heartbeat);
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
 * be a `serve` which will hold it for days. The name is [`MYSQL_GUARD_NAME`],
 * which carries the schema, so the journal of one database on a shared server
 * locks out nothing but its own second writer.
 *
 * **The guard is taken before the schema is created**, and the order is load
 * bearing rather than tidy: two processes opening one fresh journal at the same
 * time would otherwise both run the DDL against an operator's database, and the
 * one that is about to be refused has no business creating anything. The
 * Postgres arm says the rest of it, including the catalog race that makes
 * concurrent creation worse than merely pointless.
 */
async function openMysql(): Promise<JournalDriver> {
  const connection = await createConnection({
    uri: journalUrl(),
    // This side's own probes, the other direction of the question
    // [`MYSQL_WAIT_TIMEOUT`] asks the server.
    enableKeepAlive: true,
    keepAliveInitialDelay: GUARD_HEARTBEAT_MS,
    // A `BIGINT` comes back as a **string** rather than as a JavaScript number,
    // which is exact where a number is not: `dispatches.seq` is one, and so is
    // the `MAX(ordinal) + 1` a delivery's ordinal is allocated from, because
    // MySQL widens an aggregate over an `INT`. Every reader here goes through
    // `Number(…)`, so a string costs nothing and a silently rounded 64-bit value
    // would cost a row its identity.
    supportBigNumbers: true,
    bigNumberStrings: true,
  });
  const fault: ConnectionFault = {};
  // `mysql2` emits `error` on the connection whenever the far end goes away with
  // no command in flight — `_notifyError` sets `bubbleErrorToConnection` from
  // `!this._command` and emits — and an `EventEmitter` that emits `error` with
  // nothing listening ends the process. See [`ConnectionFault`]. This is the
  // first moment there is a connection to attach to: the promise `createConnection`
  // answers rejects rather than emits when it is the *connect* that failed.
  connection.on("error", (reported: unknown) => {
    fault.error ??= connectionLost(reported);
  });
  try {
    // `CONCAT_WS` rather than `CONCAT`, because a server whose `sql_mode` is
    // empty would otherwise be handed a list with a leading comma — an empty
    // mode name, which MySQL refuses. `NULLIF` is what turns the empty string
    // into the `NULL` `CONCAT_WS` skips.
    await connection.query(
      "SET SESSION sql_mode = CONCAT_WS(',', NULLIF(@@sql_mode, ''), 'ANSI_QUOTES')",
    );
    // Best effort, and said out loud where it does not take: see
    // [`guardWindowUnshortened`].
    await connection.query(MYSQL_WAIT_TIMEOUT).catch(guardWindowUnshortened);
    // The schema has to be there before the guard is taken under it: a URL that
    // names none leaves `DATABASE()` `NULL`, which makes [`MYSQL_GUARD_NAME`]
    // `NULL` and `GET_LOCK` of it an error or a nothing — and a nothing reads
    // here as a guard somebody else is holding, which is the one wrong thing to
    // tell an operator whose address is simply short a path. The DDL below would
    // fail on the same URL with MySQL's own "No database selected", so this is
    // the same refusal, made first and by name.
    const [scoped] = await connection.query<RowDataPacket[]>("SELECT DATABASE() AS db");
    const database = scoped[0]?.["db"];
    if (typeof database !== "string" || database === "") throw noDatabaseNamed();
    const [rows] = await connection.query<RowDataPacket[]>(
      `SELECT GET_LOCK(${MYSQL_GUARD_NAME}, 0) AS taken`,
      [WRITER_GUARD],
    );
    // `GET_LOCK` answers `1` when it took the lock, `0` when it timed out, and
    // `NULL` when something went wrong — and only the first is this process
    // holding the journal.
    if (Number(rows[0]?.["taken"] ?? 0) !== 1) throw guardHeld();
    for (const statement of MYSQL_SCHEMA) await connection.query(statement);
  } catch (error) {
    await connection.end().catch(() => undefined);
    throw error;
  }
  return new MysqlDriver(connection, fault);
}

BACKENDS.mysql = openMysql;
