// ---------------------------------------------------------------------------
// The Postgres `kv` arm (grammar §14.3, PRD resolved q63)
// ---------------------------------------------------------------------------
//
// Emitted only into a project one of whose stores binds `provider: postgres`,
// which is also the only project whose `package.json` pins `pg` for a store. A
// build whose stores are all local carries neither the driver nor this code,
// which is what keeps the zero-infra guarantee a property of the artifact rather
// than of a code path nobody takes.
//
// Nothing here is a second implementation of anything. The statements are
// `RemoteKv`'s and the contract is grammar 11.4's; what this file adds is a
// connection and a schema. There is no writer guard, and the section above this
// one says why.

import { Client } from "pg";

/**
 * The advisory lock one opener holds while it creates the schema.
 *
 * **Not the journal's writer guard, and the difference is the scope.** The
 * journal takes a *session*-scoped `pg_try_advisory_lock` and keeps it for as
 * long as the process lives, which is what makes it one writer
 * (`journal-postgres.ts`). A store must be multi-writer (PRD resolved q63), so
 * this is `pg_advisory_xact_lock`: the server drops it when the transaction that
 * took it commits, which is the same round trip the DDL is in. Nothing holds
 * anything afterwards, and two processes that have opened are two writers.
 *
 * It is there because the *creation* really does need one creator. Grammar 14.1
 * rule 5 admits a dialled store from a **placement** — a hub and N workers, each
 * of which opens at its own first op — so several processes reaching one fresh
 * database at once is the ordinary start-up rather than an edge case, and
 * Postgres documents `CREATE TABLE IF NOT EXISTS` as *not* atomic against a
 * concurrent creator: one of the two can fail on a duplicate key in `pg_type` or
 * `pg_class`, which is a catalog error naming nothing an operator wrote. Under
 * the lock there is one creator by construction, exactly as under the journal's
 * guard, and none of the store's ops are under anything.
 *
 * The blocking form rather than the `try` one, which is the other half of the
 * same scope argument: what is being waited for is another opener's DDL, so the
 * wait is one round trip. The journal cannot use it because what it would wait
 * for is a `serve` that may hold its guard for days.
 *
 * Two keys, and the second differs from the journal's (`WRITER_GUARD_KEYS`) so
 * that a project whose journal and stores share one database does not have its
 * store openers queue behind the journal's writer. Advisory locks share one
 * namespace per database, and the wait above is the blocking one, so a pair that
 * coincided would be an open that never returns — no SQLSTATE, no timeout and no
 * message. The two constants are in two files, so
 * `the_store_schema_lock_is_not_the_journals_writer_guard` (`codegen::stores`)
 * is what compares them; nothing else in the workspace does, and no conformance
 * case can, since reproducing it needs a Postgres journal and a Postgres store
 * open against one server at once.
 */
const POSTGRES_STORE_SCHEMA_LOCK: readonly [number, number] = [0x6167_656e, 0x742d_7374];

/**
 * The SQLSTATEs a concurrent creator raises out of a DDL that says
 * `IF NOT EXISTS`.
 *
 * Belt and braces beside [`POSTGRES_STORE_SCHEMA_LOCK`], which already makes
 * this unreachable for two openers of *this* runtime. What it covers is a
 * creator outside the lock — a migration this database is being prepared with,
 * an older build — and the cost of not covering it is an operator reading
 * `duplicate key value violates unique constraint "pg_type_typname_nsp_index"`
 * on a line they never wrote.
 *
 * `23505` is the catalog's own unique index answering, `42P07` is
 * `duplicate_table` and `42710` is `duplicate_object`, which is the index.
 */
const POSTGRES_CONCURRENT_CREATOR: readonly string[] = ["23505", "42P07", "42710"];

/**
 * The schema, created on first open and never migrated.
 *
 * `IF NOT EXISTS` throughout, so a second open of a store this build already
 * created does nothing — which is all a physical schema can be asked for here: a
 * dialled store is created by this release or a later one, so there is no older
 * database whose columns have to be probed for. The whole of it is **one
 * transaction holding [`POSTGRES_STORE_SCHEMA_LOCK`]**, because `IF NOT EXISTS`
 * is not atomic against a concurrent creator and a store is opened concurrently
 * by design.
 *
 * **`COLLATE "C"` on every column a statement compares or orders by**, and it is
 * a correctness choice rather than a preference. A `kv` key is the
 * composition's own string: grammar 11.4 puts no restriction on it, two keys
 * differing in case are two keys, and `list` answers in the order
 * `ORDER BY "key"` gives — which `src/stores.ts` fixes as UTF-8 byte order
 * (`byUtf8Bytes`), so that the SQLite arm, the `blob` arm and this one answer
 * one catalogue row the same way. A database created under a linguistic default
 * collation would fold case and punctuation in both, silently and only for some
 * rows. `COLLATE "C"` is byte order on every encoding this runs on.
 *
 * `scope_key` carries it too: it is `execution/<id>`, `session/<encoded key>` or
 * `global`, and a partition is exactly a thing two sessions must not share.
 *
 * `value` carries no collation because nothing compares it: it is the JSON text
 * `src/stores.ts` produced, held as text so it comes back as it went in. A
 * `JSONB` column would normalize key order and whitespace, so a `value:` would
 * round-trip as a different document than the one `value_schema` checked, and
 * this arm would answer something the local arm does not.
 *
 * `store_applied` is the idempotency ledger of grammar 9.4, and its primary key
 * is what makes the dedupe the **server's**: a repeated write's claim conflicts,
 * `ON CONFLICT DO NOTHING` changes no row, and `RemoteKv` reads the answer the
 * first attempt gave instead of applying the effect again. `scope_key` is on the
 * row rather than in the key so that an execution-scoped partition can be
 * deleted whole when its run ends.
 */
const POSTGRES_STORE_SCHEMA = `
BEGIN;
SELECT pg_advisory_xact_lock(${POSTGRES_STORE_SCHEMA_LOCK[0]}, ${POSTGRES_STORE_SCHEMA_LOCK[1]});
CREATE TABLE IF NOT EXISTS store_entries (
  store     TEXT COLLATE "C" NOT NULL,
  scope_key TEXT COLLATE "C" NOT NULL,
  "key"     TEXT COLLATE "C" NOT NULL,
  value     TEXT NOT NULL,
  PRIMARY KEY (store, scope_key, "key")
);
CREATE TABLE IF NOT EXISTS store_applied (
  store           TEXT COLLATE "C" NOT NULL,
  scope_key       TEXT COLLATE "C" NOT NULL,
  idempotency_key TEXT COLLATE "C" NOT NULL,
  op              TEXT NOT NULL,
  result          TEXT NOT NULL,
  PRIMARY KEY (store, idempotency_key)
);
CREATE INDEX IF NOT EXISTS store_applied_partition ON store_applied (store, scope_key);
COMMIT;
`;

/**
 * Create it, and treat a creator that got there first as having created it.
 *
 * The second attempt runs against the tables the other opener has now committed,
 * so every `IF NOT EXISTS` is the no-op it says it is. One retry rather than a
 * loop: what is being waited out is a single concurrent `CREATE`, and a server
 * that answers a duplicate a second time is telling us something other than
 * "somebody else is creating this".
 *
 * The `ROLLBACK` is not optional tidying. The DDL is one simple query carrying
 * its own `BEGIN`, so a statement that fails leaves the session in an aborted
 * transaction in which every later statement — the retry, and every store op
 * after it — is refused with `25P02`.
 */
async function createPostgresStoreSchema(client: Client): Promise<void> {
  for (let attempt = 0; ; attempt += 1) {
    try {
      await client.query(POSTGRES_STORE_SCHEMA);
      return;
    } catch (error) {
      await client.query("ROLLBACK").catch(() => undefined);
      const code = (error as { code?: unknown } | null)?.code;
      if (attempt > 0 || typeof code !== "string" || !POSTGRES_CONCURRENT_CREATOR.includes(code)) {
        throw error;
      }
    }
  }
}

/**
 * Postgres numbers its parameters, so the statements' `?`s are counted off.
 *
 * Nothing in `RemoteKv` writes a `?` inside a string literal, which is what
 * makes counting them enough — and what a statement that ever needed one would
 * have to change here first.
 */
function numberedStoreBind(sql: string): string {
  let next = 0;
  return sql.replace(/\?/g, () => `$${++next}`);
}

/** Postgres speaks the standard upsert in both of its forms. */
const POSTGRES_STORE_DIALECT: StoreDialect = {
  bind: numberedStoreBind,
  overwrite: standardOverwrite,
  ignore: standardIgnore,
};

/**
 * Whether an error a statement was rejected with means the **connection** is
 * gone rather than that the statement was refused.
 *
 * `FATAL` is Postgres's own severity for exactly that: a FATAL message aborts
 * the session that received it, so this socket is on its way out whatever the
 * code beside it says — `57P01` for an administrator's `pg_terminate_backend`,
 * `57P02` for a crash shutdown, `08006` for a connection failure. An ordinary
 * refusal is `ERROR` and leaves the session alone, so a unique violation or a
 * value too long stays what it is: this statement's failure, and this store's to
 * report as one.
 *
 * Read on the statement and not only on the client's `error` event because the
 * two arrive in that order rather than the other. `Client._handleErrorMessage`
 * hands a FATAL to the query in flight and returns; the `error` event follows
 * only when the socket itself ends, a turn of the event loop later, by which
 * time this statement has already settled. The eviction happens either way here,
 * because `Client._handleErrorEvent` emits unconditionally once the socket goes;
 * what this adds is that the op which *met* the loss is refused with the
 * sentence naming the variable an operator can change rather than with the
 * driver's own.
 */
function postgresConnectionIsGone(error: unknown): boolean {
  return (error as { severity?: unknown } | null)?.severity === "FATAL";
}

/** One Postgres server, as the connection a dialled `kv` store runs over. */
class PostgresStoreDriver implements StoreDriver {
  readonly dialect = POSTGRES_STORE_DIALECT;
  readonly #client: Client;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: { error?: Error };
  /**
   * The **name** of the variable this connection's address was read from, which
   * is what every message about it names — see [`storeConnectionLost`].
   */
  readonly #where: string;
  /** What drops this connection from the dialled cache. See [`#reported`]. */
  readonly #lost: () => void;

  constructor(client: Client, fault: { error?: Error }, where: string, lost: () => void) {
    this.#client = client;
    this.#fault = fault;
    this.#where = where;
    this.#lost = lost;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    try {
      const answered = await this.#client.query(sql, [...parameters]);
      return answered.rows as Record<string, unknown>[];
    } catch (error) {
      throw this.#reported(error);
    }
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<number> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    try {
      const answered = await this.#client.query(sql, [...parameters]);
      // `rowCount` is `null` on a statement that has no rows to count — `BEGIN`,
      // `COMMIT`, `ROLLBACK` — and the only callers that read it are the ones
      // whose statements do have them.
      return answered.rowCount ?? 0;
    } catch (error) {
      throw this.#reported(error);
    }
  }

  async close(): Promise<void> {
    await this.#client.end();
  }

  /**
   * What a rejected statement is really reported as.
   *
   * A server's refusal is itself. A connection that ended under the statement
   * ([`postgresConnectionIsGone`]) is the fault recorded, so every later
   * statement over this connection is refused with the same sentence, and
   * `lost()` called, so the retry the node's `retry:` makes runs over a
   * connection dialled again — which is what [`storeConnectionLost`] promises
   * the operator in so many words.
   *
   * Done **once**: a fault is permanent, `lost()` is the caller's whole response
   * to it, and the client's own `error` event will arrive afterwards saying the
   * same thing.
   */
  #reported(error: unknown): unknown {
    if (!postgresConnectionIsGone(error)) return error;
    if (this.#fault.error === undefined) {
      this.#fault.error = storeConnectionLost("postgres", this.#where, error);
      this.#lost();
    }
    return this.#fault.error;
  }
}

/**
 * Open one Postgres store connection: a socket and the schema.
 *
 * **One `Client` rather than a pool**, and here that is a statement about
 * transactions rather than about a guard. A keyed write is a `BEGIN`, an effect
 * and a ledger insert, and those three have to be one session — a pool hands
 * sessions out and takes them back between statements, so a transaction spread
 * over one would be three statements on up to three sessions. `RemoteKv` runs
 * every op on one queue over this connection, which is what makes a transaction
 * a transaction; two *processes* stay concurrent, which is the multi-writer
 * posture this store is for.
 *
 * `lost` is what the caller does with a connection this one reports gone — see
 * [`storeConnectionLost`].
 */
async function openPostgresStore(
  url: string,
  where: string,
  lost: () => void,
): Promise<StoreDriver> {
  const client = new Client({
    connectionString: url,
    // This side's own probes: a process whose store server has gone finds out on
    // the socket rather than on the next op's timeout.
    keepAlive: true,
  });
  const fault: { error?: Error } = {};
  // **Before `connect`**, because `pg` emits `error` on the client whenever it
  // loses the socket with no query in flight — `Client._handleErrorEvent` does
  // it unconditionally — and an `EventEmitter` that emits `error` with nothing
  // listening ends the process.
  client.on("error", (reported: unknown) => {
    fault.error ??= storeConnectionLost("postgres", where, reported);
    // …and the connection goes with it, so the next op dials a fresh one rather
    // than inheriting this error for the life of the process.
    lost();
  });
  await client.connect();
  try {
    await createPostgresStoreSchema(client);
  } catch (error) {
    await client.end().catch(() => undefined);
    throw error;
  }
  return new PostgresStoreDriver(client, fault, where, lost);
}

STORE_BACKENDS.postgres = openPostgresStore;
