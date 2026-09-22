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
 * The schema, created on first open and never migrated.
 *
 * `IF NOT EXISTS` throughout, so a second open of a store this build already
 * created does nothing — which is all a physical schema can be asked for here: a
 * dialled store is created by this release or a later one, so there is no older
 * database whose columns have to be probed for.
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
`;

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

/** One Postgres server, as the connection a dialled `kv` store runs over. */
class PostgresStoreDriver implements StoreDriver {
  readonly dialect = POSTGRES_STORE_DIALECT;
  readonly #client: Client;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: { error?: Error };

  constructor(client: Client, fault: { error?: Error }) {
    this.#client = client;
    this.#fault = fault;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const answered = await this.#client.query(sql, [...parameters]);
    return answered.rows as Record<string, unknown>[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<number> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const answered = await this.#client.query(sql, [...parameters]);
    // `rowCount` is `null` on a statement that has no rows to count — `BEGIN`,
    // `COMMIT`, `ROLLBACK` — and the only callers that read it are the ones
    // whose statements do have them.
    return answered.rowCount ?? 0;
  }

  async close(): Promise<void> {
    await this.#client.end();
  }
}

/**
 * What a statement is refused with once a store's connection has been lost.
 *
 * **Refused rather than redialled**, which on a store is a narrower claim than
 * on the journal: there is no guard to hand to another process, so the reason is
 * the simpler one — a command whose connection died mid-transaction does not
 * know whether its write landed, and a silent redial would answer as though it
 * had. The node fails under its own `retry:`/`on_error:` (grammar 9.2), and a
 * retry carrying the idempotency key of grammar 9.4 is exactly what makes that
 * safe: the ledger already holds the first attempt, or it does not.
 */
function storeConnectionLost(where: string, cause: unknown): Error {
  const detail = cause instanceof Error ? cause.message : String(cause);
  return new Error(
    `this project lost its connection to the \`postgres\` store backend at \`\${${where}}\`: ${detail}. It is not redialled inside the op that failed — a write whose connection died is one nothing can say landed or did not — so this op fails and the node's own \`retry:\` decides what happens next; a retry carries the idempotency key its first attempt carried, which is what makes it apply once (grammar 9.4, PRD 5.8)`,
  );
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
 */
async function openPostgresStore(url: string, where: string): Promise<StoreDriver> {
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
    fault.error ??= storeConnectionLost(where, reported);
  });
  await client.connect();
  try {
    await client.query(POSTGRES_STORE_SCHEMA);
  } catch (error) {
    await client.end().catch(() => undefined);
    throw error;
  }
  return new PostgresStoreDriver(client, fault);
}

STORE_BACKENDS.postgres = openPostgresStore;
