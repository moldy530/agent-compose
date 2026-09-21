// ---------------------------------------------------------------------------
// The Postgres arm (grammar §14.7, PRD resolved q62)
// ---------------------------------------------------------------------------
//
// Emitted only into a project whose target binds `provider: postgres`, which is
// also the only project whose `package.json` pins `pg`. A build that binds
// SQLite carries neither the driver nor this code, which is what keeps the
// zero-infra guarantee a property of the artifact rather than of a code path
// nobody takes.
//
// Nothing here is a second implementation of anything. The statements are
// `SqlJournal`'s and the contract is `docs/durability.md`'s; what this file adds
// is a connection, a schema, and the writer guard.

import { Client } from "pg";

/**
 * The schema, created on first open and never migrated.
 *
 * `IF NOT EXISTS` throughout, so a second open of a journal this build already
 * created does nothing — which is the whole of what `docs/durability.md` §11.2
 * asks of a physical schema, and all it can ask of this one: a remote journal is
 * created by this release or a later one, so there is no older file whose
 * columns have to be probed for. (The SQLite arm's `PRAGMA table_info` walks
 * exist because there *are* older files.) A column added here in a later release
 * is the case to read §11.2 twice for, exactly as it is there.
 *
 * **`COLLATE "C"` on every column a statement compares or orders by**, and that
 * is a correctness choice rather than a preference. A journal key is
 * `<site>#<kind>/<ordinal>` and a site is an instance path (grammar §9.4): two
 * keys differing in case are two effects, `effectsUnder` matches a prefix with
 * `substr`, and `ORDER BY "key"` has to be the order `docs/durability.md` §4
 * derives. A database created under a linguistic default collation would fold
 * punctuation and case in both — silently, and only for some rows.
 *
 * `seq` is the insertion order `docs/distributed.md` §6.2's park order breaks
 * ties with, which SQLite gets from its implicit `rowid` and this has to
 * declare. Rows are inserted and never deleted, so it is exactly monotonic.
 */
const POSTGRES_SCHEMA = `
CREATE TABLE IF NOT EXISTS executions (
  id              TEXT COLLATE "C" PRIMARY KEY,
  flow            TEXT NOT NULL,
  trigger_kind    TEXT NOT NULL,
  inputs          TEXT NOT NULL,
  session_key     TEXT NOT NULL,
  callback        TEXT,
  traceparent     TEXT,
  status          TEXT COLLATE "C" NOT NULL,
  journal_version INTEGER NOT NULL,
  started_at      TEXT COLLATE "C" NOT NULL,
  ended_at        TEXT,
  error           TEXT
);
CREATE TABLE IF NOT EXISTS effects (
  execution   TEXT COLLATE "C" NOT NULL,
  "key"       TEXT COLLATE "C" NOT NULL,
  site        TEXT COLLATE "C" NOT NULL,
  kind        TEXT COLLATE "C" NOT NULL,
  ordinal     INTEGER NOT NULL,
  request     TEXT NOT NULL,
  outcome     TEXT COLLATE "C" NOT NULL,
  payload     TEXT NOT NULL,
  refused     SMALLINT NOT NULL DEFAULT 0,
  recorded_at TEXT NOT NULL,
  PRIMARY KEY (execution, "key")
);
CREATE TABLE IF NOT EXISTS deliveries (
  execution    TEXT COLLATE "C" NOT NULL,
  ordinal      INTEGER NOT NULL,
  kind         TEXT COLLATE "C",
  trigger_kind TEXT,
  event        TEXT COLLATE "C" NOT NULL,
  url          TEXT NOT NULL,
  body         TEXT NOT NULL,
  pauses       TEXT NOT NULL,
  status       TEXT COLLATE "C" NOT NULL,
  attempts     TEXT NOT NULL,
  intended_at  TEXT COLLATE "C" NOT NULL,
  settled_at   TEXT,
  detail       TEXT,
  PRIMARY KEY (execution, ordinal)
);
CREATE TABLE IF NOT EXISTS dispatches (
  seq           BIGSERIAL NOT NULL UNIQUE,
  execution     TEXT COLLATE "C" NOT NULL,
  wait          TEXT COLLATE "C" NOT NULL,
  id            TEXT COLLATE "C" NOT NULL,
  placement     TEXT NOT NULL,
  node          TEXT NOT NULL,
  site          TEXT COLLATE "C" NOT NULL,
  inputs        TEXT NOT NULL,
  item_index    INTEGER,
  history       TEXT,
  policy        TEXT,
  status        TEXT COLLATE "C" NOT NULL,
  session       TEXT,
  outcome       TEXT COLLATE "C",
  payload       TEXT,
  parked_at     TEXT COLLATE "C" NOT NULL,
  dispatched_at TEXT,
  settled_at    TEXT,
  detail        TEXT,
  PRIMARY KEY (execution, wait)
);
CREATE INDEX IF NOT EXISTS effects_of_execution ON effects (execution);
CREATE INDEX IF NOT EXISTS executions_by_status ON executions (status, started_at);
CREATE INDEX IF NOT EXISTS deliveries_by_status ON deliveries (status, intended_at);
CREATE INDEX IF NOT EXISTS dispatches_by_id ON dispatches (id);
CREATE INDEX IF NOT EXISTS dispatches_by_status ON dispatches (status, parked_at);
`;

/**
 * Postgres numbers its parameters, so the statements' `?`s are counted off.
 *
 * Nothing in `SqlJournal` writes a `?` inside a string literal — every literal
 * it spells is a status word — which is what makes counting them enough, and
 * what a statement that ever needed one would have to change here first.
 */
function numberedBind(sql: string): string {
  let next = 0;
  return sql.replace(/\?/g, () => `$${++next}`);
}

/** Postgres speaks the standard upsert, and orders dispatches by `seq`. */
const POSTGRES_DIALECT: Dialect = {
  provider: "postgres",
  conflict: standardConflict,
  bind: numberedBind,
  insertionOrder: "seq",
};

/** The journal as a Postgres database, on one connection this process holds. */
class PostgresDriver implements JournalDriver {
  readonly dialect = POSTGRES_DIALECT;
  readonly #client: Client;

  constructor(client: Client) {
    this.#client = client;
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Row[]> {
    const answered = await this.#client.query(sql, [...parameters]);
    return answered.rows as Row[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<void> {
    await this.#client.query(sql, [...parameters]);
  }

  async close(): Promise<void> {
    // The advisory lock goes with the session, so ending it is releasing the
    // guard — there is nothing to unlock and nothing left holding it if this
    // never runs (see [`guardHeld`]).
    await this.#client.end();
  }
}

/**
 * Open the Postgres journal: one connection, the schema, and the writer guard.
 *
 * **One `Client` rather than a pool**, and that is the design rather than a
 * simplification. The journal has exactly one writer (`docs/durability.md` §2,
 * PRD resolved q42), every statement it runs is a single short one, and
 * `SqlJournal` already serializes them — so a pool would buy no parallelism this
 * module can use while making the writer guard unholdable: `pg_advisory_lock` is
 * **session**-scoped, and a pool hands sessions out and takes them back.
 *
 * The guard is taken with `pg_try_advisory_lock` rather than `pg_advisory_lock`:
 * a second opener is told what is happening ([`guardHeld`]) rather than left
 * blocking on a connection that may be a `serve` which will hold it for days.
 */
async function openPostgres(): Promise<JournalDriver> {
  const client = new Client({ connectionString: journalUrl() });
  await client.connect();
  try {
    await client.query(POSTGRES_SCHEMA);
    const guard = await client.query("SELECT pg_try_advisory_lock($1, $2) AS taken", [
      WRITER_GUARD_KEYS[0],
      WRITER_GUARD_KEYS[1],
    ]);
    const taken = (guard.rows[0] as { taken?: unknown } | undefined)?.taken;
    if (taken !== true) throw guardHeld();
  } catch (error) {
    await client.end().catch(() => undefined);
    throw error;
  }
  return new PostgresDriver(client);
}

BACKENDS.postgres = openPostgres;
