//
// The execution journal: what makes a run survive the process that started it
// (PRD 5.11, resolved q26–q29, `docs/durability.md`).
//
// `./runtime.ts` is what a node *does*; this module is the record of what it
// did. Every effect a compiled graph issues — a model call, a tool execution, a
// store op, a `human` answer — is written here as it happens, and a resumed
// execution re-runs the same graph consuming that record read-only up to the
// frontier (the first effect the journal does not hold). It is byte-identical
// in every project this compiler release builds, like `./runtime.ts`,
// `./cel.ts` and `./stores.ts`.
//
// # The journal is not the trace
//
// They share a keying discipline and nothing else. `docs/trace.md` §11 keeps
// model completions, tool results, human answers and secrets **out** of the
// trace, which is an observability artifact a reader may ship anywhere; replay
// needs exactly those payloads, byte for byte, or it cannot answer a recorded
// effect without re-issuing it. So the journal is a second artifact with the
// same sensitivity as this project's stores: private recovery data, never
// uploaded, never rendered, deleted by deleting the file. `docs/durability.md`
// §8 is normative for that posture; nothing here weakens `docs/trace.md` §11.
//
// What the two *do* share is identity. An effect is addressed by the instance
// path of grammar 9.4 — the frames `docs/trace.md` §8 describes — so a key read
// out of a trace and a key read out of a journal name the same effect site.
// This module invents no second addressing scheme.
//
// # Where it lives
//
// One SQLite file per project, beside the project's stores:
//
// ```text
// <project>/.agent-compose/journal.sqlite
// ```
//
// `AGENT_COMPOSE_DATA_DIR` moves the whole directory, exactly as it moves the
// stores. The path is stable across `run`, `serve` and `resume` of one project
// and one target, which is what makes `agent-compose resume <execution>` find
// the execution a crashed `run` left behind (resolved q27: "beside the project",
// "one file to delete").
//
// # The driver
//
// `node-sqlite3-wasm`, the same one `./stores.ts` opens, for the reason PRD
// §9.18 gives: a generated module runs on Bun **and** on Node, so it may not
// reach for `bun:sqlite` or `node:sqlite`. It is already pinned, so the journal
// costs no new dependency.
//
// # Atomicity, and what a crash can leave
//
// One effect is one `INSERT`, which SQLite runs in an implicit transaction of
// its own: a process that dies mid-write leaves the row absent, never half
// present, so nothing in this file can parse as a complete record that is not
// one. `synchronous = FULL` is what makes "committed" mean "on the disk"
// rather than "in the page cache". SQLite's **rollback journal** is what backs
// that, and a write-ahead log is deliberately not asked for: this driver's
// virtual file system does not implement one — the pragma is accepted and
// leaves the mode at `delete` — so asking would be a line that reads like a
// guarantee and is not one. Atomicity per statement, which is the property
// this file needs, is the same either way.
//
// What a crash *can* leave is an effect that happened with no row for it: the
// row is written when the effect answers, and the window between the two is
// not closable by anything on this side of the network. A replay re-executes
// such an effect, which is the at-least-once compromise grammar 9.4's
// idempotency keys exist for — the key a repeated write carries is the one the
// first attempt carried, so the receiver that dedupes still does.
//
// # Concurrency
//
// A `serve` process runs many executions at once, and every statement here is
// synchronous: `node-sqlite3-wasm` blocks the event loop for the duration of a
// call, so two executions can never interleave inside one statement and no
// intra-process locking is needed.
//
// Across *processes* this release keeps the boundary `./stores.ts` keeps and
// PRD 5.10 draws — `--target local` is one process, and **one process at a time
// writes a project's journal**. That stays a rule rather than a promise, but it
// is not an unenforced one: `node-sqlite3-wasm` takes SQLite's exclusive lock by
// creating `<file>.lock` as a directory and gives it back by removing it, so a
// second process meets `SQLITE_BUSY` rather than interleaving into pages the
// first has not committed. `busy_timeout` is set before any statement that can
// contend, and the open is retried under a deadline for the statements the
// pragma does not cover.
//
// What that lock does not do is die with its owner. A process killed **inside**
// a write never reaches the `rmdir`, and the directory it leaves would refuse
// every later open of that journal — the resume of the interrupted execution
// and every future run of the project alike. So a lock still held after this
// process has waited the deadline out is treated as the corpse it is under the
// rule above, and removed ([`LOCK_DIRECTORY`]). The rollback journal the same
// crash leaves is untouched: SQLite recovers it on the next open.
//
// So the two live surfaces are kept apart rather than serialized. `serve`
// recovers every open execution at start, which means an execution a live
// `serve` holds is already being replayed by it and is finished through
// `POST /executions/:id/resume`; `agent-compose resume` is for an execution
// **no live process is running** — the one a crashed `run` left behind.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

// ---------------------------------------------------------------------------
// Version
// ---------------------------------------------------------------------------

/**
 * The journal format's version, as `docs/durability.md` §11 pins it.
 *
 * Written on every execution row, and read back when one is replayed: a journal
 * written by an older compiler release is refused by version rather than
 * misread. It moves under `docs/durability.md` §11's rules, which are
 * `docs/trace.md` §10's applied to this artifact.
 */
export const JOURNAL_VERSION = 1;

// ---------------------------------------------------------------------------
// Where the data lives
// ---------------------------------------------------------------------------

/** The variable that moves a project's whole data directory. */
export const DATA_DIRECTORY = "AGENT_COMPOSE_DATA_DIR";

/**
 * The emitted project's root: the directory `src/` sits in.
 *
 * Derived from this module's own URL rather than from `process.cwd()`, because
 * a run's journal must not depend on where a process happened to be started:
 * `bun src/index.ts run …` from the project directory and the same command from
 * a repository root three levels up have to address one journal, or a `resume`
 * typed from the other directory would find nothing.
 */
const PROJECT_ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

/**
 * Where this project keeps what it has to survive a restart.
 *
 * Declared here rather than in `./stores.ts` — which re-exports it — because
 * this module is the leaf: `./stores.ts` imports `./runtime.ts` and
 * `./runtime.ts` imports this file, so a journal that reached back for the
 * store module's copy would close a cycle for one path join.
 */
export function dataRoot(): string {
  const override = process.env[DATA_DIRECTORY];
  return override === undefined || override === ""
    ? path.join(PROJECT_ROOT, ".agent-compose")
    : path.resolve(override);
}

/** The journal file: one per project, whatever runs against it. */
export function journalPath(): string {
  return path.join(dataRoot(), "journal.sqlite");
}

// ---------------------------------------------------------------------------
// What is recorded
// ---------------------------------------------------------------------------

/**
 * The four families of effect a compiled graph issues.
 *
 * They are families rather than node kinds on purpose: what a replay has to
 * answer is a *call to the world*, and the same family is reached from more
 * than one construct — a `tool` is an `exec:`, an `http:` or a `function:`
 * binding, whether it is a node's own activity or a tool an agent's model
 * called. `docs/durability.md` §3 is the inventory, and names the site in
 * `./runtime.ts` each one is written at.
 */
export type EffectKind = "model" | "tool" | "store" | "human";

/** How one recorded effect ended. */
export type JournalOutcome =
  | { readonly kind: "value"; readonly value: unknown }
  | { readonly kind: "error"; readonly name: string; readonly message: string };

/** One effect, as the journal holds it. */
export interface JournalRecord {
  readonly execution: string;
  /** `<site>#<kind>/<ordinal>` — see [`EffectRecorder`]. */
  readonly key: string;
  /** The effect site's instance path (grammar 9.4), flattened. */
  readonly site: string;
  readonly kind: EffectKind;
  readonly ordinal: number;
  /** The canonical JSON of what identified the request. See [`canonical`]. */
  readonly request: string;
  readonly outcome: JournalOutcome;
  /** When it was recorded, as an ISO 8601 instant. */
  readonly recordedAt: string;
}

/** How an execution ended, or that it has not. */
export type ExecutionStatus = "open" | "completed" | "failed";

/** One execution's lifecycle row (`docs/durability.md` §3.5). */
export interface ExecutionRow {
  readonly id: string;
  readonly flow: string;
  /**
   * What started it: `manual` for `run` and for a `manual` trigger, the
   * trigger's own name for an `http` one.
   *
   * Recorded and never acted on. Resolved q28: "triggers fire once — recovery
   * replays executions that exist; it does not re-fire the trigger that created
   * them", so this is what a reader consults, not what recovery dispatches on.
   */
  readonly trigger: string;
  /** The invocation's inputs, as the flow's `inputs:` parsed them. */
  readonly inputs: Record<string, unknown>;
  /** The session identity `scope: session` stores key off (grammar 11.3). */
  readonly sessionKey: string;
  /**
   * Where this execution's completion webhook goes, for an `async` `http`
   * trigger that asked for one (grammar 13.3's `callback:`) — and absent for
   * every other invocation.
   *
   * Recorded because the process that **finishes** an execution need not be the
   * one that started it: `serve` recovers every open execution at start
   * (`docs/durability.md` §6.1), and a caller who was handed a `202` and is
   * waiting for a push has no other way to be told. The URL rather than the
   * request it came out of: it is what the webhook needs, and a whole HTTP
   * request kept for one field of it would be the largest private payload in
   * the journal and the least of it used (§8).
   */
  readonly callback?: string;
  readonly status: ExecutionStatus;
  readonly journalVersion: number;
  readonly startedAt: string;
  readonly endedAt?: string;
  /** Why it failed, for a reader of the journal. Absent on the other two. */
  readonly error?: string;
}

// ---------------------------------------------------------------------------
// The interface
// ---------------------------------------------------------------------------

/**
 * What the runtime asks of a journal, and the whole of it.
 *
 * One interface, one implementation in this release: resolved q27 makes the
 * journal a **deploy-target slot** — `--target local` binds SQLite, a
 * distributed target will bind Postgres — and every target this compiler can
 * currently build is process-local, so SQLite is what every project gets and
 * nothing in the composition says so. The shape is deliberately the shape a
 * Postgres implementation could take: keyed reads, one-row appends, and no
 * assumption that the store is a file or that the process owns it.
 */
export interface Journal {
  /** Record that an execution has begun. Idempotent on the id. */
  begin(row: ExecutionRow): void;
  /** Record how it ended. */
  end(id: string, status: Exclude<ExecutionStatus, "open">, error?: string): void;
  /** One execution's lifecycle row. */
  execution(id: string): ExecutionRow | undefined;
  /** Every execution the journal holds open, oldest first. */
  openExecutions(): readonly ExecutionRow[];
  /** What this execution recorded at `key`, if anything. */
  lookup(execution: string, key: string): JournalRecord | undefined;
  /** Append one effect. */
  append(record: JournalRecord): void;
}

// ---------------------------------------------------------------------------
// The SQLite implementation
// ---------------------------------------------------------------------------

type SqliteModule = typeof import("node-sqlite3-wasm");
type Database = InstanceType<SqliteModule["Database"]>;
type Row = Record<string, unknown>;

/**
 * The driver, loaded on first use.
 *
 * The `default` dance is `./stores.ts`'s and is here for its reason: the
 * package is CommonJS, so an ESM importer sees its exports under `default`,
 * while the ambient type declaration describes the CommonJS namespace directly.
 */
let driver: Promise<SqliteModule> | undefined;
function sqlite(): Promise<SqliteModule> {
  driver ??= (async () => {
    const loaded = (await import("node-sqlite3-wasm")) as unknown as SqliteModule & {
      readonly default?: SqliteModule;
    };
    return loaded.default ?? loaded;
  })();
  return driver;
}

const SCHEMA = `
CREATE TABLE IF NOT EXISTS executions (
  id              TEXT PRIMARY KEY,
  flow            TEXT NOT NULL,
  trigger_kind    TEXT NOT NULL,
  inputs          TEXT NOT NULL,
  session_key     TEXT NOT NULL,
  callback        TEXT,
  status          TEXT NOT NULL,
  journal_version INTEGER NOT NULL,
  started_at      TEXT NOT NULL,
  ended_at        TEXT,
  error           TEXT
);
CREATE TABLE IF NOT EXISTS effects (
  execution   TEXT NOT NULL,
  key         TEXT NOT NULL,
  site        TEXT NOT NULL,
  kind        TEXT NOT NULL,
  ordinal     INTEGER NOT NULL,
  request     TEXT NOT NULL,
  outcome     TEXT NOT NULL,
  payload     TEXT NOT NULL,
  recorded_at TEXT NOT NULL,
  PRIMARY KEY (execution, key)
);
CREATE INDEX IF NOT EXISTS effects_of_execution ON effects (execution);
CREATE INDEX IF NOT EXISTS executions_by_status ON executions (status, started_at);
`;

/** The journal as a SQLite file — the only backend `--target local` binds. */
class SqliteJournal implements Journal {
  readonly #database: Database;

  constructor(database: Database) {
    this.#database = database;
  }

  begin(row: ExecutionRow): void {
    this.#database.run(
      `INSERT INTO executions
         (id, flow, trigger_kind, inputs, session_key, callback, status, journal_version, started_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT (id) DO NOTHING`,
      [
        row.id,
        row.flow,
        row.trigger,
        JSON.stringify(row.inputs),
        row.sessionKey,
        row.callback ?? null,
        row.status,
        row.journalVersion,
        row.startedAt,
      ],
    );
  }

  end(id: string, status: Exclude<ExecutionStatus, "open">, error?: string): void {
    this.#database.run(
      "UPDATE executions SET status = ?, ended_at = ?, error = ? WHERE id = ?",
      [status, new Date().toISOString(), error ?? null, id],
    );
  }

  execution(id: string): ExecutionRow | undefined {
    const found = this.#database.get("SELECT * FROM executions WHERE id = ?", [id]) as Row | null;
    return found === null ? undefined : executionOf(found);
  }

  openExecutions(): readonly ExecutionRow[] {
    const rows = this.#database.all(
      "SELECT * FROM executions WHERE status = 'open' ORDER BY started_at ASC, id ASC",
    ) as Row[];
    return rows.map((row) => executionOf(row));
  }

  lookup(execution: string, key: string): JournalRecord | undefined {
    const found = this.#database.get("SELECT * FROM effects WHERE execution = ? AND key = ?", [
      execution,
      key,
    ]) as Row | null;
    if (found === null) return undefined;
    const row = found;
    const payload = String(row["payload"]);
    const outcome: JournalOutcome =
      row["outcome"] === "error"
        ? errorOutcome(payload)
        : { kind: "value", value: JSON.parse(payload) as unknown };
    return {
      execution: String(row["execution"]),
      key: String(row["key"]),
      site: String(row["site"]),
      kind: String(row["kind"]) as EffectKind,
      ordinal: Number(row["ordinal"]),
      request: String(row["request"]),
      outcome,
      recordedAt: String(row["recorded_at"]),
    };
  }

  append(record: JournalRecord): void {
    const payload =
      record.outcome.kind === "value"
        ? canonical(record.outcome.value)
        : canonical({ name: record.outcome.name, message: record.outcome.message });
    // One statement, so one implicit transaction: a crash leaves the row absent
    // rather than half written (see the module header).
    this.#database.run(
      `INSERT INTO effects
         (execution, key, site, kind, ordinal, request, outcome, payload, recorded_at)
       VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
       ON CONFLICT (execution, key) DO NOTHING`,
      [
        record.execution,
        record.key,
        record.site,
        record.kind,
        record.ordinal,
        record.request,
        record.outcome.kind,
        payload,
        record.recordedAt,
      ],
    );
  }

}

/** One error outcome, read back off its stored payload. */
function errorOutcome(payload: string): JournalOutcome {
  const held = JSON.parse(payload) as { name?: unknown; message?: unknown };
  return {
    kind: "error",
    name: typeof held.name === "string" ? held.name : "Error",
    message: typeof held.message === "string" ? held.message : payload,
  };
}

/** One `executions` row, as this module reads it. */
function executionOf(row: Row): ExecutionRow {
  const held = row;
  const endedAt = held["ended_at"];
  const error = held["error"];
  const callback = held["callback"];
  return {
    id: String(held["id"]),
    flow: String(held["flow"]),
    trigger: String(held["trigger_kind"]),
    inputs: JSON.parse(String(held["inputs"])) as Record<string, unknown>,
    sessionKey: String(held["session_key"]),
    ...(callback === null || callback === undefined ? {} : { callback: String(callback) }),
    status: String(held["status"]) as ExecutionStatus,
    journalVersion: Number(held["journal_version"]),
    startedAt: String(held["started_at"]),
    ...(endedAt === null || endedAt === undefined ? {} : { endedAt: String(endedAt) }),
    ...(error === null || error === undefined ? {} : { error: String(error) }),
  };
}

/**
 * The project's journal, opened and migrated on first use.
 *
 * The promise is cached rather than the handle, for the reason `./stores.ts`
 * caches its own: opening is asynchronous — the driver is imported lazily — and
 * a `serve` recovering several executions at once would otherwise open several
 * handles to one file.
 */
let opening: Promise<Journal> | undefined;

export function openJournal(): Promise<Journal> {
  opening ??= (async () => {
    const { Database } = await sqlite();
    fs.mkdirSync(dataRoot(), { recursive: true });
    return new SqliteJournal(await migrated(Database));
  })();
  void opening.catch(() => {
    opening = undefined;
  });
  return opening;
}

/**
 * How long the journal waits on a locked file, and how long the retry below
 * keeps trying for.
 *
 * There *is* something to wait on: this driver's virtual file system takes
 * SQLite's exclusive lock by creating `<file>.lock` as a directory and gives it
 * back by removing it, so a second process on one journal meets `SQLITE_BUSY`
 * rather than interleaving into the same pages. `busy_timeout` is set first for
 * that reason, and the retry beneath it covers the schema statements the pragma
 * cannot.
 *
 * What that lock does **not** do is die with the process holding it — see
 * [`LOCK_DIRECTORY`].
 */
const LOCK_WAIT_MS = 5_000;

/**
 * The lock a killed writer leaves behind, and why breaking it is right.
 *
 * A `mkdir` lock is released by an `rmdir` that a process which dies inside its
 * write never reaches. Nothing else ever removes it, so one interrupted write
 * would leave a directory that refuses **every** later open of that journal:
 * not just the resume of the execution the crash interrupted, but every future
 * run of the project. A durability story whose one artifact a crash can render
 * permanently unopenable is not one.
 *
 * So a lock that is still there after this process has waited [`LOCK_WAIT_MS`]
 * for it is treated as a corpse and removed, and the open is tried once more.
 * That is sound under exactly the rule this file already keeps (the module
 * header, `docs/durability.md` §2): **one process at a time writes a project's
 * journal**. A lock nobody gave back inside five seconds is, under that rule,
 * a lock whose owner is gone — and every write here is a single statement, so a
 * live owner never holds one for anything like that long.
 *
 * The hot rollback journal the same crash leaves is *not* touched: SQLite
 * recovers it on the next open, which is what makes the interrupted write leave
 * no half-written row (§2).
 */
const LOCK_DIRECTORY = ".lock";

/** Remove a lock no live process is giving back. See [`LOCK_DIRECTORY`]. */
function breakStaleLock(): boolean {
  const lock = `${journalPath()}${LOCK_DIRECTORY}`;
  if (!fs.existsSync(lock)) return false;
  try {
    fs.rmSync(lock, { recursive: true, force: true });
    return true;
  } catch {
    // A lock this process cannot remove is one it cannot get past either; the
    // open below fails with the driver's own message rather than with this.
    return false;
  }
}

/** The open handle, with the schema applied. */
async function migrated(
  Database: SqliteModule["Database"],
): Promise<InstanceType<SqliteModule["Database"]>> {
  const deadline = Date.now() + LOCK_WAIT_MS;
  let broke = false;
  let delay = 10;
  for (;;) {
    const database = new Database(journalPath());
    try {
      // **First**, so that every statement after it waits on a locked file
      // rather than failing at once. A `PRAGMA` that arrives after the contended
      // statement is a setting nobody read.
      database.exec(`PRAGMA busy_timeout = ${LOCK_WAIT_MS};`);
      // What makes "committed" mean "on the disk" rather than "in the page
      // cache", which is the whole of what a journal is for.
      database.exec("PRAGMA synchronous = FULL;");
      database.exec(SCHEMA);
      // `CREATE TABLE IF NOT EXISTS` leaves a table that exists exactly as it
      // is, so a column the schema grew after a file was created is a column
      // that file does not have. §11.2 makes a physical schema change
      // compatible only where it still reads older files, and a column added to
      // the lifecycle row has to be writable in one too — an `INSERT` naming a
      // column the file lacks would fail every new execution in it.
      const columns = database.all("PRAGMA table_info(executions)") as Row[];
      if (!columns.some((column) => column["name"] === "callback")) {
        database.exec("ALTER TABLE executions ADD COLUMN callback TEXT;");
      }
      return database;
      // A write-ahead log is deliberately **not** asked for. This driver's
      // virtual file system does not implement one — `PRAGMA journal_mode =
      // WAL` is accepted and leaves the mode at `delete` — so asking would be a
      // line that reads like a guarantee and is not one. The rollback journal
      // it uses instead is atomic per statement, which is the property this
      // file needs (see the module header).
    } catch (error) {
      database.close();
      if (Date.now() >= deadline) {
        // The deadline is up. Either a lock is still held — in which case its
        // owner is gone and it goes, once ([`LOCK_DIRECTORY`]) — or this is a
        // failure waiting cannot fix, and it is the caller's.
        if (broke || !breakStaleLock()) throw error;
        broke = true;
        continue;
      }
      // Whatever refused the open, tried again under a deadline rather than
      // failed on at once: the schema statements are the widest window a journal
      // has, and the honest thing to do about a file that is momentarily busy is
      // to let this process in rather than to fail its run over a table that
      // already exists.
      await new Promise((resolve) => setTimeout(resolve, delay));
      delay = Math.min(delay * 2, 200);
    }
  }
}

/**
 * Whether this project has a journal at all.
 *
 * Asked by `resume` before it opens one, so a project that has never run says
 * so — `openJournal` would otherwise create an empty file and then report the
 * execution id as unknown, which sends a reader to look at the id rather than
 * at the directory (PRD G3).
 */
export function journalExists(): boolean {
  return fs.existsSync(journalPath());
}

// ---------------------------------------------------------------------------
// Canonical values
// ---------------------------------------------------------------------------

/**
 * A value as the journal writes it: JSON with object keys in sorted order.
 *
 * Sorted because two of the three things this string is used for are
 * *comparisons* — a request identity checked against a recorded one, and the
 * divergence diagnostic that reports the difference — and a key order that
 * follows insertion would make two identical requests compare unequal because
 * one was built by a different branch of the same code.
 *
 * A member whose value is `undefined` is dropped exactly as `JSON.stringify`
 * drops it. A value that is `undefined` **whole** — a host function that
 * answered nothing (grammar 6.1 requires JSON of one, and `undefined` is not
 * JSON) — is written as `null` rather than as the JS `undefined` that
 * `JSON.stringify` answers: the payload column is `NOT NULL` and the driver
 * refuses to bind `undefined` at all, so leaving it would replace whatever the
 * node's own `output:` had to say about an empty answer with a SQLite error
 * raised *after* the effect had already happened.
 */
export function canonical(value: unknown): string {
  return JSON.stringify(sorted(value)) ?? "null";
}

/**
 * A value as a **later generation** will be handed it: this journal's own round
 * trip, applied to the generation that is recording it.
 *
 * `docs/durability.md` §11.1 promises that "a value written by one generation is
 * the value the next one is handed", and that promise has two ends. A replay
 * reads its value back through `JSON.parse` of [`canonical`], so its object keys
 * arrive **sorted**; the generation that recorded it held whatever order the
 * code that built the value happened to use. Handing the recording generation
 * the raw value would make the two differ — invisibly, until the value is
 * re-serialized into some later effect's request identity, where the difference
 * is reported as a divergence the composition never made. (A `kv` `get` builds
 * its row as `{value, found}` and reads back as `{found, value}`; an agent that
 * called a store tool puts that row verbatim into its next model call's turns.)
 *
 * So both generations are handed the round trip, and the asymmetry is gone.
 */
function revived(value: unknown): unknown {
  return JSON.parse(canonical(value)) as unknown;
}

function sorted(value: unknown): unknown {
  if (Array.isArray(value)) return value.map((element) => sorted(element));
  if (value === null || typeof value !== "object") return value;
  const held = value as Record<string, unknown>;
  const out: Record<string, unknown> = {};
  for (const key of Object.keys(held).sort()) {
    if (held[key] === undefined) continue;
    out[key] = sorted(held[key]);
  }
  return out;
}

// ---------------------------------------------------------------------------
// Divergence
// ---------------------------------------------------------------------------

/**
 * A replay that reached a recorded effect it cannot honour (resolved q29).
 *
 * "A divergence discovered at replay time — a recorded answer that fails the
 * current contract — fails the resume with a diagnostic naming the divergent
 * step, rather than silently re-executing an effect the journal claimed to
 * hold." This is that failure, and the message is the diagnostic: the node
 * path, the instance path, the ordinal, and what disagreed.
 *
 * It is deliberately **not** a `NodeFailure` and is not routed by a node's
 * `on_error:`. A composition's error policy decides what to do about the world
 * misbehaving; a journal that does not describe this graph is not the world
 * misbehaving, and absorbing it with `on_error: skip` would carry on from a
 * state the record does not support.
 */
export class ReplayDivergence extends Error {
  readonly key: string;
  readonly site: string;
  readonly kind: EffectKind;
  readonly ordinal: number;

  /**
   * `lead` names *which* of the two the failure is, and defaults to the common
   * one: the record and this build's composition disagree. The other reading —
   * the record is this run's and the world it was recorded against is gone —
   * passes its own, because a sentence saying the journal does not describe the
   * run would send a reader to a composition that never moved.
   */
  constructor(
    record: Pick<JournalRecord, "key" | "site" | "kind" | "ordinal">,
    detail: string,
    lead = "this execution's journal does not describe this run",
  ) {
    super(
      `${lead} at \`${record.site}\` ` +
        `(${record.kind} effect #${record.ordinal}, key \`${record.key}\`): ${detail}`,
    );
    this.name = "ReplayDivergence";
    this.key = record.key;
    this.site = record.site;
    this.kind = record.kind;
    this.ordinal = record.ordinal;
  }
}

/** How much of a disagreeing value a diagnostic quotes. */
const EXCERPT = 200;

/** One side of a divergence, short enough to read. */
function excerpt(text: string): string {
  return text.length <= EXCERPT ? text : `${text.slice(0, EXCERPT)}…`;
}

// ---------------------------------------------------------------------------
// Recording and replaying one execution's effects
// ---------------------------------------------------------------------------

/**
 * One execution's journal, plus the counters that address its effects.
 *
 * Held per execution rather than per node execution, and that is what makes a
 * key unique under a `retry:`. Grammar 9.4's idempotency key is *positional* —
 * a repeated attempt at one effect reuses its key, which is what a receiver
 * dedupes on — but a replay has to reproduce the **sequence**, including the
 * attempt that failed. So the ordinal a journal key carries counts every effect
 * of a kind ever issued at a site within this execution, and never restarts: a
 * node whose first attempt's model call was refused and whose second answered
 * records two effects, and a replay consumes them in that order.
 */
export interface JournalSession {
  readonly execution: string;
  readonly journal: Journal;
  /**
   * Whether this generation may consume records rather than only write them.
   *
   * `false` on the generation that *created* the execution, which is what keeps
   * a live run from reading its own writes back — an ordinal is allocated once
   * per effect, so it could not, but saying so here means a lookup is not even
   * attempted and a fresh run pays nothing for durability but the write.
   */
  readonly resuming: boolean;
  /** Next ordinal per `<site>#<kind>`. */
  readonly ordinals: Map<string, number>;
}

/** Every execution this process is journaling, by id. */
const sessions = new Map<string, JournalSession>();

/**
 * Open a session for one execution.
 *
 * Called by `runFlow` before the graph is streamed, so every node under it —
 * including a `flow:` node's instance, a `map`'s dispatch and a subflow a model
 * called, none of which is a `runFlow` of its own — finds the same session by
 * execution id and needs no plumbing to reach it.
 */
export function openSession(
  execution: string,
  journal: Journal,
  resuming: boolean,
): JournalSession {
  const session: JournalSession = { execution, journal, resuming, ordinals: new Map() };
  sessions.set(execution, session);
  return session;
}

/** Close one execution's session. The journal handle is the project's. */
export function closeSession(execution: string): void {
  sessions.delete(execution);
  latched.delete(execution);
}

/**
 * The recorder for one effect site, or `undefined` where nothing is journaling.
 *
 * `undefined` is a real answer rather than a defensive one: an ejected project
 * that calls `runFlow`'s internals directly, and this module's own unit tests,
 * run graphs with no session open, and every effect site is written to work
 * without one.
 */
export function recorderFor(execution: string, site: string): EffectRecorder | undefined {
  const session = sessions.get(execution);
  return session === undefined ? undefined : new EffectRecorder(session, site);
}

/**
 * What one recorded effect answered, and how a replay hands it back.
 *
 * `held` is the whole of the read: it is `undefined` at and past the frontier,
 * and a value or an error before it. A site that has one does **not** call the
 * world; a site that does not, does, and then keeps what it got.
 */
export interface EffectSlot {
  readonly key: string;
  /**
   * The three halves of [`key`], carried apart so a caller that has to *raise*
   * something about this effect names it the way a record does. Together with
   * `key` they are a `Pick<JournalRecord, …>`, which is what
   * [`ReplayDivergence`] takes.
   */
  readonly site: string;
  readonly kind: EffectKind;
  readonly ordinal: number;
  readonly held: JournalOutcome | undefined;
  /**
   * Record what the live effect answered, and answer with the value the
   * **journal** now holds — which is what the caller must go on with. See
   * [`revived`]: the two generations are handed the same value or the promise
   * `docs/durability.md` §11.1 makes is not one.
   */
  keep(value: unknown): unknown;
  /** Record what the live effect threw. */
  fail(error: unknown): void;
}

/**
 * The keys one effect site derives, and the record behind them.
 *
 * A key is
 *
 * ```text
 * <site> "#" <kind> "/" <ordinal>
 * ```
 *
 * where `<site>` is the effect site's instance path (grammar 9.4) flattened the
 * way `docs/trace.md` §8 flattens it — the node's own for a node's activity,
 * the dispatch's for a `map` item, the instance's for anything inside a `flow:`
 * node or a flow-as-tool call — and `<ordinal>` counts effects of that kind at
 * that site, from `0`. `#` separates them because it appears in neither half: a
 * node id is an identifier (grammar 2.1) and every other frame component is a
 * decimal.
 */
export class EffectRecorder {
  readonly #session: JournalSession;
  readonly #site: string;

  constructor(session: JournalSession, site: string) {
    this.#session = session;
    this.#site = site;
  }

  /** This recorder's site, flattened (grammar 9.4). */
  get site(): string {
    return this.#site;
  }

  /**
   * The recorder for a site nested under this one — a `map`'s dispatch.
   *
   * The counters stay the session's, so a node `retry:` that re-runs a whole
   * fan-out does not re-derive its first attempt's keys.
   */
  child(site: string): EffectRecorder {
    return new EffectRecorder(this.#session, site);
  }

  /**
   * Claim the next key of `kind` at this site, and read what the journal holds
   * for it.
   *
   * `request` is the effect's **identity**: enough of the call to say that a
   * recorded answer belongs to this request and not to a different one the same
   * site would have made under a different composition. It is compared verbatim
   * against the recorded identity, and a difference is a [`ReplayDivergence`]
   * rather than a re-execution (resolved q29).
   */
  claim(kind: EffectKind, request: unknown): EffectSlot {
    // A divergence raised where nothing could carry it out — a detached `map`
    // delivery, whose whole point is that the flow instance does not wait for it
    // (grammar 8.6 rule 7) — belongs to the **execution** rather than to that
    // branch: resolved q29 makes a divergence un-absorbable "by `retry:`,
    // `on_error:`, `on_item_error:`, or any policy at any nesting depth", and
    // `detach: true` is a policy. Re-raised at the next effect any branch of this
    // execution reaches, so a run with work left stops at once instead of
    // finishing against a record that has stopped describing it. See
    // [`latchDivergence`].
    const poisoned = latched.get(this.#session.execution);
    if (poisoned !== undefined) throw poisoned;
    const counter = `${this.#site}#${kind}`;
    const ordinal = this.#session.ordinals.get(counter) ?? 0;
    this.#session.ordinals.set(counter, ordinal + 1);
    const key = `${counter}/${ordinal}`;
    const site = this.#site;
    const session = this.#session;
    const identity = canonical(request);

    let held: JournalOutcome | undefined;
    if (session.resuming) {
      const found = session.journal.lookup(session.execution, key);
      if (found !== undefined) {
        if (found.request !== identity) {
          throw new ReplayDivergence(
            { key, site, kind, ordinal },
            `the journal recorded a request this run does not make. Recorded: ${excerpt(
              found.request,
            )}. This run: ${excerpt(identity)}`,
          );
        }
        held = found.outcome;
        // Request identity is only the **first** of resolved q29's two
        // divergences. The second — "a recorded answer fails the current
        // contract" — cannot be decided here, because the contract belongs to the
        // node that asked: it is decided where the answer is parsed, off this
        // note. See [`recordedAnswerOf`].
        if (held.kind === "value") {
          noteReplayed(held.value, {
            execution: session.execution,
            key,
            site,
            kind,
            ordinal,
            request: found.request,
          });
        }
      }
      // The first key the journal does not hold is the frontier, and past it
      // this execution is live again (resolved q29): `held` stays undefined and
      // the caller performs the effect. Nothing is recorded about *reaching* it,
      // because a frontier is per key rather than a state the execution enters —
      // a `map` whose third item had run and whose fourth had not resumes with
      // three replayed branches and one live one.
    }

    const write = (outcome: JournalOutcome): void => {
      session.journal.append({
        execution: session.execution,
        key,
        site,
        kind,
        ordinal,
        request: identity,
        outcome,
        recordedAt: new Date().toISOString(),
      });
    };

    return {
      key,
      site,
      kind,
      ordinal,
      held,
      keep: (value) => {
        // Recorded **and returned** as the journal now holds it, so this
        // generation and the next are handed the same value ([`revived`]).
        const kept = revived(value);
        write({ kind: "value", value: kept });
        return kept;
      },
      fail: (error) => write({ kind: "error", ...named(error) }),
    };
  }
}

// ---------------------------------------------------------------------------
// A recorded answer that fails the current contract (resolved q29's second half)
// ---------------------------------------------------------------------------

/**
 * Which record each replayed value came out of.
 *
 * resolved q29 makes **two** things a divergence: a recorded request the run no
 * longer makes, and "a recorded answer [that] fails the current contract". Only
 * the first is decidable at the effect seam — the second is decided by the
 * schema the *node* declares, which the seam has never seen and which sits one
 * call away in the emitted node body. So the seam leaves a note, and the parse
 * reads it ([`recordedAnswerOf`], `parseResult` in `./runtime.ts`).
 *
 * Weakly held, and by identity: the note lives exactly as long as the value the
 * graph is carrying, and a value that is not a replayed one is not in it. Every
 * object *inside* a replayed payload is noted too, because what a contract
 * refuses is as often a field of the answer as the answer itself.
 *
 * The alternative — letting the parse raise its ordinary `ResultMismatch` — is
 * the failure resolved q29 exists to prevent: that class is a node failure, a
 * `retry:` absorbs it, and the retry's second attempt claims an ordinal past the
 * frontier and re-issues the effect **live**.
 */
type ReplayedFrom = Pick<
  JournalRecord,
  "execution" | "key" | "site" | "kind" | "ordinal" | "request"
>;

const replayed = new WeakMap<object, ReplayedFrom>();

/** Note a replayed payload, and everything inside it. See [`replayed`]. */
function noteReplayed(value: unknown, record: ReplayedFrom): void {
  if (value === null || typeof value !== "object") return;
  replayed.set(value, record);
  for (const held of Object.values(value as Record<string, unknown>)) {
    noteReplayed(held, record);
  }
}

/**
 * The divergence a value that came out of the journal raises when the contract
 * it is being held to refuses it — and `undefined` for a value this generation
 * produced itself, which is an ordinary failure of the world.
 *
 * There is a third answer folded into the second, and it is what keeps this from
 * breaking compositions nobody touched: a recorded answer may fail a contract
 * that has not moved, because it failed it on the generation that recorded it
 * too — a flaky `exec:` under a `retry:` whose first attempt answered
 * off-contract and whose second did not. The journal says which happened. The
 * ordinal counts every effect of a kind ever issued at a site (§4), so if that
 * ladder went round again the **next** record at this site is its second
 * attempt — and what makes it that attempt rather than merely the next thing
 * the site did is that a retry repeats the *identical request*. So the next
 * record is compared by request identity, not merely counted: where it repeats
 * this one, the mismatch is one the composition already had and already decided,
 * the ladder does now what it did then, and its next attempt is a replay rather
 * than a live call. Where the journal holds nothing there — or holds a
 * *different* request, which is the site going on to its next effect rather than
 * retrying this one — the original never went round again, so this contract is
 * one this build brought, and re-running the effect under it is the
 * re-execution q29 refuses.
 *
 * Counting alone was the earlier reading and it is wrong at every site that
 * issues two or more effects of one kind: an agent whose loop calls `tool.alpha`
 * and then `tool.beta` records `…#tool/0` and `…#tool/1`, and a tightened
 * contract on *alpha* would find beta's record sitting where a second attempt
 * would have been and report an ordinary mismatch — which a `retry:` absorbs,
 * and whose second attempt walks past the frontier and issues both effects live.
 */
export function recordedAnswerOf(value: unknown, detail: string): ReplayDivergence | undefined {
  if (value === null || typeof value !== "object") return undefined;
  const record = replayed.get(value);
  if (record === undefined) return undefined;
  const session = sessions.get(record.execution);
  if (session !== undefined) {
    const key = `${record.site}#${record.kind}/${record.ordinal + 1}`;
    const next = session.journal.lookup(record.execution, key);
    if (next !== undefined && next.request === record.request) return undefined;
  }
  return new ReplayDivergence(
    record,
    `the recorded answer no longer satisfies this run's contract: ${detail}`,
  );
}

// ---------------------------------------------------------------------------
// A divergence with nobody to throw it to
// ---------------------------------------------------------------------------

/** The divergence each execution is holding, where one was raised off-thread. */
const latched = new Map<string, ReplayDivergence>();

/**
 * Hold a divergence raised on a branch **nothing awaits**.
 *
 * There is exactly one such branch: a detached `map` delivery, whose `.catch` is
 * the one place in this runtime where a failure legitimately stops travelling
 * (grammar 8.6 rule 7 — "nothing it does can delay the enclosing flow
 * instance"). A [`ReplayDivergence`] is the one failure that may not stop there:
 * resolved q29 makes it un-absorbable by any policy at any nesting depth, and
 * swallowing it would report a resume as complete while a delivery the record
 * claims to hold was never made.
 *
 * Held rather than thrown, because there is nobody to throw to. Two readers pick
 * it up: [`EffectRecorder.claim`], so any branch still running fails at its next
 * effect, and `runFlow` in `./graph.ts`, so an execution with no effects left
 * fails on the way out. The **first** is kept, because the first is the one that
 * describes where the record and the run parted company.
 */
export function latchDivergence(execution: string, divergence: ReplayDivergence): void {
  if (sessions.has(execution)) {
    if (!latched.has(execution)) latched.set(execution, divergence);
    return;
  }
  // The run this delivery belonged to has already ended — rule 7 is why nothing
  // waited for it — so there is no run left to fail. Said on stderr rather than
  // dropped: a divergence nobody can be told about is still one a reader has to
  // be able to find.
  process.stderr.write(`${divergence.name}: ${divergence.message}\n`);
}

/** The divergence latched on this execution, if one was. */
export function latchedDivergence(execution: string): ReplayDivergence | undefined {
  return latched.get(execution);
}

/** An error as the journal keeps it: the two halves `describe` reads. */
function named(error: unknown): { name: string; message: string } {
  if (error instanceof Error) return { name: error.name, message: error.message };
  return { name: "Error", message: String(error) };
}

/**
 * The error a replayed failure raises.
 *
 * Its `name` and `message` are the recorded ones, so every message this runtime
 * composes out of a failure — a `NodeFailure`'s text, a trace entry's `error`,
 * the human report — reads exactly as it did on the generation that failed.
 * What a replay cannot restore is the platform `cause` chain beneath it, which
 * `docs/trace.md` §11 already keeps out of the format and which only the human
 * report prints.
 */
export function replayedFailure(outcome: Extract<JournalOutcome, { kind: "error" }>): Error {
  const error = new Error(outcome.message);
  error.name = outcome.name;
  return error;
}

/**
 * Run one effect under the journal: consume a recorded answer, or perform it
 * and record what it answered.
 *
 * The shape every effect site in `./runtime.ts` and `./stores.ts` takes, so
 * "which effects are journaled" is a question a reader answers by finding the
 * calls to this function (`docs/durability.md` §3).
 */
export async function journaled<T>(
  recorder: EffectRecorder | undefined,
  kind: EffectKind,
  request: unknown,
  perform: () => Promise<T>,
  /**
   * Called with the slot the moment it is claimed, before anything is answered
   * and before the effect could be performed.
   *
   * For a caller that has to decide something from *whether the journal holds
   * this effect* — `./stores.ts` is the one, and what it decides is whether a
   * live op is about to read a store the recorded prefix filled in a process
   * that is gone. It runs in the same synchronous step the ordinal was
   * allocated in, so two branches of one execution cannot interleave between
   * the claim and the decision. It may throw, and a throw here reaches the
   * caller with the effect not performed and nothing written.
   */
  inspect?: (slot: EffectSlot) => void,
): Promise<T> {
  if (recorder === undefined) return await perform();
  const slot = recorder.claim(kind, request);
  inspect?.(slot);
  if (slot.held !== undefined) {
    if (slot.held.kind === "error") throw replayedFailure(slot.held);
    return slot.held.value as T;
  }
  let value: T;
  try {
    value = await perform();
  } catch (error) {
    // A divergence is not an outcome: it is this build refusing to go on
    // against a record it cannot honour, raised from *inside* an effect it did
    // not perform — a nested one, in a flow-as-tool child or a subflow whose
    // own seam is under this one. Recording it would put an error at a key the
    // effect never reached, and a later resume would replay that error as the
    // world's, absorbable by the very policies resolved q29 keeps it away from.
    if (error instanceof ReplayDivergence) throw error;
    slot.fail(error);
    throw error;
  }
  // The **kept** value rather than the live one: what the journal now holds is
  // what a resumed generation will be handed, so it is what this one goes on
  // with too (`docs/durability.md` §11.1, and see [`revived`]).
  return slot.keep(value) as T;
}
