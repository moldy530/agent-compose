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
 *    8.0, so a server older than that refuses this DDL by name at the first open
 *    rather than folding keys afterwards.
 *
 * **The arm's floor is higher than this collation's**, and it is asserted rather
 * than discovered: [`duplicateKeyOverwrite`] spells the upsert with the row
 * alias MySQL took in **8.0.19**, so 8.0.19 is the version this arm needs whole.
 * A server between the two would create both tables cleanly and then refuse
 * every keyed write with a syntax error, which is why [`openMysqlStore`] reads
 * the server's version and refuses it by name *before* this DDL runs
 * ([`mysqlIsBelowFloor`]).
 *
 * `scope_key` and `store` are this compiler's own strings — a store name is an
 * identifier (grammar 2.1) and a partition is `global`, `session/<encoded key>`
 * or `execution/<id>`, percent-encoded to ASCII by `encodeKey` — so they are
 * `ascii` and `ascii_bin`, which is one byte per character and exact.
 *
 * **The uniqueness is on a hash, and that is what keeps the bounds off the
 * index budget.** InnoDB indexes a key of at most 3072 bytes under
 * `ROW_FORMAT=DYNAMIC`, and a `VARCHAR` counts at its charset's *widest* — four
 * bytes per character for `utf8mb4`, whatever the value really holds. A primary
 * key over the literal `(store, scope_key, "key")` therefore buys its two
 * halves against each other: a `scope_key` wide enough for a session key with
 * CJK in it (`encodeKey` percent-encodes, so one such character is nine ASCII
 * ones) leaves a `key` column of a few hundred characters, and grammar 11.4
 * puts no restriction on a `kv` key at all. A key two arms take and one refuses
 * is the same broken contract this file already refuses `TEXT` over, and on the
 * key it is worse than on the value: the store that broke is the one the deploy
 * file swapped in, and the composition was green on a laptop.
 *
 * So the columns are sized for what a composition writes and the **primary key
 * is `(store, scope_hash, key_hash)`** — 128 + 32 + 32 = 192 bytes, which the
 * limit never comes near again. The hashes are `STORED` generated columns, so
 * the shared statements in `src/stores.ts` never mention them: they insert
 * `(store, scope_key, "key", value)` and the server derives the rest, which is
 * what keeps one implementation of grammar 11.4 rather than two. SHA-256 is
 * what makes "one row per key" still true — two keys colliding is not a case
 * this or any other store defends against — and `utf8mb4_0900_bin` still
 * decides every comparison a statement writes, because `"key" = ?` is answered
 * by the column and not by its hash.
 *
 * `store_entries_partition` is what the statements actually read through: the
 * primary key orders by a hash, so it cannot serve `WHERE store = ? AND
 * scope_key = ? AND "key" = ?` or the partition-wide `DELETE` that
 * [`releaseExecution`] makes. It is a *prefix* index, which is legal precisely
 * because it is not the unique one — MySQL narrows on the prefix and rechecks
 * the whole value.
 *
 * **The residual bounds are the row rather than the index**, and they are
 * stated because they are still bounds: `key` holds 2048 characters and
 * `scope_key` 2048 encoded ASCII ones (`session/` plus 2040 ASCII characters of
 * session key, or 226 CJK ones — `encodeKey` spends nine on each). The local arm's SQLite `TEXT` has no bound
 * and the Postgres arm's is its own — a btree tuple of about 2704 bytes across
 * the whole primary key — so this is the arm to check first when a very long
 * key has to travel. Past the bounds is an **error** rather than a truncation,
 * because [`openMysqlStore`] sets `STRICT_TRANS_TABLES` — and a truncated key
 * is two keys becoming one row, which is the same wrong answer the collation is
 * about.
 *
 * `ROW_FORMAT=DYNAMIC` is declared rather than inherited because the index
 * limit above is 767 bytes under the older formats a server's
 * `innodb_default_row_format` can still name, and a schema that created itself
 * differently on two servers would be the divergence this file exists to avoid.
 *
 * `value` is `LONGTEXT`: `TEXT` holds 64 KiB, a `kv` value is whatever
 * `value_schema` admits, and a write two arms take and one refuses is not one
 * contract. It is text rather than `JSON` for the reason the Postgres arm gives:
 * the column holds the JSON `src/stores.ts` produced, and a `JSON` column would
 * normalize it.
 *
 * `store_applied` is the idempotency ledger of grammar 9.4, and its primary key
 * is what makes the dedupe the **server's**. Its key really is index-sized
 * without help: an idempotency key is `<execution id>/<instance path>` (PRD
 * 5.8), every frame of which this compiler composes out of identifiers and
 * decimals, so 2048 ASCII bytes beside `store`'s 128 is 2176 and inside the
 * limit.
 */
const MYSQL_STORE_SCHEMA = [
  `CREATE TABLE IF NOT EXISTS store_entries (
  store      VARCHAR(128)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key  VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  \`key\`      VARCHAR(2048) CHARACTER SET utf8mb4 COLLATE utf8mb4_0900_bin NOT NULL,
  value      LONGTEXT NOT NULL,
  scope_hash BINARY(32) AS (UNHEX(SHA2(scope_key, 256))) STORED NOT NULL,
  key_hash   BINARY(32) AS (UNHEX(SHA2(\`key\`, 256))) STORED NOT NULL,
  PRIMARY KEY (store, scope_hash, key_hash),
  KEY store_entries_partition (store, scope_key(255), \`key\`(191))
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
  `CREATE TABLE IF NOT EXISTS store_applied (
  store           VARCHAR(128)  CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  scope_key       VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  idempotency_key VARCHAR(2048) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  op              VARCHAR(16) CHARACTER SET ascii COLLATE ascii_bin NOT NULL,
  result          LONGTEXT NOT NULL,
  PRIMARY KEY (store, idempotency_key),
  KEY store_applied_partition (store, scope_key(255))
) ENGINE=InnoDB ROW_FORMAT=DYNAMIC DEFAULT CHARSET=utf8mb4`,
];

/**
 * MySQL's "that table is already there" — the one error a concurrent creator
 * raises out of a DDL that says `IF NOT EXISTS`.
 *
 * A store is the slot a mesh opens from several processes at once: grammar 14.1
 * rule 5 admits a dialled backend from a *placement*, so a hub and its workers
 * each reach their first store op at start-up and each open a connection. The
 * journal has a writer guard in front of its DDL and therefore one creator by
 * construction; a store must not have one (PRD resolved q63), so the concurrent
 * case is ordinary here rather than an edge, and what is left is to tolerate it.
 *
 * MySQL's metadata locks serialize `CREATE TABLE` against itself, so this is the
 * milder half of the problem — `IF NOT EXISTS` normally answers a warning. It is
 * tolerated rather than assumed because the reverse costs an operator a raw
 * catalog error on a line they did not write, and re-running the statement is
 * free.
 */
const MYSQL_TABLE_EXISTS = 1050;

/**
 * MySQL's overwriting upsert, spelled with the row alias the standard calls
 * `excluded`.
 *
 * `AS excluded … = excluded.value` rather than the older `VALUES(value)`, which
 * MySQL deprecated in 8.0.19 and which reads as a function call rather than as
 * the row being inserted. The alias clause belongs to the `INSERT` rather than
 * to the conflict clause, which is why this hook returns both halves: the shared
 * statement appends whatever it is given after the `VALUES (…)` list, and that
 * is exactly where an alias goes.
 *
 * **The alias is what sets this arm's floor at 8.0.19**, one release above the
 * 8.0 its collation asks for ([`MYSQL_STORE_SCHEMA`]): the same release that
 * deprecated `VALUES(value)` is the one that took `AS excluded`, and an older
 * server answers `ER_PARSE_ERROR` at the `AS` — on every `set` and on nothing
 * else, since no other statement here has a conflict clause with a row in it.
 * That failure is a live deployment's rather than an open's, so the floor is
 * asserted at the open instead ([`mysqlIsBelowFloor`]) and the hook keeps the
 * spelling that is not deprecated.
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
 *
 * **That second number is a property of the connection rather than of the
 * statement**, which is why [`openMysqlStore`] dials with `-FOUND_ROWS`. Under
 * `CLIENT_FOUND_ROWS` — which `mysql2` sets by default — MySQL answers 1 rather
 * than 0 for "an existing row set to its current values", so an insert and a
 * duplicate become the same number, `RemoteKv.#apply` reads every claim as
 * taken, and the dedupe this clause *is* never happens: a redelivered write
 * applies a second time and files `deduped: false` where the other two arms file
 * `deduped: true` with the first attempt's row.
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

/**
 * What makes `ORDER BY "key"` read the whole key rather than a prefix of it.
 *
 * `max_sort_length` is the number of bytes a sort compares, and its default is
 * **1024** — past which MySQL compares values as equal and orders them however
 * the sort happened to land. A `kv` key column holds 2048 `utf8mb4` characters
 * ([`MYSQL_STORE_SCHEMA`]), so the default would cut a sort key in the middle of
 * the one op whose whole contract is an order: grammar 11.4's `list` answers in
 * UTF-8 byte order on every backend, and `limit:` makes that order decide which
 * keys come back at all rather than only in which sequence.
 *
 * Asked for by name for [`MYSQL_STORE_AUTOCOMMIT`]'s reason, and sized to the
 * column so the two cannot drift apart: 2048 characters × 4 bytes.
 */
const MYSQL_STORE_SORT_LENGTH = "SET SESSION max_sort_length = 8192";

/**
 * …and what the session really carries once it has been sent, beside the server
 * it was sent to.
 *
 * One statement for the two because the open already makes this round trip and
 * a floor check is not worth a second one: the version is read at exactly the
 * moment the `autocommit` read-back is, which is before any DDL runs.
 */
const MYSQL_STORE_SESSION_HELD =
  "SELECT @@session.autocommit AS autocommit, VERSION() AS version";

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

/**
 * Whether the server this connection reached is older than **MySQL 8.0.19**,
 * which is the version this arm's statements are spelled for.
 *
 * The floor is [`duplicateKeyOverwrite`]'s rather than the schema's: the row
 * alias arrived in 8.0.19 while `utf8mb4_0900_bin` has been there since 8.0, so
 * the DDL is not what an earlier 8.0 server refuses — it takes both tables and then
 * answers a syntax error on every `set`, for the life of the deployment, while
 * every `get`, `list` and `delete` keeps working. That is the failure this
 * predicate exists to turn into an open that refuses by name.
 *
 * **Only a version that parses and is definitely below the floor is refused**,
 * which is [`openMysqlStore`]'s rule for the `autocommit` read-back read once
 * more: a proxy or a fork that answers something this does not recognize costs
 * nothing rather than costing a working deployment its open, and what it gets
 * instead is the server's own error on the statement that needed the version.
 * A `5.5.5-` prefix — MariaDB's protocol-compatibility hack — parses as 5.5.5
 * and is refused, which is the right answer either way: a MariaDB has neither
 * the row alias nor the collation [`MYSQL_STORE_SCHEMA`] names.
 */
function mysqlIsBelowFloor(version: string): boolean {
  const parts = /^(\d+)\.(\d+)\.(\d+)/.exec(version);
  if (parts === null) return false;
  const major = Number(parts[1]);
  const minor = Number(parts[2]);
  if (major !== 8) return major < 8;
  if (minor !== 0) return false;
  return Number(parts[3]) < 19;
}

/** What a server below that floor is refused with. */
function storeServerTooOld(where: string, version: string): Error {
  return new Error(
    `the \`mysql\` store backend at \`\${${where}}\` is version \`${version}\`, and this arm needs MySQL 8.0.19 or newer: a \`store set\` upserts with the \`AS excluded\` row alias, which arrived in 8.0.19, so an older server would create this store's tables and then refuse every keyed write with a syntax error while every read kept answering (grammar 14.3, PRD resolved q63). Bind this store to a server at 8.0.19 or newer`,
  );
}

/**
 * Whether an error a statement was rejected with means the **connection** is
 * gone rather than that the statement was refused.
 *
 * `fatal` is `mysql2`'s own word for it, and it sets the flag at exactly the
 * places a connection stops being usable: `_handleFatalError` (a network error,
 * a protocol error, a handshake that failed), the `PROTOCOL_CONNECTION_LOST` the
 * stream's `close` raises, and `_addCommandClosedState` — which is what every
 * statement written to an already-dead connection takes. A server's error packet
 * carries none of it, so `ER_DUP_ENTRY` and a value too long stay what they are:
 * this statement's failure, and this store's to report as one.
 *
 * **It is read on the statement rather than only on the connection's `error`
 * event, and that is the whole of why this exists.** `_notifyError` computes
 * `bubbleErrorToConnection` from `!this._command`: a connection that dies with a
 * statement **in flight** hands the error to that statement's `onResult` and
 * emits nothing at all, so the listener [`openMysqlStore`] attaches never runs.
 * That is not the rare half of the failure, it is the ordinary one — a managed
 * failover, a proxy's idle reaper or an operator's `KILL` takes the socket away
 * under whichever op was holding it. Left to the listener alone, such a
 * connection would sit in the dialled cache with no fault recorded, every later
 * op would be a `query()` on a closed connection answering `mysql2`'s raw
 * `Can't add new command when connection is in closed state`, and it would do so
 * for the life of the process — which on a `serve`, the deployment a dialled
 * store exists for, is for ever. The Postgres arm has no such hole:
 * `Client._handleErrorEvent` errors the queries **and** emits, unconditionally.
 */
function mysqlConnectionIsGone(error: unknown): boolean {
  return (error as { fatal?: unknown } | null)?.fatal === true;
}

/** One MySQL server, as the connection a dialled `kv` store runs over. */
class MysqlStoreDriver implements StoreDriver {
  readonly dialect = MYSQL_STORE_DIALECT;
  readonly #connection: Connection;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: { error?: Error };
  /**
   * The **name** of the variable this connection's address was read from, which
   * is what every message about it names — see [`storeConnectionLost`].
   */
  readonly #where: string;
  /** What drops this connection from the dialled cache. See [`#reported`]. */
  readonly #lost: () => void;

  constructor(
    connection: Connection,
    fault: { error?: Error },
    where: string,
    lost: () => void,
  ) {
    this.#connection = connection;
    this.#fault = fault;
    this.#where = where;
    this.#lost = lost;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    try {
      const [rows] = await this.#connection.query<RowDataPacket[]>(sql, [...parameters]);
      return rows as Record<string, unknown>[];
    } catch (error) {
      throw this.#reported(error);
    }
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<number> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    try {
      const [result] = await this.#connection.query(sql, [...parameters]);
      // `BEGIN`, `COMMIT` and `ROLLBACK` answer a header too, and its
      // `affectedRows` is 0 — which is what the callers that do not read it
      // expect and what the callers that do would never ask of one.
      const header = result as Partial<ResultSetHeader>;
      return typeof header.affectedRows === "number" ? header.affectedRows : 0;
    } catch (error) {
      throw this.#reported(error);
    }
  }

  async close(): Promise<void> {
    await this.#connection.end();
  }

  /**
   * What a rejected statement is really reported as.
   *
   * A server's refusal is itself. A **fatal** one is this connection ending
   * under the op that was using it ([`mysqlConnectionIsGone`]), and it is
   * recorded here exactly where the connection's own `error` listener would have
   * recorded it: the fault is set, so every later statement over this connection
   * is refused with the same sentence rather than with a raw closed-state
   * message, and `lost()` drops it from the dialled cache so that the retry the
   * node's `retry:` makes runs over a connection dialled again — which is what
   * [`storeConnectionLost`] promises the operator in so many words.
   *
   * Done **once**: a fault is permanent, `lost()` is the caller's whole response
   * to it, and a second call would queue a second close behind the first.
   */
  #reported(error: unknown): unknown {
    if (!mysqlConnectionIsGone(error)) return error;
    if (this.#fault.error === undefined) {
      this.#fault.error = storeConnectionLost("mysql", this.#where, error);
      this.#lost();
    }
    return this.#fault.error;
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
 * **The server's version comes back in that same read**, and a server below
 * this arm's 8.0.19 floor is refused there rather than at its first `set` — see
 * [`mysqlIsBelowFloor`] for why the DDL is not what catches it.
 *
 * The connection charset is **left at `mysql2`'s own default**, which is
 * `utf8mb4` — the charset the key and value columns are declared in. Naming one
 * here would mean naming a *collation*, since that is what the driver's option
 * takes, and a comparison between two utf8mb4 collations is settled by
 * coercibility in the column's favour either way: the name would buy nothing and
 * could only be wrong.
 *
 * **`-FOUND_ROWS` is the one client flag this arm spells**, and it is the
 * dedupe of grammar 9.4 rather than a tuning knob. `mysql2` puts `FOUND_ROWS`
 * in `getDefaultFlags()`, and under `CLIENT_FOUND_ROWS` MySQL answers an
 * affected-rows of 1 rather than 0 for "an existing row set to its current
 * values" — which is exactly the shape of the ledger claim in
 * [`duplicateKeyNoop`], so every claim would read as taken and a redelivered
 * write would apply twice. `mergeFlags` reads a leading `-` as "drop this
 * default", and the flag is asked for here rather than left to the address
 * because a `?flags=` on the URL is only consulted where this option is absent —
 * which is also what keeps an operator's URL from taking it back off.
 * Nothing else this arm runs reads an affected-rows that the flag changes: a
 * `DELETE`'s count is the same under both, and no statement here is a bare
 * `UPDATE`.
 */
async function openMysqlStore(
  url: string,
  where: string,
  lost: () => void,
): Promise<StoreDriver> {
  const connection = await createConnection({
    uri: url,
    // This side's own probes, the other direction of the socket question.
    enableKeepAlive: true,
    // See the note above: the ledger claim's arithmetic, not a preference. The
    // list form because that is what `mysql2` types the option as, and
    // `mergeFlags` takes an array as written.
    flags: ["-FOUND_ROWS"],
  });
  const fault: { error?: Error } = {};
  // `mysql2` emits `error` on the connection whenever the far end goes away with
  // no command in flight — `_notifyError` sets `bubbleErrorToConnection` from
  // `!this._command` and emits — and an `EventEmitter` that emits `error` with
  // nothing listening ends the process. This is the first moment there is a
  // connection to attach to: the promise `createConnection` answers rejects
  // rather than emits when it is the *connect* that failed.
  //
  // It is **half** of how this arm hears about a lost socket, and deliberately
  // so: the other half is [`MysqlStoreDriver.#reported`], which covers the case
  // this listener is never called for — the far end going away with a statement
  // in flight, which `mysql2` reports to that statement and to nobody else.
  connection.on("error", (reported: unknown) => {
    fault.error ??= storeConnectionLost("mysql", where, reported);
    // …and the connection goes with it, so the next op dials a fresh one rather
    // than inheriting this error for the life of the process. See
    // [`storeConnectionLost`].
    lost();
  });
  try {
    // **First of everything**, so that nothing this open runs can already be
    // inside a transaction nobody will commit.
    await connection.query(MYSQL_STORE_AUTOCOMMIT);
    await connection.query(MYSQL_STORE_SORT_LENGTH);
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
    // …and the server itself, refused **before** the DDL rather than after it:
    // a server under the floor takes these tables and refuses every keyed write
    // afterwards, so an open that created them and then failed would leave an
    // operator reading a syntax error rather than a sentence. See
    // [`mysqlIsBelowFloor`].
    const version = String(session[0]?.["version"] ?? "");
    if (mysqlIsBelowFloor(version)) throw storeServerTooOld(where, version);
    // The schema has to be named before the DDL runs under it: a URL that names
    // none would fail with MySQL's own "No database selected" on the first
    // `CREATE TABLE`, which tells an operator nothing about the line they wrote.
    const [scoped] = await connection.query<RowDataPacket[]>("SELECT DATABASE() AS db");
    const database = scoped[0]?.["db"];
    if (typeof database !== "string" || database === "") throw storeNamesNoDatabase(where);
    for (const statement of MYSQL_STORE_SCHEMA) {
      try {
        await connection.query(statement);
      } catch (error) {
        // A process that lost the race to create this table, which is the
        // ordinary case at a mesh's start-up rather than an edge one. See
        // [`MYSQL_TABLE_EXISTS`].
        if ((error as { errno?: unknown } | null)?.errno !== MYSQL_TABLE_EXISTS) throw error;
      }
    }
  } catch (error) {
    await connection.end().catch(() => undefined);
    throw error;
  }
  return new MysqlStoreDriver(connection, fault, where, lost);
}

STORE_BACKENDS.mysql = openMysqlStore;
