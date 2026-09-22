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
 * store openers queue behind the journal's writer.
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
 * **Refused rather than redialled *inside the op*.** On a store that is a
 * narrower claim than on the journal: there is no guard to hand to another
 * process, so the reason is the simpler one — a command whose connection died
 * mid-transaction does not know whether its write landed, and a silent redial
 * under it would answer as though it had. The node fails under its own
 * `retry:`/`on_error:` (grammar 9.2), and a retry carrying the idempotency key
 * of grammar 9.4 is exactly what makes that safe: the ledger already holds the
 * first attempt, or it does not.
 *
 * **And the connection is dropped, so the retry has one to run over.** Neither
 * `pg` nor `mysql2` reconnects on its own and this fault is permanent once set,
 * so a connection left in the dialled cache would refuse every op of every later
 * execution with this same error until the process restarted — which on a
 * `serve`, the deployment a dialled store exists for, is for ever. The
 * journal's identical stickiness is deliberate, because a lost session there is
 * a lost writer guard and redialling would fork the record; a store has no guard
 * and nothing to fork, so the narrow claim above is the whole of it.
 */
function storeConnectionLost(where: string, cause: unknown): Error {
  const detail = cause instanceof Error ? cause.message : String(cause);
  return new Error(
    `this project lost its connection to the \`postgres\` store backend at \`\${${where}}\`: ${detail}. It is not redialled inside the op that failed — a write whose connection died is one nothing can say landed or did not — so this op fails and the node's own \`retry:\` decides what happens next; a retry carries the idempotency key its first attempt carried, which is what makes it apply once (grammar 9.4, PRD 5.8), and it runs over a connection dialled again rather than over this one`,
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
    fault.error ??= storeConnectionLost(where, reported);
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
  return new PostgresStoreDriver(client, fault);
}

STORE_BACKENDS.postgres = openPostgresStore;
