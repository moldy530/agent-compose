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
 * is the case to read §11.2 twice for, exactly as it is there. A **table** added
 * in one is not: `lineage` (PRD resolved q65) arrived after this schema first
 * shipped, and `IF NOT EXISTS` is the whole of its migration into a database an
 * earlier build created.
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
  session       TEXT COLLATE "C",
  outcome       TEXT COLLATE "C",
  payload       TEXT,
  parked_at     TEXT COLLATE "C" NOT NULL,
  dispatched_at TEXT,
  settled_at    TEXT,
  detail        TEXT,
  PRIMARY KEY (execution, wait)
);
CREATE TABLE IF NOT EXISTS lineage (
  execution       TEXT COLLATE "C" PRIMARY KEY,
  parent          TEXT COLLATE "C" NOT NULL,
  idempotency_key TEXT COLLATE "C" NOT NULL,
  item_index      INTEGER,
  admission       TEXT
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

/**
 * How the server is asked to notice that the far end of this session is gone.
 *
 * `tcp_keepalives_*` rather than `idle_session_timeout`, and that is the
 * difference between "this host is unreachable" and "this host is quiet". The
 * keepalive probes are answered by the peer's **kernel**, so a hub whose event
 * loop is wedged behind a long step still keeps its session; only a machine that
 * is really gone fails to answer. A timeout measured on the application's own
 * silence would reap the first of those too, which is the failure that costs the
 * record rather than a few minutes of a takeover.
 *
 * The three compose to [`GUARD_REAP_SECONDS`]: the server waits
 * `tcp_keepalives_idle` seconds of silence, then sends `tcp_keepalives_count`
 * probes `tcp_keepalives_interval` apart before it ends the session and drops
 * every advisory lock on it.
 */
const KEEPALIVE_IDLE_SECONDS = 60;

/** …how far apart the probes after that silence are. */
const KEEPALIVE_INTERVAL_SECONDS = 20;

/** …and how many of them go unanswered before the session is ended. */
const KEEPALIVE_PROBES = (GUARD_REAP_SECONDS - KEEPALIVE_IDLE_SECONDS) / KEEPALIVE_INTERVAL_SECONDS;

/**
 * The settings, as one statement. All three are `USERSET`, so a session sets its
 * own without any privilege beyond the one it connected with.
 */
const POSTGRES_KEEPALIVES = `SET tcp_keepalives_idle = ${KEEPALIVE_IDLE_SECONDS}; SET tcp_keepalives_interval = ${KEEPALIVE_INTERVAL_SECONDS}; SET tcp_keepalives_count = ${KEEPALIVE_PROBES}`;

/**
 * What makes "committed" mean "in the write-ahead log" rather than "in a buffer
 * the next power cut takes with it".
 *
 * The SQLite arm sets `PRAGMA synchronous = FULL` for exactly this, and says so
 * in one line; this is the same sentence spoken to the other server.
 * `docs/durability.md` §2.1 promises that one effect is one `INSERT`
 * **committed before it is answered**, and §2.3 spells that out as "each
 * statement is its own transaction and the server has written it before it
 * answers". On a server whose `synchronous_commit` is `off` — settable
 * cluster-wide in `postgresql.conf`, per database with `ALTER DATABASE … SET`
 * and per role with `ALTER ROLE … SET`, and the default on more than one managed
 * provider's "fast" tier — an `INSERT` into `effects` answers the hub before its
 * WAL record is flushed, and a crash or a power loss discards up to roughly
 * three times `wal_writer_delay` of effects this journal has already claimed to
 * hold. A replay then re-issues model calls and tool invocations whose answers
 * the record said were down, which is the one thing §2.1's per-record atomicity
 * rules out — and the crash window §2.3 calls "the round trip" would be silently
 * wider than one.
 *
 * `USERSET` like the three above, so this costs no privilege either. It is the
 * arm's standing rule, the same one the MySQL side applies to `sql_mode` and
 * `autocommit`: **a session setting this journal's statements rest on is stated
 * by the arm rather than assumed of the server**. Unlike the reap window it is
 * not best-effort — a server that refuses it fails the open, because a journal
 * that cannot promise §2.1 is not this journal.
 */
const POSTGRES_SYNCHRONOUS_COMMIT = "SET synchronous_commit = on";

/**
 * …and what the session really carries once both statements have been sent.
 *
 * Read back rather than assumed, because on this backend a `SET` that *succeeds*
 * is not a setting that took. The assign hooks for the three keepalive GUCs call
 * `pq_setkeepalives*` and **discard the return value**: a platform without
 * `TCP_KEEPIDLE` logs "setting the keepalive idle time is not supported" on the
 * server and the `SET` still answers, and a Unix-domain-socket connection is a
 * documented no-op that also answers. In both cases `SHOW tcp_keepalives_idle`
 * reads back `0` — which is the only place the difference is visible from here.
 *
 * Without this read the `.catch` in [`openPostgres`] is unreachable, so
 * [`guardWindowUnshortened`] never prints, and a hub whose host vanished holds
 * its `pg_try_advisory_lock` until Linux's own two-hour `tcp_keepalive_time`
 * reaps the backend — while the takeover `serve` is refused by [`guardHeld`]'s
 * text promising that "a host that vanished outright releases it within 300s".
 * The operator waits five minutes, is refused again, and the only explanation
 * the message offers is a live `serve` that does not exist.
 */
const POSTGRES_SESSION_HELD =
  "SELECT current_setting('synchronous_commit') AS synchronous_commit, " +
  "current_setting('tcp_keepalives_idle') AS idle, " +
  "current_setting('tcp_keepalives_interval') AS probe_interval, " +
  "current_setting('tcp_keepalives_count') AS probes";

/** What an unflushed server is refused with. See [`POSTGRES_SYNCHRONOUS_COMMIT`]. */
function writesAreNotFlushed(): Error {
  return new Error(
    `this project's journal is ${journalLocation()}, and that server answers before it has written: \`synchronous_commit\` is \`off\` on this session even though it was asked for \`on\`. Every effect this journal records would be acknowledged out of a buffer, so a crash or a power loss would drop records the run has already treated as down and a replay would re-issue the model calls and tool invocations behind them — which is the one thing \`docs/durability.md\` §2.1 promises it will not do. Take the \`synchronous_commit = off\` off this database or role (\`ALTER DATABASE … SET\`, \`ALTER ROLE … SET\`, \`postgresql.conf\`), or point \`journal:\` at a server that flushes`,
  );
}

/**
 * Read the session back, and say what did not take.
 *
 * Two severities, because two different promises are at stake. The flush is
 * §2.1's and is not negotiable, so a server that will not make it is refused by
 * name. The reap window is §2.3's five-minute bound on a *lockout*, which costs
 * a takeover minutes rather than costing the record — §2.3 says in as many words
 * that a server which will not take it is "warned about on stderr and opened
 * anyway".
 */
async function confirmPostgresSession(client: Client): Promise<void> {
  let held: Record<string, unknown> | undefined;
  try {
    const answered = await client.query(POSTGRES_SESSION_HELD);
    held = answered.rows[0] as Record<string, unknown> | undefined;
  } catch (error) {
    // The read is the check, so a read that failed is a window nothing
    // confirmed — which is what the warning says.
    guardWindowUnshortened(error);
    return;
  }
  if (held?.["synchronous_commit"] === "off") throw writesAreNotFlushed();
  const asked: readonly (readonly [string, string, number])[] = [
    ["tcp_keepalives_idle", "idle", KEEPALIVE_IDLE_SECONDS],
    ["tcp_keepalives_interval", "probe_interval", KEEPALIVE_INTERVAL_SECONDS],
    ["tcp_keepalives_count", "probes", KEEPALIVE_PROBES],
  ];
  const missed = asked.filter(([, column, seconds]) => Number(held?.[column]) !== seconds);
  if (missed.length === 0) return;
  guardWindowUnshortened(
    `the server took the settings and this session carries ${missed
      .map(([name, column]) => `\`${name}\` = ${String(held?.[column] ?? "")}`)
      .join(", ")}`,
  );
}

/** The journal as a Postgres database, on one connection this process holds. */
class PostgresDriver implements JournalDriver {
  readonly dialect = POSTGRES_DIALECT;
  readonly #client: Client;
  /** What the connection reported, if it has reported anything. */
  readonly #fault: ConnectionFault;
  /** What keeps a shortened reap window from reaping an idle hub. */
  readonly #heartbeat: ReturnType<typeof setInterval>;

  constructor(client: Client, fault: ConnectionFault) {
    this.#client = client;
    this.#fault = fault;
    this.#heartbeat = setInterval(() => {
      // Nothing reads the answer and nothing acts on the failure: a round trip
      // is the whole point, and a connection that could not make one has
      // already told the listener in [`openPostgres`]. The rejection is caught
      // so that a lost connection is one refused statement rather than an
      // unhandled rejection.
      void this.#client.query("SELECT 1").catch(() => undefined);
    }, GUARD_HEARTBEAT_MS);
    this.#heartbeat.unref();
  }

  async all(sql: string, parameters: readonly Bound[]): Promise<Row[]> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    const answered = await this.#client.query(sql, [...parameters]);
    return answered.rows as Row[];
  }

  async run(sql: string, parameters: readonly Bound[]): Promise<void> {
    if (this.#fault.error !== undefined) throw this.#fault.error;
    await this.#client.query(sql, [...parameters]);
  }

  async close(): Promise<void> {
    // The advisory lock goes with the session, so ending it is releasing the
    // guard — there is nothing to unlock, and a session this process never got
    // to end is reaped by the server inside [`GUARD_REAP_SECONDS`] (see
    // [`guardHeld`]).
    clearInterval(this.#heartbeat);
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
 * **The session states what it rests on rather than inheriting it**, which is
 * the MySQL arm's rule read on this backend: `synchronous_commit` decides
 * whether "committed" means "in the write-ahead log" or "in a buffer", and it is
 * settable per cluster, per database and per role — so it is asked for by name
 * ([`POSTGRES_SYNCHRONOUS_COMMIT`]) and read back with the reap window
 * ([`confirmPostgresSession`]) rather than assumed of the server.
 *
 * The guard is taken with the `try` form rather than the blocking one: a second
 * opener is told what is happening ([`guardHeld`]) rather than left blocking on
 * a connection that may be a `serve` which will hold it for days.
 *
 * **The guard is taken before the schema is created**, and the order is load
 * bearing rather than tidy. Two processes opening one fresh journal at the same
 * time — a `serve` restart overlapping the one it replaces, an
 * `agent-compose resume` typed beside a live `serve` — would otherwise both run
 * the DDL, and `CREATE TABLE IF NOT EXISTS` is documented as *not* atomic
 * against a concurrent creator: one of the two can fail on a duplicate key in
 * `pg_type`, which is a catalog error rather than the refusal this design
 * promises. Under the guard there is one creator by construction, and the opener
 * that is about to be refused runs no DDL against an operator's database at all.
 */
async function openPostgres(): Promise<JournalDriver> {
  const client = new Client({
    connectionString: journalUrl(),
    // This side's own probes, which are the other direction of the same
    // question the server-side settings below ask: a hub whose database has
    // gone finds out on the heartbeat rather than on the next execution.
    keepAlive: true,
    keepAliveInitialDelayMillis: GUARD_HEARTBEAT_MS,
  });
  const fault: ConnectionFault = {};
  // **Before `connect`**, because `pg` emits `error` on the client whenever it
  // loses the socket with no query in flight — `Client._handleErrorEvent` does
  // it unconditionally — and an `EventEmitter` that emits `error` with nothing
  // listening ends the process. See [`ConnectionFault`].
  client.on("error", (reported: unknown) => {
    fault.error ??= connectionLost(reported);
  });
  await client.connect();
  try {
    // Best effort, and said out loud where it does not take: a `SET` the server
    // refuses lands in this `catch`, and a `SET` it accepts without applying
    // lands in [`confirmPostgresSession`]'s read-back. Both reach
    // [`guardWindowUnshortened`], and on this backend the second is the path
    // that really happens.
    await client.query(POSTGRES_KEEPALIVES).catch(guardWindowUnshortened);
    // Not best-effort: §2.1's "committed before it is answered" rests on it, so
    // a server that refuses it refuses the open. See
    // [`POSTGRES_SYNCHRONOUS_COMMIT`].
    await client.query(POSTGRES_SYNCHRONOUS_COMMIT);
    await confirmPostgresSession(client);
    const guard = await client.query("SELECT pg_try_advisory_lock($1, $2) AS taken", [
      WRITER_GUARD_KEYS[0],
      WRITER_GUARD_KEYS[1],
    ]);
    const taken = (guard.rows[0] as { taken?: unknown } | undefined)?.taken;
    if (taken !== true) throw guardHeld();
    await client.query(POSTGRES_SCHEMA);
  } catch (error) {
    await client.end().catch(() => undefined);
    throw error;
  }
  return new PostgresDriver(client, fault);
}

BACKENDS.postgres = openPostgres;
