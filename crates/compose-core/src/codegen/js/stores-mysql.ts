// ---------------------------------------------------------------------------
// The MySQL `kv` arm (grammar §14.3, PRD resolved q63)
// ---------------------------------------------------------------------------
//
// Emitted only into a project one of whose stores binds `provider: mysql`, which
// is also the only project whose `package.json` pins `mysql2` for a store. A
// build whose stores are all local carries neither the driver nor this code,
// which is what keeps the zero-infra guarantee a property of the artifact rather
// than of a code path nobody takes.
//
// Nothing here is a second implementation of anything. The statements are
// `RemoteKv`'s and the contract is grammar 11.4's; what this file adds is a
// connection and a schema. There is no writer guard, and the section above this
// one says why.

import { createConnection } from "mysql2/promise";
import type { Connection, ResultSetHeader, RowDataPacket } from "mysql2/promise";

/**
 * The schema, created on first open and never migrated.
 *
 * `IF NOT EXISTS` throughout, and the index declared **inside** the table
 * because MySQL has no `CREATE INDEX IF NOT EXISTS` and a second open must not
 * be an error.
 *
 * **The key's collation is `utf8mb4_0900_bin`, and both halves of that name are
 * load bearing.**
 *
 *  * `utf8mb4` rather than `ascii`, which is where a store's keys part company
 *    with a journal's. A journal key is an instance path this compiler composes
 *    out of identifiers and decimals; a `kv` key is whatever the composition
 *    evaluated — grammar 11.4 puts no restriction on it, and the corpus
 *    `src/stores.ts` already drives has emoji in it.
 *  * `_bin` because MySQL's default collation is case-**insensitive**: two keys
 *    differing only in case would be one row under a primary key whose whole job
 *    is to tell them apart, so a `get` would answer another key's value. It is
 *    also what makes `ORDER BY "key"` the UTF-8 byte order every other arm
 *    answers `list` in (`byUtf8Bytes` in `src/stores.ts`), since utf8mb4's byte
 *    order is its code-point order.
 *  * `_0900_` because that family is **NO PAD**, and the older `utf8mb4_bin` is
 *    PAD SPACE. Under PAD SPACE `'a'` and `'a '` compare equal, so a store
 *    holding both would hold one row and a `get` of one would answer the other —
 *    silently, and only for keys with trailing whitespace. It arrived in MySQL
 *    8.0, which is therefore this arm's floor; a server older than that refuses
 *    the DDL by name at the first open rather than folding keys afterwards.
 *
 * `scope_key` and `store` are this compiler's own strings — a store name is an
 * identifier (grammar 2.1) and a partition is `global`, `session/<encoded key>`
 * or `execution/<id>`, percent-encoded to ASCII by `encodeKey` — so they are
 * `ascii` and `ascii_bin`, which is one byte per character and exact.
 *
 * **The bounds are index arithmetic.** InnoDB indexes a key of at most 3072
 * bytes under `ROW_FORMAT=DYNAMIC`, and a `VARCHAR` counts at its charset's
 * widest: `store` 128 ASCII bytes plus `scope_key` 512 plus `key` 512 × 4 is
 * 2688, which leaves room and is where a wider column would have to be checked
 * again. `ROW_FORMAT=DYNAMIC` is declared rather than inherited because that
 * limit is 767 bytes under the older formats a server's
 * `innodb_default_row_format` can still name, and a schema that created itself
 * differently on two servers would be the divergence this file exists to avoid.
 * Past the bounds is an **error** rather than a truncation, because
 * [`openMysqlStore`] sets `STRICT_TRANS_TABLES` — and a truncated key is two
 * keys becoming one row, which is the same wrong answer the collation is about.
 *
 * `value` is `LONGTEXT`: `TEXT` holds 64 KiB, a `kv` value is whatever
 * `value_schema` admits, and a write two arms take and one refuses is not one
 * contract. It is text rather than `JSON` for the reason the Postgres arm gives:
 * the column holds the JSON `src/stores.ts` produced, and a `JSON` column would
 * normalize it.
 *
 * `store_applied` is the idempotency ledger of grammar 9.4, and its primary key
 * is what makes the dedupe the **server's**.
 */
const MYSQL_STORE_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS store_entries (
  store     VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  \`key\`     VARCHAR(512) CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_bin NOT NULL,
  value     LONGTEXT NOT NULL,
  PRIMARY KEY (store, scope_key, \`key\`)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS store_applied (
  store           VARCHAR(128) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key       VARCHAR(512) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  idempotency_key VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  op              VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  result          LONGTEXT NOT NULL,
  PRIMARY KEY (store, idempotency_key),
  KEY store_applied_partition (store, scope_key)
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
];

/**
 * MySQL's overwriting upsert, spelled with the row alias the standard calls
 * `excluded`.
 *
 * `AS excluded … = excluded.value` rather than the older `VALUES(value)`, which
 * MySQL deprecated in 8.0.19 and which reads as a function call rather than as
 * the row being inserted. The alias clause belongs to the `INSERT` rather than
 * to the conflict clause, which is why this hook returns both halves: the shared
 * statement appends whatever it is given after the `VALUES (…)` list, and that
 * is exactly where an alias goes. MySQL 8.0 is this arm's floor anyway — see
 * [`MYSQL_STORE_SCHEMA`] on `utf8mb4_0900_bin`.
 */
function duplicateKeyOverwrite(_target: string, column: string): string {
  return `AS excluded ON DUPLICATE KEY UPDATE ${column} = excluded.${column}`;
}

/**
 * …and its no-op one: an assignment that changes nothing.
 *
 * `INSERT IGNORE` is deliberately not the spelling — it swallows every error the
 * statement can raise rather than only the duplicate key, so a value MySQL
 * refused would be a row silently not written rather than a failure this store
 * reports. That matters twice over here, because this clause is the **dedupe**:
 * a claim that failed for any other reason must not read as "somebody already
 * applied this key", which is the one answer that would drop a write.
 *
 * `affectedRows` is what `RemoteKv` reads off it, and MySQL's arithmetic is the
 * one this rests on: 1 for a row inserted, and **0** for a duplicate whose
 * update assigns the column to itself and changes nothing.
 */
function duplicateKeyNoop(_target: string, noop: string): string {
  return `ON DUPLICATE KEY UPDATE ${noop}`;
}

/** MySQL takes `?` as written. */
const MYSQL_STORE_DIALECT: StoreDialect = {
  bind: positionalBind,
  overwrite: duplicateKeyOverwrite,
  ignore: duplicateKeyNoop,
};

/**
 * What makes each statement its own transaction unless `RemoteKv` opened one.
 *
 * `mysql2` sends **nothing** — `grep -rn autocommit node_modules/mysql2/lib/`
 * matches only its constant tables — so what this connection gets is whatever
 * the server's `autocommit` is, and that is a dynamic system variable an
 * operator can set globally or through `init_connect`. With it off, every write
 * this store makes joins one transaction with no `COMMIT` anywhere: reads over
 * this same connection see their own uncommitted rows, so a run reports every op
 * green, and then the process ends and the server rolls all of it back. A second
 * process would have seen none of it in the meantime, which is the one property
 * a dialled store exists for.
 *
 * The same rule the journal's arm states, read on a store: a session setting
 * these statements rest on is stated by the arm rather than assumed of the
 * server.
 */
const MYSQL_STORE_AUTOCOMMIT = "SET SESSION autocommit = 1";

/** …and what the session really carries once it has been sent. */
const MYSQL_STORE_SESSION_HELD = "SELECT @@session.autocommit AS autocommit";

/** What a session that would not commit is refused with. */
function storeWritesWouldNotCommit(where: string): Error {
  return new Error(
    `the \`mysql\` store backend at \`\${${where}}\` does not commit: \`autocommit\` is off on this session even though it was asked to be on. Every write a store made would join one open transaction and be rolled back when this process ends, and no other process would see a row of it in the meantime — which is the property a dialled store exists for (PRD 5.8, resolved q63). Take the \`autocommit = 0\` off this server or its \`init_connect\`, or bind this store to a server that commits`,
  );
}

/** What an address naming no schema is refused with. */
function storeNamesNoDatabase(where: string): Error {
  return new Error(
    `the \`mysql\` store backend at \`\${${where}}\` names no database: a \`mysql://\` URL carries the schema as its path (\`mysql://user:pass@host:3306/agent_compose\`), and this one stops at the host. A store's tables live in a schema, so there is nothing to open until the variable names one (grammar 4.3, 14.3)`,
  );
}

/** What a statement is refused with once a store's connection has been lost. */
function storeConnectionLost(where: string, cause: unknown): Error {
  const detail = cause instanceof Error ? cause.message : String(cause);
  return new Error(
    `this project lost its connection to the \`mysql\` store backend at \`\${${where}}\`: ${detail}. It is not redialled inside the op that failed — a write whose connection died is one nothing can say landed or did not — so this op fails and the node's own \`retry:\` decides what happens next; a retry carries the idempotency key its first attempt carried, which is what makes it apply once (grammar 9.4, PRD 5.8)`,
  );
}

/** One MySQL server, as the connection a dialled `kv` store runs over. */
class MysqlStoreDriver implements StoreDriver {
  readonly dialect = MYSQL_STORE_DIALECT;
  readonly #connection: Connection;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: { error?: Error };

  constructor(connection: Connection, fault: { error?: Error }) {
    this.#connection = connection;
    this.#fault = fault;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const [rows] = await this.#connection.query<RowDataPacket[]>(sql, [...parameters]);
    return rows as Record<string, unknown>[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<number> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const [result] = await this.#connection.query(sql, [...parameters]);
    // `BEGIN`, `COMMIT` and `ROLLBACK` answer a header too, and its
    // `affectedRows` is 0 — which is what the callers that do not read it
    // expect and what the callers that do would never ask of one.
    const header = result as Partial<ResultSetHeader>;
    return typeof header.affectedRows === "number" ? header.affectedRows : 0;
  }

  async close(): Promise<void> {
    await this.#connection.end();
  }
}

/**
 * Open one MySQL store connection: a socket, the session, and the schema.
 *
 * **One connection rather than a pool**, for the Postgres arm's reason: a keyed
 * write is a `BEGIN`, an effect and a ledger insert, and a pool would hand those
 * three up to three sessions.
 *
 * `ANSI_QUOTES` is set on the session because the shared statements quote the
 * one column whose name is a reserved word — `"key"` — the way SQLite and
 * Postgres both read it. It changes what a double quote means and nothing else:
 * every literal `RemoteKv` spells is single-quoted, and this connection runs no
 * statement but its own.
 *
 * **`STRICT_TRANS_TABLES` is set beside it, and it is the schema's arithmetic
 * rather than a preference.** Every bounded column above rests on an over-long
 * value being an *error*; without the mode MySQL right-**truncates** instead,
 * silently, and the column it costs most is the key — two keys sharing a
 * 512-character prefix would truncate to the same primary key, so one would
 * overwrite the other's value and a `get` of either would answer the survivor.
 *
 * **`NO_BACKSLASH_ESCAPES` is cleared**, and that one is removed rather than
 * added because every parameter this arm binds is escaped on the *client*:
 * `mysql2`'s `query` interpolates parameters itself with `SqlString`, which
 * spells a quote `\'` and a backslash `\\`, and those are escapes only while the
 * server reads a backslash as one. On a server carrying the mode, a `kv` value
 * with an apostrophe in it ends its own string literal early and the write is
 * refused mid-run, while one with only double quotes — which every JSON payload
 * this store writes has — is *stored* with its backslashes and throws in the
 * `JSON.parse` on the way back out.
 *
 * **`autocommit` is asserted rather than inherited** — see
 * [`MYSQL_STORE_AUTOCOMMIT`] — and read back, because a connection proxy that
 * answers a `SET` on the client's behalf without applying it to the backend
 * session is the way this fails rather than an error.
 *
 * The connection charset is **left at `mysql2`'s own default**, which is
 * `utf8mb4` — the charset the key and value columns are declared in. Naming one
 * here would mean naming a *collation*, since that is what the driver's option
 * takes, and a comparison between two utf8mb4 collations is settled by
 * coercibility in the column's favour either way: the name would buy nothing and
 * could only be wrong.
 */
async function openMysqlStore(url: string, where: string): Promise<StoreDriver> {
  const connection = await createConnection({
    uri: url,
    // This side's own probes, the other direction of the socket question.
    enableKeepAlive: true,
  });
  const fault: { error?: Error } = {};
  // `mysql2` emits `error` on the connection whenever the far end goes away with
  // no command in flight — `_notifyError` sets `bubbleErrorToConnection` from
  // `!this._command` and emits — and an `EventEmitter` that emits `error` with
  // nothing listening ends the process. This is the first moment there is a
  // connection to attach to: the promise `createConnection` answers rejects
  // rather than emits when it is the *connect* that failed.
  connection.on("error", (reported: unknown) => {
    fault.error ??= storeConnectionLost(where, reported);
  });
  try {
    // **First of everything**, so that nothing this open runs can already be
    // inside a transaction nobody will commit.
    await connection.query(MYSQL_STORE_AUTOCOMMIT);
    // `CONCAT_WS` rather than `CONCAT`, because a server whose `sql_mode` is
    // empty would otherwise be handed a list with a leading comma — an empty
    // mode name, which MySQL refuses. `NULLIF` is what turns the empty string
    // into the `NULL` `CONCAT_WS` skips. The `REPLACE` is the other direction —
    // `NO_BACKSLASH_ESCAPES` taken out rather than a mode put in — and it is
    // comma-wrapped so that a member removed from the middle of the list does
    // not leave the doubled comma MySQL refuses.
    await connection.query(
      "SET SESSION sql_mode = TRIM(BOTH ',' FROM REPLACE(CONCAT(',', " +
        "CONCAT_WS(',', NULLIF(@@sql_mode, ''), 'ANSI_QUOTES', 'STRICT_TRANS_TABLES')" +
        ", ','), ',NO_BACKSLASH_ESCAPES,', ','))",
    );
    const [session] = await connection.query<RowDataPacket[]>(MYSQL_STORE_SESSION_HELD);
    const autocommit = Number(session[0]?.["autocommit"]);
    // Acted on only when the server answered a number that is definitely not
    // `1`, so a driver that one day hands it back in some other shape costs
    // nothing rather than costing every MySQL deployment its open.
    if (Number.isFinite(autocommit) && autocommit !== 1) throw storeWritesWouldNotCommit(where);
    // The schema has to be named before the DDL runs under it: a URL that names
    // none would fail with MySQL's own "No database selected" on the first
    // `CREATE TABLE`, which tells an operator nothing about the line they wrote.
    const [scoped] = await connection.query<RowDataPacket[]>("SELECT DATABASE() AS db");
    const database = scoped[0]?.["db"];
    if (typeof database !== "string" || database === "") throw storeNamesNoDatabase(where);
    for (const statement of MYSQL_STORE_SCHEMA) await connection.query(statement);
  } catch (error) {
    await connection.end().catch(() => undefined);
    throw error;
  }
  return new MysqlStoreDriver(connection, fault);
}

STORE_BACKENDS.mysql = openMysqlStore;
