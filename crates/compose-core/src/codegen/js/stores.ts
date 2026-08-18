//
// Attachable storage: the local backends every `store.*` runs on (PRD 5.8,
// grammar 11).
//
// `./graph.ts` binds one `StoreBinding` per `store.*` — its kind, its scope, the
// schemas it declares and the backend the active target resolved — and this
// module is what those bindings do. It is byte-identical in every project this
// compiler release builds, like `./runtime.ts` and `./cel.ts`.
//
// # The zero-infra guarantee
//
// PRD 5.8: "`--target local` substitutes SQLite/local disk for every store
// unconditionally". So a composition with a `store.*` in it runs with nothing
// installed and nothing configured: `kv` and `vector` are tables in a SQLite
// database this module creates on first use, and `blob` is a directory of files.
// Grammar 14.2's other providers — `redis`, `postgres`, `chroma`, `pgvector`,
// `qdrant`, `s3`, `gcs` — are M3's ("production `storage_backends` behind the
// store plugin interface"), and a binding that resolves to one says so at the
// op rather than answering out of the wrong store.
//
// # Where the data goes
//
// Under the **project's own** data directory, which is the emitted project's
// root — the directory `src/` sits in — rather than the process's working
// directory: a graph must read the same store whether it was launched from its
// own directory or from a repository root three levels up. `AGENT_COMPOSE_DATA_DIR`
// overrides it whole.
//
// ```text
// <project>/.agent-compose/
//   stores/<name>.sqlite            one database per `kv`/`vector` store
//   blobs/<name>/<partition>/values/<encoded key>
//   blobs/<name>/<partition>/types/<encoded key>     the `content_type:` of a `put`
// ```
//
// `<partition>` is the store's scope made concrete: `global`, `session/<encoded
// session key>`, or `execution/<execution id>`. In a SQLite-backed store the
// same partition is the `scope_key` column instead, so one file holds every
// session and a query never sees another's rows.
//
// A key becomes a file name by percent-encoding everything outside
// `[A-Za-z0-9_-]`, which is reversible (so `list` answers with the keys that were
// written, not with what the filesystem made of them) and leaves no name that
// could climb out of the directory: `.` and `/` are both encoded, so `..` is
// `%2E%2E`.
//
// # Scope
//
// * `execution` — dies with the run. Its rows live in an **in-memory** database
//   opened per execution and closed by [`releaseExecution`], which `runFlow`
//   calls when the run ends; its blobs live under a directory removed at the same
//   point. Nothing survives the process, which is what "dies with the run" has to
//   mean for a store nothing else can address.
// * `session` — partitioned by the session key the trigger supplied
//   (`execution.session_key`, grammar 11.3). Cross-session memory is exactly this:
//   a session-scoped store plus a trigger-supplied session key.
// * `global` — one partition for the whole project.
//
// # Replay discipline (PRD 5.8)
//
// Store ops are effects, and both halves of that are here:
//
// * **reads are recorded** — every op that answers with stored data writes a
//   `StoreRecord` onto the node's trace entry, carrying what it answered. That
//   record is the history a replay is meant to consume instead of the live store;
//   this release has no checkpointer to replay *from* (M3 owns durable
//   execution), so the record is what a trace holds and what a later replay will
//   read.
// * **writes are at-least-once, keyed** — a store-op node's write carries the
//   idempotency key of grammar 9.4, and this backend dedupes on it: the key and
//   the op's own answer are written in the **same transaction** as the effect, so
//   an attempt that failed half way leaves neither, and a retry of the same
//   effect site answers what the first attempt answered instead of writing twice.
//
// A store write an **agent** made through a synthesized tool (grammar 11.5)
// carries no key and is not deduped. Grammar 9.4 names exactly two carriers — "a
// detached `map` dispatch (§8.6 rule 7) and a store write (§11.4)" — and §11.4 is
// the store-op *node* catalog. The reading is also what the mechanism is for: a
// key substitutes for an outcome nobody observed, and a tool call's outcome goes
// straight back to the model that asked for it.
//
// # One process
//
// These backends are the local, zero-infra ones, and they assume the project is
// one process: SQLite here is a WebAssembly build over `node:fs` with no
// cross-process locking, so two `agent-compose run`s sharing a session-scoped
// store are outside what this release promises. That is the same boundary PRD
// 5.10 draws — `--target local` is one process — and production backends are M3.

import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import * as runtime from "./runtime.ts";
import type { EmbedBinding, RunContext, StoreRecord } from "./runtime.ts";

// ---------------------------------------------------------------------------
// What a binding says (grammar 11.1)
// ---------------------------------------------------------------------------

/** The three kinds grammar 11.1 admits. */
export type StoreKind = "kv" | "vector" | "blob";

/** The three lifetimes (Decision D35). */
export type StoreScope = "execution" | "session" | "global";

/** Grammar 14.2's storage vocabulary, over all three kinds. */
export type BackendProvider =
  | "memory"
  | "sqlite"
  | "redis"
  | "postgres"
  | "sqlite_vec"
  | "chroma"
  | "pgvector"
  | "qdrant"
  | "local_fs"
  | "s3"
  | "gcs";

/** The providers this compiler release actually implements. */
const LOCAL_PROVIDERS: readonly BackendProvider[] = ["memory", "sqlite", "sqlite_vec", "local_fs"];

/** Which backend a store resolved to, and where that resolution came from. */
export interface BackendBinding {
  /** The storage plugin (grammar 14.2). */
  readonly provider: BackendProvider;
  /**
   * How it was resolved, in the words grammar 11.3 uses: the `local` target's
   * unconditional substitution, a named alias, or a per-kind default. Carried so
   * a store bound to a backend this release cannot open says *why* it is bound
   * to it, which is the line an author has to change.
   */
  readonly from: string;
}

/** A resolved `store.*` (grammar 11.1), as `./graph.ts` binds it. */
export interface StoreBinding {
  /** Its typed address (grammar 2.2). */
  readonly address: string;
  /** Its local name — what a synthesized tool is named after (grammar 11.5). */
  readonly name: string;
  readonly kind: StoreKind;
  readonly scope: StoreScope;
  /** `description:` — LLM-facing on an attached store. */
  readonly description?: string;
  /**
   * Whether a `vector` store declares a `metadata_schema:` at all.
   *
   * Absent and `{}` are different declarations (Decision D114): a store with
   * none has no metadata, and a match carries no `metadata` field to read.
   */
  readonly metadata: boolean;
  /** `embed:` — `vector` only (grammar 11.2). */
  readonly embed?: EmbedBinding;
  /** The backend the active target resolved (grammar 11.3, 14.2). */
  readonly backend: BackendBinding;
}

// ---------------------------------------------------------------------------
// What an op takes (grammar 11.4)
// ---------------------------------------------------------------------------

/** The seven ops of grammar 11.4's catalog. */
export type StoreOp = "get" | "set" | "delete" | "list" | "search" | "upsert" | "put";

/**
 * One op's parameters, already evaluated.
 *
 * A store-op node evaluates the CEL ones in its **input** phase, outside its
 * error policy, for the reason every node builds its input there: an expression
 * that cannot be evaluated fails the execution rather than the activity
 * (grammar 4.1, Decisions D78, D110). A synthesized tool's arguments arrive here
 * having been parsed against the same schema the model was constrained by.
 */
export interface StoreParams {
  /** `key:` */
  readonly key?: string;
  /** `value:` — a string on a `vector upsert`/`blob put`, an object on a `kv set`. */
  readonly value?: unknown;
  /** `query:` */
  readonly query?: string;
  /** `prefix:` */
  readonly prefix?: string;
  /** `top_k:` */
  readonly topK?: number;
  /** `limit:` */
  readonly limit?: number;
  /** `filter:` */
  readonly filter?: Readonly<Record<string, unknown>>;
  /** `metadata:` */
  readonly metadata?: Readonly<Record<string, unknown>>;
  /** `content_type:` */
  readonly contentType?: string;
}

/** Where an op was issued from, and what key it carries (grammar 9.4). */
export interface StoreSite {
  /** Which consumption surface ran it (PRD 5.8). */
  readonly via: "node" | "tool";
  /**
   * The idempotency key of grammar 9.4, on the surface that carries one.
   *
   * Present on a store-op **node**'s write and absent everywhere else — see the
   * module header for why a tool-invoked write is not a carrier.
   */
  readonly idempotencyKey?: string;
}

/** The ops that write, which are the ones a key and a dedupe apply to. */
function writes(op: StoreOp): boolean {
  return op === "set" || op === "delete" || op === "upsert" || op === "put";
}

// ---------------------------------------------------------------------------
// Where the data lives
// ---------------------------------------------------------------------------

/** The variable that moves a project's whole data directory. */
export const DATA_DIRECTORY = "AGENT_COMPOSE_DATA_DIR";

/**
 * The emitted project's root: the directory `src/` sits in.
 *
 * Derived from this module's own URL rather than from `process.cwd()`, because a
 * store's data must not depend on where a process happened to be started: `bun
 * src/index.ts run …` from the project directory and the same command from a
 * repository root have to address one store.
 */
const PROJECT_ROOT = path.dirname(path.dirname(fileURLToPath(import.meta.url)));

/** Where this project keeps what its stores hold. */
export function dataRoot(): string {
  const override = process.env[DATA_DIRECTORY];
  return override === undefined || override === ""
    ? path.join(PROJECT_ROOT, ".agent-compose")
    : path.resolve(override);
}

/** One key, as a file name: reversible, and unable to climb out. */
export function encodeKey(key: string): string {
  const encoder = new TextEncoder();
  let encoded = "";
  for (const character of key) {
    if (/^[A-Za-z0-9_-]$/.test(character)) {
      encoded += character;
      continue;
    }
    for (const byte of encoder.encode(character)) {
      encoded += `%${byte.toString(16).toUpperCase().padStart(2, "0")}`;
    }
  }
  return encoded;
}

/** The inverse of [`encodeKey`], for the names a `list` reads back. */
function decodeKey(name: string): string {
  return decodeURIComponent(name);
}

/**
 * The longest encoded key a local blob store accepts.
 *
 * A file name is what a `blob` key becomes, and every filesystem this release
 * runs on caps one at 255 bytes. Refusing a longer key by name beats an
 * `ENAMETOOLONG` naming a path the author never wrote — and beats hashing it,
 * which would make `list` answer with names nobody stored.
 */
const MAX_ENCODED_KEY = 200;

/** The scope partition an op addresses (see the module header). */
function partition(store: StoreBinding, execution: runtime.ExecutionIdentity): string {
  if (store.scope === "global") return "global";
  if (store.scope === "execution") return `execution/${execution.id}`;
  const session = execution.session_key;
  if (session === "") {
    throw new Error(
      `\`${store.address}\` is \`scope: session\` and this execution has no session key: a session-scoped store keys off the identity its trigger supplies (grammar 11.3) — pass \`--session <key>\` to \`agent-compose run\`, or declare \`session_key:\` on the trigger`,
    );
  }
  return `session/${encodeKey(session)}`;
}

// ---------------------------------------------------------------------------
// The SQLite backend: `kv` and `vector`
// ---------------------------------------------------------------------------

type SqliteModule = typeof import("node-sqlite3-wasm");
type Database = InstanceType<SqliteModule["Database"]>;
type Row = Record<string, unknown>;

/**
 * The driver, loaded on first use.
 *
 * A dynamic import so that a composition with no store never pays for it: the
 * WebAssembly build costs tens of milliseconds to instantiate, and `./graph.ts`
 * imports this module unconditionally.
 *
 * The `default` dance is the one both supported runtimes agree on: the package
 * is CommonJS, so an ESM importer sees its exports under `default` — and the
 * ambient type declaration describes the CommonJS namespace directly, which is
 * why the cast is here rather than a `default` the types could name.
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

/**
 * Every open database, by the cache key [`handleKey`] derives.
 *
 * The map holds the **promise** rather than the handle, because opening one is
 * asynchronous — the driver is imported on first use — and a `map` dispatches
 * its instances concurrently (grammar 8.6). Two instances reaching one store in
 * the same turn would otherwise both find nothing cached, both open the file,
 * and leave one handle behind unclosed with writes going through the other.
 * Caching the promise makes the second caller wait for the first's answer, which
 * is the whole of the fix.
 */
const DATABASES = new Map<string, Promise<Database>>();

/** What an execution has opened, so [`releaseExecution`] can let it go. */
const PER_EXECUTION = new Map<string, Set<string>>();

/** Which database a store's op addresses. */
function handleKey(store: StoreBinding, execution: runtime.ExecutionIdentity): string {
  // ` ` cannot appear in a store name (grammar 2.1) or in an execution id,
  // so the two halves cannot be confused for one another.
  return store.scope === "execution" ? `${store.name} ${execution.id}` : store.name;
}

const SCHEMA = `
CREATE TABLE IF NOT EXISTS entries (
  scope_key TEXT NOT NULL,
  key       TEXT NOT NULL,
  value     TEXT NOT NULL,
  metadata  TEXT,
  embedding TEXT,
  PRIMARY KEY (scope_key, key)
);
CREATE TABLE IF NOT EXISTS applied (
  idempotency_key TEXT PRIMARY KEY,
  op              TEXT NOT NULL,
  result          TEXT NOT NULL
);
`;

/** The database a store's op runs against, opened and migrated on first use. */
function open(store: StoreBinding, execution: runtime.ExecutionIdentity): Promise<Database> {
  const cacheKey = handleKey(store, execution);
  const held = DATABASES.get(cacheKey);
  if (held !== undefined) return held;

  const opening = (async () => {
    const { Database } = await sqlite();
    let opened: Database;
    if (store.scope === "execution" || store.backend.provider === "memory") {
      // Nothing on disk: an execution-scoped store dies with the run, and
      // `provider: memory` says so outright.
      opened = new Database(":memory:");
    } else {
      const directory = path.join(dataRoot(), "stores");
      fs.mkdirSync(directory, { recursive: true });
      opened = new Database(path.join(directory, `${encodeKey(store.name)}.sqlite`));
    }
    opened.exec(SCHEMA);
    return opened;
  })();
  // Registered before the first `await` inside it, so a concurrent caller finds
  // this promise rather than opening a second handle. A failed open is dropped
  // from the cache so the next op tries again instead of inheriting the failure
  // forever.
  DATABASES.set(cacheKey, opening);
  void opening.catch(() => {
    if (DATABASES.get(cacheKey) === opening) DATABASES.delete(cacheKey);
  });
  if (store.scope === "execution") remember(execution.id, cacheKey);
  return opening;
}

/** Note a resource this execution owns, so its end can release it. */
function remember(execution: string, resource: string): void {
  let held = PER_EXECUTION.get(execution);
  if (held === undefined) {
    held = new Set();
    PER_EXECUTION.set(execution, held);
  }
  held.add(resource);
}

/**
 * Let go of everything an execution's `scope: execution` stores held.
 *
 * Called by `runFlow` when a run ends, however it ended. A long-lived process —
 * the generated `serve` app — runs many executions, so an execution-scoped store
 * that stayed open would be a leak *and* a lie: "dies with the run" is the
 * lifetime grammar 11.1 declares.
 */
export function releaseExecution(id: string): void {
  const held = PER_EXECUTION.get(id);
  if (held === undefined) return;
  PER_EXECUTION.delete(id);
  for (const resource of held) {
    const database = DATABASES.get(resource);
    if (database !== undefined) {
      DATABASES.delete(resource);
      // The handle is behind a promise (see [`DATABASES`]), so the close is
      // scheduled rather than performed: an op still in flight when the run
      // ended is the one case, and letting it finish beats closing the file
      // underneath it. Both rejections are swallowed — an open that failed has
      // nothing to close, and a close that failed released it anyway.
      void database.then(
        (held) => {
          try {
            held.close();
          } catch {
            // A database that is already closed is one that is already released.
          }
        },
        () => {},
      );
      continue;
    }
    // Anything else this execution owned is a directory of blobs.
    try {
      fs.rmSync(resource, { recursive: true, force: true });
    } catch {
      // Best effort: a temporary directory that outlives the process is a
      // nuisance, and failing a completed run over one would be worse.
    }
  }
}

/** Run `work` inside a transaction, rolling back whatever it did if it throws. */
function transact<T>(database: Database, work: () => T): T {
  database.exec("BEGIN IMMEDIATE");
  try {
    const answer = work();
    database.exec("COMMIT");
    return answer;
  } catch (error) {
    try {
      database.exec("ROLLBACK");
    } catch {
      // The rollback failing says nothing the original error does not.
    }
    throw error;
  }
}

// ---------------------------------------------------------------------------
// The op catalog (grammar 11.4)
// ---------------------------------------------------------------------------

/**
 * Run one op against its store, and record what it did (grammar 11.4, PRD 5.8).
 *
 * The answer is the op's **derived output row**, field for field — which is what
 * `writes:` binds by name and what a downstream guard reads (Decision D34). One
 * field of that row may be absent: `value` on a `kv`/`blob` `get` that missed,
 * which writes nothing and fails a read of it (Decision D110). That is why the
 * result is handed back as it is rather than parsed against the emitted Zod for
 * the row: the row's schema is a *description* of a shape this module builds,
 * not a contract with something outside the process, and a `get` that missed is
 * exactly the case a required-`value` parse would refuse.
 */
export async function runStoreOp(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  context: RunContext,
  site: StoreSite,
): Promise<Record<string, unknown>> {
  supported(store);
  const execution = context.execution;
  const scopeKey = partition(store, execution);
  const key = params.key;

  // At-least-once, deduped on the key (grammar 9.4). Asked before the effect and
  // recorded with it in one transaction, so an attempt that failed half way
  // leaves nothing for the next one to trip over.
  const idempotencyKey = writes(op) ? site.idempotencyKey : undefined;

  const answer = await perform(
    store,
    op,
    params,
    scopeKey,
    execution,
    idempotencyKey,
    context.signal,
  );
  record(context, {
    store: store.address,
    op,
    effect: writes(op) ? "write" : "read",
    via: site.via,
    scope: store.scope,
    ...(key === undefined ? {} : { key }),
    ...(writes(op) ? {} : { answer: answer.row }),
    ...(idempotencyKey === undefined ? {} : { idempotencyKey, deduped: answer.deduped }),
  });
  return answer.row;
}

/** One op's answer, and whether the backend had already applied its key. */
interface Applied {
  readonly row: Record<string, unknown>;
  readonly deduped: boolean;
}

/** Refuse a backend this compiler release does not implement (PRD §7 M3). */
function supported(store: StoreBinding): void {
  if (LOCAL_PROVIDERS.includes(store.backend.provider)) return;
  throw new Error(
    `\`${store.address}\` is bound to the \`${store.backend.provider}\` backend (${store.backend.from}), which this compiler release does not implement: production \`storage_backends\` — Redis, pgvector, S3 and the rest of grammar 14.2's vocabulary — land behind the store plugin interface in M3 (PRD §7). Build for \`--target local\`, which substitutes SQLite and local disk for every store unconditionally (PRD 5.8)`,
  );
}

async function perform(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  execution: runtime.ExecutionIdentity,
  idempotencyKey: string | undefined,
  signal: AbortSignal,
): Promise<Applied> {
  if (store.kind === "blob") {
    return blobOp(store, op, params, scopeKey, execution, idempotencyKey);
  }
  const database = await open(store, execution);
  // A `vector` write and a `vector` search both need a vector, and computing one
  // is a network call — so it happens before the transaction rather than inside
  // it, where it would hold a write lock across a round trip.
  const vector =
    store.kind === "vector" && (op === "upsert" || op === "search")
      ? await embed(
          store,
          op === "search"
            ? required(params.query, "query", store)
            : String(params.value ?? ""),
          signal,
        )
      : undefined;

  return transact(database, () => {
    if (idempotencyKey !== undefined) {
      const seen = database.get("SELECT result FROM applied WHERE idempotency_key = ?", [
        idempotencyKey,
      ]) as Row | null;
      if (seen !== null) {
        return { row: JSON.parse(String(seen["result"])) as Record<string, unknown>, deduped: true };
      }
    }
    const row = tabular(store, op, params, scopeKey, database, vector);
    if (idempotencyKey !== undefined) {
      database.run("INSERT INTO applied (idempotency_key, op, result) VALUES (?, ?, ?)", [
        idempotencyKey,
        op,
        JSON.stringify(row),
      ]);
    }
    return { row, deduped: false };
  });
}

/** The `kv` and `vector` rows of grammar 11.4's catalog. */
function tabular(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  database: Database,
  vector: number[] | undefined,
): Record<string, unknown> {
  switch (op) {
    case "get": {
      const key = required(params.key, "key", store);
      const found = database.get("SELECT value FROM entries WHERE scope_key = ? AND key = ?", [
        scopeKey,
        key,
      ]) as Row | null;
      // A miss answers `found: false` and **no** `value` at all (Decision D110).
      return found === null
        ? { found: false }
        : { value: JSON.parse(String(found["value"])) as unknown, found: true };
    }
    case "set": {
      const key = required(params.key, "key", store);
      database.run(
        "INSERT INTO entries (scope_key, key, value) VALUES (?, ?, ?) " +
          "ON CONFLICT (scope_key, key) DO UPDATE SET value = excluded.value",
        [scopeKey, key, JSON.stringify(params.value ?? {})],
      );
      return { key };
    }
    case "delete": {
      const key = required(params.key, "key", store);
      const result = database.run("DELETE FROM entries WHERE scope_key = ? AND key = ?", [
        scopeKey,
        key,
      ]);
      return { deleted: result.changes > 0 };
    }
    case "list": {
      const limit = required(params.limit, "limit", store);
      const prefix = params.prefix ?? "";
      const rows = database.all(
        "SELECT key FROM entries WHERE scope_key = ? AND substr(key, 1, ?) = ? ORDER BY key LIMIT ?",
        [scopeKey, prefix.length, prefix, limit],
      ) as Row[];
      return { keys: rows.map((row) => String(row["key"])) };
    }
    case "upsert": {
      const key = required(params.key, "key", store);
      database.run(
        "INSERT INTO entries (scope_key, key, value, metadata, embedding) VALUES (?, ?, ?, ?, ?) " +
          "ON CONFLICT (scope_key, key) DO UPDATE SET value = excluded.value, " +
          "metadata = excluded.metadata, embedding = excluded.embedding",
        [
          scopeKey,
          key,
          String(params.value ?? ""),
          JSON.stringify(params.metadata ?? {}),
          JSON.stringify(vector ?? []),
        ],
      );
      return { id: key };
    }
    case "search": {
      const topK = required(params.topK, "top_k", store);
      const filter = params.filter ?? {};
      const rows = database.all(
        "SELECT key, value, metadata, embedding FROM entries WHERE scope_key = ? ORDER BY key",
        [scopeKey],
      ) as Row[];
      const scored: { id: string; score: number; text: string; metadata: unknown }[] = [];
      for (const row of rows) {
        const metadata = JSON.parse(String(row["metadata"] ?? "{}")) as Record<string, unknown>;
        if (!matchesFilter(metadata, filter)) continue;
        scored.push({
          id: String(row["key"]),
          score: cosine(vector ?? [], JSON.parse(String(row["embedding"] ?? "[]")) as number[]),
          text: String(row["value"]),
          metadata,
        });
      }
      // Ties break by key, which is what makes two searches over one store
      // answer in one order: a fan-out reading a store must not depend on which
      // rows SQLite happened to hand back first.
      scored.sort((left, right) =>
        right.score === left.score
          ? left.id < right.id
            ? -1
            : left.id > right.id
              ? 1
              : 0
          : right.score - left.score,
      );
      return {
        matches: scored.slice(0, topK).map((match) =>
          // A store that declares no `metadata_schema` has no metadata at all,
          // so its matches carry no `metadata` field (Decision D114).
          store.metadata
            ? { id: match.id, score: match.score, text: match.text, metadata: match.metadata }
            : { id: match.id, score: match.score, text: match.text },
        ),
      };
    }
    default:
      throw new Error(`\`${store.address}\` is \`kind: ${store.kind}\` and takes no \`${op}\``);
  }
}

/** The `blob` rows of grammar 11.4's catalog, over a directory of files. */
function blobOp(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  execution: runtime.ExecutionIdentity,
  idempotencyKey: string | undefined,
): Applied {
  const root = path.join(dataRoot(), "blobs", encodeKey(store.name), scopeKey);
  const values = path.join(root, "values");
  const types = path.join(root, "types");
  const ledger = path.join(root, "applied");
  // An execution-scoped blob store's whole partition goes when the run does.
  if (store.scope === "execution") remember(execution.id, root);

  if (idempotencyKey !== undefined) {
    const marker = path.join(ledger, encodeKey(idempotencyKey));
    if (fs.existsSync(marker)) {
      return { row: JSON.parse(fs.readFileSync(marker, "utf8")) as Record<string, unknown>, deduped: true };
    }
    const row = blobEffect(store, op, params, values, types);
    fs.mkdirSync(ledger, { recursive: true });
    fs.writeFileSync(marker, JSON.stringify(row));
    return { row, deduped: false };
  }
  return { row: blobEffect(store, op, params, values, types), deduped: false };
}

function blobEffect(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  values: string,
  types: string,
): Record<string, unknown> {
  switch (op) {
    case "put": {
      const name = blobName(store, required(params.key, "key", store));
      fs.mkdirSync(values, { recursive: true });
      fs.writeFileSync(path.join(values, name), String(params.value ?? ""));
      if (params.contentType !== undefined) {
        fs.mkdirSync(types, { recursive: true });
        fs.writeFileSync(path.join(types, name), params.contentType);
      }
      return { key: params.key };
    }
    case "get": {
      const name = blobName(store, required(params.key, "key", store));
      const file = path.join(values, name);
      return fs.existsSync(file)
        ? { value: fs.readFileSync(file, "utf8"), found: true }
        : { found: false };
    }
    case "delete": {
      const name = blobName(store, required(params.key, "key", store));
      const file = path.join(values, name);
      const existed = fs.existsSync(file);
      if (existed) fs.rmSync(file);
      fs.rmSync(path.join(types, name), { force: true });
      return { deleted: existed };
    }
    case "list": {
      const limit = required(params.limit, "limit", store);
      const prefix = params.prefix ?? "";
      if (!fs.existsSync(values)) return { keys: [] };
      const keys = fs
        .readdirSync(values)
        .map(decodeKey)
        .filter((key) => key.startsWith(prefix))
        .sort();
      return { keys: keys.slice(0, limit) };
    }
    default:
      throw new Error(`\`${store.address}\` is \`kind: blob\` and takes no \`${op}\``);
  }
}

/** One blob key as a file name, refused where a filesystem could not hold it. */
function blobName(store: StoreBinding, key: string): string {
  if (key === "") {
    throw new Error(`\`${store.address}\` was asked for the empty key, which addresses no blob`);
  }
  const encoded = encodeKey(key);
  if (encoded.length > MAX_ENCODED_KEY) {
    throw new Error(
      `\`${store.address}\` was given a key of ${key.length} characters, which encodes to ${encoded.length} bytes: the local blob backend stores one key per file and a file name is capped at ${MAX_ENCODED_KEY} encoded bytes`,
    );
  }
  return encoded;
}

/** A parameter grammar 11.4's row requires, or a message naming what is missing. */
function required<T>(value: T | undefined, name: string, store: StoreBinding): T {
  if (value === undefined) {
    throw new Error(`the \`${name}\` of an op on \`${store.address}\` evaluated to nothing`);
  }
  return value;
}

/** Whether a row's metadata satisfies a `filter:` — equality, key by key. */
function matchesFilter(
  metadata: Readonly<Record<string, unknown>>,
  filter: Readonly<Record<string, unknown>>,
): boolean {
  for (const [name, wanted] of Object.entries(filter)) {
    if (JSON.stringify(metadata[name]) !== JSON.stringify(wanted)) return false;
  }
  return true;
}

/** Cosine similarity, `0` where either side has no magnitude. */
function cosine(left: readonly number[], right: readonly number[]): number {
  const width = Math.min(left.length, right.length);
  let dot = 0;
  let leftNorm = 0;
  let rightNorm = 0;
  for (let index = 0; index < width; index += 1) {
    dot += left[index]! * right[index]!;
    leftNorm += left[index]! * left[index]!;
    rightNorm += right[index]! * right[index]!;
  }
  if (leftNorm === 0 || rightNorm === 0) return 0;
  return dot / (Math.sqrt(leftNorm) * Math.sqrt(rightNorm));
}

/**
 * One text, as a vector, through the store's own `embed:` (grammar 11.2).
 *
 * `signal` is the **node's**, handed down from the op's own context: an
 * embedding is a network request made inside an activity, so a node whose
 * `timeout:` budget runs out should stop waiting on the socket rather than leave
 * it open behind an abandoned promise (grammar 9.2, and see
 * `runtime.runActivity`).
 */
async function embed(store: StoreBinding, text: string, signal: AbortSignal): Promise<number[]> {
  const binding = store.embed;
  if (binding === undefined) {
    throw new Error(`\`${store.address}\` is \`kind: vector\` and declares no \`embed:\``);
  }
  const [vector] = await runtime.callEmbeddings(binding, [text], signal);
  return vector ?? [];
}

/** Append one record to the node's trace entry, when the node is collecting. */
function record(context: RunContext, entry: StoreRecord): void {
  context.storeRecords?.push(entry);
}

// ---------------------------------------------------------------------------
// The synthesized tool surface (grammar 11.5)
// ---------------------------------------------------------------------------

/**
 * One store op reached as an LLM-facing tool (grammar 11.5, PRD 5.8).
 *
 * The arguments arrive parsed against the same schema the model was constrained
 * by, and they are exactly the op's own parameter row (grammar 11.4) with the
 * CEL positions replaced by values the model supplied. The mapping is here
 * rather than in `./graph.ts` because it is the same for every store of a kind.
 */
export async function runStoreTool(
  store: StoreBinding,
  op: StoreOp,
  args: Record<string, unknown>,
  context: RunContext,
): Promise<Record<string, unknown>> {
  const params: StoreParams = {
    ...(args["key"] === undefined ? {} : { key: String(args["key"]) }),
    ...(args["value"] === undefined ? {} : { value: args["value"] }),
    ...(args["query"] === undefined ? {} : { query: String(args["query"]) }),
    ...(args["prefix"] === undefined ? {} : { prefix: String(args["prefix"]) }),
    ...(args["top_k"] === undefined ? {} : { topK: Number(args["top_k"]) }),
    ...(args["limit"] === undefined ? {} : { limit: Number(args["limit"]) }),
    ...(args["filter"] === undefined
      ? {}
      : { filter: args["filter"] as Record<string, unknown> }),
    ...(args["metadata"] === undefined
      ? {}
      : { metadata: args["metadata"] as Record<string, unknown> }),
    ...(args["content_type"] === undefined ? {} : { contentType: String(args["content_type"]) }),
  };
  return await runStoreOp(store, op, params, context, { via: "tool" });
}
