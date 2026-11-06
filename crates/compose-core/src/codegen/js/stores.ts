//
// Attachable storage: the backends every `store.*` runs on (PRD 5.8,
// grammar 11).
//
// `./graph.ts` binds one `StoreBinding` per `store.*` — its kind, its scope, the
// schemas it declares and the backend the active target resolved — and this
// module is what those bindings do. Its invariant half is byte-identical in
// every project this compiler release builds, like `./runtime.ts` and
// `./cel.ts`; a target that binds a **dialled** `kv` backend gets that arm
// appended, and the driver it imports pinned, exactly as `./journal.ts` carries
// the arm its target's `journal:` bound (PRD resolved q62, q63).
//
// # The zero-infra guarantee
//
// PRD 5.8: "`--target local` substitutes SQLite/local disk for every store
// unconditionally". So a composition with a `store.*` in it runs with nothing
// installed and nothing configured: `kv` and `vector` are tables in a SQLite
// database this module creates on first use, and `blob` is a directory of files.
// That is what `--target local` binds, whatever any deploy file says, and it is
// unchanged by everything below.
//
// # The dialled backends
//
// A **named** target may bind a `kv` store to `postgres` or `mysql` (grammar
// 14.3, PRD resolved q63), and those are implemented here, behind this file's
// one interface: `runStoreOp` is the entry point on every backend, the ops are
// grammar 11.4's same rows, the scope partitions are keyed by the same
// derivation, the reads are recorded and the writes are deduped on the same
// key — so a store-op node, a synthesized tool and the trace record they file
// cannot tell which arm answered. What that buys is the thing a file cannot
// give: several processes reaching **one** store, which is why grammar 14.1
// rule 5 admits a dialled backend where it refuses a process-local one.
//
// A store is deliberately multi-writer, and nothing here takes a writer guard —
// see the dialled section for the whole of that, including the per-key
// last-write-wins a plain `set` still means.
//
// # Versions, conditions and partial updates (PRD resolved q68)
//
// Every `kv` key carries a **version**, drawn from one monotonic sequence per
// partition (a store and one scope key): every write that lands takes the next
// value of it. A sequence rather than a per-key counter, so a key deleted and
// set again never shows a version it showed before — the ABA race a
// compare-and-set exists to close stays closed across a delete. `get` answers
// it beside the value.
//
// A `kv` write may be **conditional** — `if: absent`, `if: present`, or
// `if_version: <n>` — and a condition that does not hold applies nothing and
// answers `applied: false`, with the version the key holds where it holds one:
// a normal answer an edge routes on, never a failure. Three ops write **part**
// of a value — `merge`, `append`, `incr` — and each reads what the key holds,
// applies itself, validates the whole result against the store's
// `value_schema`, and commits, all in one transaction. Every write is therefore
// a read-modify-write of its key, serialized against every other write to that
// key on every backend: SQLite's own serialized writer (`BEGIN IMMEDIATE`)
// locally, and a row lock on the partition's sequence row and on the key's
// entry on a dialled server. Two concurrent appends keep both items; a result
// the schema refuses fails the op and changes nothing. What each write decides,
// given what its key held, is [`decideKvWrite`] — one function every arm calls,
// so the arms differ in how they lock and store a row and in nothing a write
// means.
//
// Every single-key `kv` write answers `{ applied, version, deduped }` beside
// its own fields — a `batch`, which wrote many keys, answers `applied` and
// `deduped` with each entry's version in its `results` instead — and the
// idempotency ledger below stores that **whole** answer: a retried write is
// answered with its first attempt's `applied` and `version` and
// `deduped: true`, so a retried successful claim never fails against its own
// first write.
//
// # Reading at scale (PRD resolved q70)
//
// A `kv` `list` is one **page**: the first `limit` keys after an exclusive
// cursor (`after:`, the last key of the page before), in UTF-8 byte order on
// every arm, with a `next` exactly when more remain and — under
// `include_values: true` — each key's value and version beside it. `count`
// answers how many keys a prefix holds, and `get_many` reads up to 1000 keys
// in one statement and answers them in the order asked. All three are reads:
// recorded, answered out of the record on a replay, carrying no idempotency
// key. They are one implementation every `kv` arm runs ([`partitionRead`]),
// and which rows they see goes through one fragment ([`liveRows`]): the keys
// alive at the op's clock, so a key whose time is up is neither listed,
// counted nor read (PRD resolved q69).
//
// Grammar 14.3's other providers — `redis`, `chroma`, `pgvector`, `qdrant`,
// `s3`, `gcs` — are still M3's ("production `storage_backends` behind the store
// plugin interface"), and a binding that resolves to one says so at the op
// rather than answering out of the wrong store.
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
//   blobs/<name>/<partition>/applied/<key digest>    the idempotency ledger
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
// * `execution` — dies with the run on every backend, though not by the same
//   mechanism. On a local backend its rows live in an **in-memory** database
//   opened per execution and closed by [`releaseExecution`], which `runFlow`
//   calls when the run ends; its blobs live under a directory removed at the same
//   point; and a dialled `kv` store's rows are a partition on the server that the
//   same call **deletes**. So "dies with the run" is the lifetime on all three,
//   and only the in-memory one also dies with the *process* — which is the
//   difference the replay discipline below turns on.
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
//   `StoreRecord` onto the node's trace entry, carrying what it answered, and
//   the op itself is written to this project's journal (`./journal.ts`). The
//   second of those is what a replay consumes: a resumed execution answers a
//   read out of the record instead of asking the live store, and does not apply
//   a **write** a second time — the row the journal holds is the row the first
//   generation wrote, `deduped` included (PRD resolved q29,
//   `docs/durability.md` §3.3).
// * **writes are at-least-once, keyed** — a store-op node's write carries the
//   idempotency key of grammar 9.4, and this backend dedupes on it: a retry of
//   the same effect site answers what the first attempt answered instead of
//   writing twice. A `kv` write whose node wrote `idempotency_key:` carries an
//   **authored** key instead (PRD resolved q73) — `author/<store>/<key>`, see
//   "Authored keys" below — and dedupes on it the same way, across every
//   execution that writes the store under it.
//
//   The key is what a *receiver* dedupes on, and it stays load-bearing under
//   durability for the one window a journal cannot close: an effect that
//   happened and whose journal row did not land is re-executed on replay, and
//   carries the key its first attempt carried.
//
//   In the SQLite-backed kinds the key and the op's own answer are written in
//   the **same transaction** as the effect, so an attempt that failed half way
//   leaves neither. The `blob` backend is a directory rather than a database and
//   has no transaction to join: it applies the effect and then publishes the
//   ledger entry by writing it to a temporary name and renaming it into place,
//   which is atomic on every filesystem this runs on — so a half-written marker
//   is impossible, but a process that dies **between** the two leaves the effect
//   applied with no marker, and the next attempt applies it again. For `put`
//   that is a byte-identical overwrite; for `delete` the repeat answers
//   `deleted: false` where the first attempt answered `deleted: true`. That is
//   the difference between a transactional store and a filesystem, and it is
//   written down here rather than promised away.
//
// A store a **resumed** execution reads across the frontier therefore has to be
// one that outlives the **process**, and which stores those are is a property of
// the backend as much as of the scope. A store whose data is on disk or on a
// server is exactly the world the recorded prefix left behind: `scope: session`
// and `scope: global` under the local backends are files, a `scope: execution`
// `blob` store is a directory, and a `kv` store bound to `postgres` or `mysql`
// is rows on a server at **every** scope — `scope: execution` fixes when
// [`releaseExecution`] deletes that partition (at the generation that *ends* the
// execution, never at one that only parked) rather than where it lives. What
// does not outlive the process is what was opened inside it: a
// `scope: execution` `kv`/`vector` store on a local backend, and any
// `kv`/`vector` store a target bound to `provider: memory`. Their rows died with
// the process, and a replayed write is not applied a second time, so a live read
// past the frontier would answer out of an empty database. That is refused
// rather than answered, by [`inProcessState`] over [`inProcessOnly`], and
// `docs/durability.md` §5 is normative for it.
//
// A store write an **agent** made through a synthesized tool (grammar 11.5)
// carries no key — derived or authored — and is not deduped. Grammar 9.4 names exactly two carriers — "a
// detached `map` dispatch (§8.6 rule 7) and a store write (§11.4)" — and §11.4 is
// the store-op *node* catalog. The reading is also what the mechanism is for: a
// key substitutes for an outcome nobody observed, and a tool call's outcome goes
// straight back to the model that asked for it.
//
// # Authored keys (PRD resolved q73)
//
// A `kv` write's node may name its own ledger key, `idempotency_key:` — a CEL
// string over the node's input phase, so a retry re-derives the same one — and
// the site then carries `author/<store>/<key>` in place of grammar 9.4's
// derived key ([`authoredKey`]). Nothing else about the write changes: the
// ledger is asked before the effect and written with it, in one transaction,
// and a write whose key the ledger holds is answered with the row it holds —
// the first write's **whole** answer, its `version` included — and
// `deduped: true`, writing nothing. What changes is who can repeat it: a
// derived key is one effect site of one execution, while an authored key is
// one business event of one store, so a second **execution** writing under it
// is answered exactly as a retry of the first would be.
//
// **The namespace is the first segment.** Every derived key begins
// `<execution id>/`, and no execution id is `author` or begins `author/`
// ([`runtime.refuseReservedExecutionId`]: every id this runtime mints is
// `exec_` and a UUID, and the one boundary that takes a caller's id refuses
// that one), so the two kinds of key can never collide, a settled execution's
// prune — a range under its `<execution id>/` — never reaches an authored row,
// and a reader tells them apart by `author/` alone ([`authored`]).
//
// **Per store, not per partition.** Every ledger here is keyed by the
// idempotency key with no partition in it — a store file's `applied` table, a
// dialled store's `(store, idempotency_key)` — and an authored key's namespace
// names the store, so two sessions of one `scope: session` store writing under
// one authored key are one write: the second session is answered with the
// first's row, and its own partition is not written. An execution-scoped
// store's partition is one run's, which no second execution reaches, so the
// compiler refuses an authored key there and [`runStoreOp`] refuses one a
// caller hands it.
//
// **One key, one answer.** The row an authored key is answered with is the
// answer of the write that first took the key, in its op's shape. A second
// node naming the key is handed that row whenever it holds every field the
// second node's own output declares — the same op, any two of the partial
// updates, or a `set` after a partial update, without the `value` — and fails
// naming both ops where it does not, rather than being handed an answer its
// own output does not describe ([`answeredFrom`]).
//
// **Retention follows the store.** A derived row lives until its execution
// settles (see "Retention"). An authored row has no execution to settle, so
// it lives as long as the store's `default_ttl:` — its `expires_at` is the
// store's clock at the write plus that default — and, on a store that declares
// none, for good. An expired ledger row answers nothing, by the one inequality
// every expiry is read through ([`live`]), so the next write under its key
// applies as a first write does and takes the row's place; and it leaves
// storage inside the two sweeps an expired key does: the write sweep — made by
// a write carrying an authored key, at most [`SWEEP_PER_WRITE`] rows of its
// store's ledger — and the process-start sweep, a batch of at most
// [`SWEEP_PER_BATCH`] at a time behind the ops. That is the trade the stores
// topic states: without a `default_ttl:` the ledger grows by one row per
// business key, for ever, and that is what makes a business event land once
// however late its duplicate arrives.
//
// # One process
//
// **The local backends** assume the project is one process, and that is the
// paragraph a dialled binding is the way out of: SQLite here is a WebAssembly
// build over `node:fs`, whose virtual
// file system takes a lock by creating `<file>.lock` as a directory, so two
// `agent-compose run`s sharing a session-scoped store are outside what this
// release promises — the second one's op waits out `PRAGMA busy_timeout` and
// then fails the node with `SQLite3Error: database is locked` rather than
// corrupting anything. That is the same boundary PRD 5.10 draws — `--target
// local` is one process. The emitted `README.md` says so where a reader meets
// the data directory, because `agent-compose run` is the surface where running
// two at once is the obvious thing to try. A deployment that really has two
// processes over one store binds a dialled backend, which is what the section
// below is (PRD resolved q63).
//
// **A lock does not die with its owner**, which is the same fact `./journal.ts`
// is written around and matters here for the same reason: a `run` killed inside
// a store write never reaches the `rmdir`, and the directory it leaves would
// refuse every later open of that store — the resume of the very execution the
// crash interrupted included, whose live ops past the frontier are promised the
// world the recorded prefix left behind (`docs/durability.md` §5). So [`open`]
// waits [`LOCK_WAIT_MS`] and then treats a lock still standing as a corpse,
// exactly as the journal does and under the same rule that makes it sound.
//
// # Expiry (PRD resolved q69)
//
// A `kv` write may say how long the key it leaves lives — `ttl:` on `set`,
// `merge`, `append` and `incr`, `default_ttl:` on the store for a write that
// names none, `ttl: never` to clear a default — and the key then carries an
// absolute `expires_at`: the store's clock at the write plus the `ttl`, in
// epoch milliseconds. The **store's** clock: the server's on a dialled arm, so
// two processes writing one store agree about time whatever their own clocks
// say, and the process's on SQLite and `memory`, which have no other. Every
// write that lands sets it again — to its own `ttl`, else the default, else to
// nothing — so a write refreshes an expiry and a write with neither clears it,
// and a write that declined changes nothing, its key's expiry included.
//
// **An expired key is absent**, at the instant it expires and to everything:
// `get`, `list`, `count` and `get_many` do not see it, and neither does any
// condition — `if: absent` holds of it, `if: present` and `if_version:` do not
// — so a `set` with `if: absent` on an expired key applies and takes it, which
// with PRD resolved q68's versions is a lease. There is one notion of "alive"
// and it is [`live`]: a write reads its key through it under the write's lock,
// and the SQL reads carry the same inequality as [`liveClause`], which every
// read of many keys applies through [`liveRows`]. Expiry consumes no version —
// the sequence is never touched by it — so a key set again after expiring
// takes the partition's next one, which is past every version the key ever
// showed.
//
// **Removal is lazy, on a stated schedule.** An expired row stays in storage,
// invisible, until one of two things happens first: a write to its partition,
// which inside its own transaction deletes at most [`SWEEP_PER_WRITE`] of that
// partition's expired rows before it reads its key; or the next process start,
// which **starts** a sweep of every row of the store that had expired by then.
// "Process start" is two things here, and both start the one sweep: the start
// of a `run`, a `resume` or a `serve` ([`sweepBoundStores`], called by
// `./cli.ts` and `./serve.ts` before the first execution), which starts it on
// every `kv` store the composition binds whether or not anything will reach
// it; and the first op any process makes on a store it has not swept yet — the
// open of a SQLite file, the first op over a dialled connection — which is
// how a placement's worker, a library caller and a connection redialled after
// a loss sweep too. That sweep runs **behind** the ops, between them, one batch
// of at most [`SWEEP_PER_BATCH`] rows at a time, so no op waits for more than
// one batch however large the backlog is; and [`drainStores`], on a
// command's way out — a `run` or a `resume` exiting, a `serve` stopped — waits
// for it to finish, so a command has removed what had expired in every store it
// swept by the time it exits. Both sweeps run on an index over the expiry, so
// neither reads a row that is not expired. On a dialled store each batch is one
// partition's, taken under that partition's version-sequence lock exactly as a
// write takes it, so a sweep and a write are serialized rather than able to
// deadlock ([`RemoteKv`]'s `#sweepStep`).
//
// `AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS` shifts the clock every arm reads by a
// whole number of milliseconds — added to `Date.now()` locally and to the
// server's clock inside the statement on a dialled arm — which is how a test
// moves time forward past an expiry without waiting for it ([`clockOffset`]).
//
// # Retention
//
// What was written to a `session` or `global` store stays until it expires or
// the file is deleted, which is the point of those scopes. The one part of a
// store that is not the composition's own data is the **idempotency ledger** —
// the `applied` table, one row per keyed write — which serves an at-least-once
// guarantee and is kept exactly as long as that guarantee has someone to serve:
// a ledger row is what makes a retry of its effect site answer instead of
// writing twice, and an execution can retry only while its journal row is open.
// Every derived grammar 9.4 key begins `<execution id>/`, and PRD resolved q65 makes a
// settled execution one that never resumes, so when an execution's row closes
// `completed` or `failed` — and the detached deliveries it started, which
// write under its id, have finished — [`pruneLedger`] deletes the rows under
// that prefix in every store partition the execution could have written: a
// child execution's under the child's own id, when the child settles. An execution
// the journal still holds open keeps every row, so its resume writes nothing
// twice (PRD resolved q29). The ledger's derived rows therefore grow with the
// executions that are open rather than with history; its authored rows are the
// one exception, and their retention is their store's (see "Authored keys"
// above): no prune reaches `author/`. Which stores to prune is **not** a record
// this process keeps of the writes it made: a placed node's keyed write is
// made in a worker, and a resumed generation's replay of a placed node runs
// there too, so no one process sees every write an execution made. The prune
// visits every store the composition binds ([`bindsStores`]) instead — which
// is every store any process of it could have written — and deletes the
// settled execution's prefix from each; the rows are found by a range on the
// ledger's primary key, so a store the execution never wrote costs a seek
// that finds nothing.
//
// **The prune runs behind the outcome, never in front of it.** It is started
// once the row has closed and is never awaited by the settlement, so a bound
// store the execution never reached — one whose server is slow, refusing, or
// not answering at all — cannot delay or withhold the run's answer: `runFlow`
// returns when the row has closed. Each store is pruned on its own, so one
// that does not answer holds up no other, and a dial that has not completed in
// [`STORE_DIAL_MS`] fails. [`drainStores`], on a command's way out, waits for
// every prune in flight — and starts the ones still waiting on detached
// deliveries, which the exit is about to cut off — so a command's settled
// executions leave no rows behind. A `run` or a `resume` makes that wait as it
// exits ([`releaseStores`]); a `serve` lets its prunes finish in the background
// while it serves and makes the same wait when it is stopped, which is the one
// exit it has (`./serve.ts`).
//
// # Per-key history (PRD resolved q74)
//
// A `kv` store whose definition declares `history: true` records every write
// that lands as a **history row** beside the entry it leaves — `{ seq, op,
// old_value, new_value, version, actor, at }` — in a side table keyed by the
// partition and the key: a store file's `history`, a dialled server's
// `store_history`. The row is inserted **inside the write's own transaction**,
// after the entry's own statement and by the clock that write read, by every
// write path on every arm ([`historyRow`]): so a write that commits has its
// row, and one that does not — a schema-refused partial update, a statement the
// server failed, a crash before the `COMMIT` — has none, by construction.
//
// **What records a row is what lands.** A write whose condition did not hold
// (`applied: false`) changed nothing and records nothing; a write the ledger
// answered (`deduped: true`, a retry or an authored key's second execution)
// wrote nothing and records nothing; a replayed write is answered out of the
// journal and reaches no store at all (PRD resolved q29) — its row is the one
// the first attempt recorded. Every write that applied records one: a `set`,
// a `merge`, an `append`, an `incr` and a `delete` that removed the key; a
// `delete` that applied to a key holding nothing, with no value on either
// side; and an `append` with `unique: true` whose item was already there,
// which changes neither value nor version — its row holds the same value on
// both sides and the version the key kept, because it applied and refreshed
// its key's expiry, which is a write the ruling's "every write" names. A
// `batch` is one write of many keys (PRD resolved q71) and records one row per
// entry, each with the entry's own op, values and version, all inside the
// batch's transaction — and an entry whose condition declines the batch rolls
// back to the savepoint the rows were written under, so a declined batch
// records none.
//
// **The actor is a tagged object**, the one shape that types every writer in a
// derived row: `{ kind: "node", execution, path }` for a store-op node, whose
// `path` is the node's grammar 9.4 instance path — the site carries it on every
// node, derived key or authored, so an authored write (PRD resolved q73) names
// the same writer its derived one would; `{ kind: "tool", execution, path }`
// for a synthesized tool an agent called, whose `path` is the agent
// execution's instance path and the call's own frame (`<tool name>/<call
// ordinal>`, [`runtime.toolInstancePath`]); and `{ kind: "cli" }` for an
// operator's write at the project's command line (PRD resolved q72), which
// belongs to no execution.
//
// **`seq` is the row's place in its key's history**, and it is an order, not a
// count: an integer the table assigns as the row is inserted — SQLite's
// `AUTOINCREMENT`, a Postgres `BIGSERIAL`, a MySQL `AUTO_INCREMENT` — so it is
// never handed out twice, rows a retention sweep removed included, and two rows
// of one key may skip the numbers other keys' rows took between them. Every
// write to a key holds its partition's sequence lock (SQLite's serialized
// writer locally), so a key's rows are inserted, and take their `seq`, in the
// order their writes commit — which is what lets a page past a cursor never
// miss a row committed after it was read.
//
// **The `history` read pages one key exactly as a `list` pages a partition**
// ([`historyRead`]): the first `limit` rows after an exclusive cursor, asked for
// as `limit + 1` so the extra row says whether more remain; `next` is present
// exactly then, and is the page's last `seq` as a decimal string — the `after:`
// the page after it takes. The empty string is the cursor before the first
// row, as `list`'s `after: ""` is before every key. It is a read: recorded,
// answered out of the record on a replay, carrying no key.
//
// **Retention is `history_retention:`**, a duration on the definition, and a
// row older than it — its `at` at least that far behind the store's clock — is
// invisible to every `history` read at once and leaves storage on q69's
// schedule: inside a write to its partition, at most [`SWEEP_PER_WRITE`] rows
// before the write reads its key, and at the next process start, a batch of at
// most [`SWEEP_PER_BATCH`] at a time behind the ops. The cut-off is computed at
// the read and the sweep from the store's retention as it is declared then, so
// a changed `history_retention:` applies to every row, old ones included. A
// store that declares none keeps every row, for good — one per write — which
// is the trade the stores topic states. An operator's `export` and `import`
// carry no history: an export is the entries, and an import's writes are an
// operator's, recorded as `cli` like any other.
//
// History is a side table, not a query surface (PRD 5.8): it is read one key
// at a time, by `history` and `agent-compose store log`, and offered to no
// agent at any access level.

import crypto from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import process from "node:process";
import { fileURLToPath } from "node:url";

import { ReplayDivergence, dataRoot, journaled } from "./journal.ts";
import type { EffectSlot } from "./journal.ts";
import * as runtime from "./runtime.ts";
import type { EmbedBinding, RunContext, StoreRecord } from "./runtime.ts";

// Where a project's data lives is `./journal.ts`'s to say, because that module
// is the leaf of the emitted import graph — this one imports `./runtime.ts` and
// `./runtime.ts` imports it — and re-exported here because `./cli.ts` and every
// ejected reader learned the name from this module.
export { DATA_DIRECTORY, dataRoot } from "./journal.ts";

// ---------------------------------------------------------------------------
// What a binding says (grammar 11.1)
// ---------------------------------------------------------------------------

/** The three kinds grammar 11.1 admits. */
export type StoreKind = "kv" | "vector" | "blob";

/** The three lifetimes (Decision D35). */
export type StoreScope = "execution" | "session" | "global";

/** Grammar 14.3's storage vocabulary, over all three kinds. */
export type BackendProvider =
  | "memory"
  | "sqlite"
  | "redis"
  | "postgres"
  | "mysql"
  | "sqlite_vec"
  | "chroma"
  | "pgvector"
  | "qdrant"
  | "local_fs"
  | "s3"
  | "gcs";

/**
 * The providers whose bytes live **in the process that reaches them**.
 *
 * Grammar 14.1 rule 5's criterion, read at run time (Decision D131): a heap map,
 * a SQLite file with or without the vector extension, and a directory of blobs
 * are opened by whichever process reaches them. `validate` refuses a placement
 * over exactly this set; here it decides which arm answers an op.
 */
const IN_PROCESS_PROVIDERS: readonly BackendProvider[] = [
  "memory",
  "sqlite",
  "sqlite_vec",
  "local_fs",
];

/** Whether a provider's store is opened in this process. See above. */
function opensInProcess(provider: BackendProvider): boolean {
  return IN_PROCESS_PROVIDERS.includes(provider);
}

/** Which backend a store resolved to, and where that resolution came from. */
export interface BackendBinding {
  /** The storage plugin (grammar 14.3). */
  readonly provider: BackendProvider;
  /**
   * How it was resolved, in the words grammar 11.3 uses: the `local` target's
   * unconditional substitution, a named alias, or a per-kind default. Carried so
   * a store bound to a backend this release cannot open says *why* it is bound
   * to it, which is the line an author has to change.
   */
  readonly from: string;
  /**
   * The **name** of the variable holding this backend's address, on an entry
   * that declared a `url:` (grammar 4.3, 14.3).
   *
   * A name and never a value, like every credential this artifact carries: the
   * deploy file writes an `${ENV}` reference, `validate` never sees an address,
   * and the environment partition has already routed this variable to every
   * process that reaches this store (`docs/distributed.md` §9.1). It is what a
   * dialled arm opens with (PRD resolved q63), and absent on every binding a
   * target's built-in decided — which is every process-local one.
   */
  readonly urlEnv?: string;
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
  /**
   * `value_schema:`, as the emitted Zod — on a `kv` store.
   *
   * What a `merge`, `append` or `incr` holds the whole value it is about to
   * commit to (PRD resolved q68). A binding without one validates nothing,
   * which no binding `./graph.ts` emits is.
   */
  readonly value?: runtime.ResultSchema<unknown>;
  /**
   * `default_ttl:`, in milliseconds — on a `kv` store that declares one.
   *
   * The expiry a write that names no `ttl:` gives the key it leaves (PRD
   * resolved q69): a store-op node's that wrote none, and every synthesized
   * tool's, whose arguments carry no `ttl` at all. Absent means such a key
   * never expires.
   */
  readonly defaultTtl?: number;
  /**
   * `history: true` — on a `kv` store that declares it: every write that lands
   * records a history row in its own transaction (PRD resolved q74). Absent
   * means it keeps none.
   */
  readonly history?: boolean;
  /**
   * `history_retention:`, in milliseconds — beside `history: true`: how long a
   * history row is kept. Absent means every row is kept for good.
   */
  readonly historyRetention?: number;
  /** The backend the active target resolved (grammar 11.3, 14.2). */
  readonly backend: BackendBinding;
}

// ---------------------------------------------------------------------------
// What an op takes (grammar 11.4)
// ---------------------------------------------------------------------------

/**
 * The ops of grammar 11.4's catalog — its seven, PRD resolved q68's three
 * partial updates, q70's two reads, q71's `batch`, and q74's `history`.
 */
export type StoreOp = "get" | "set" | "delete" | "list" | "search" | "upsert" | "put" | "merge" | "append" | "incr" | "count" | "get_many" | "batch" | "history";

/** The literal conditions a `kv` write's `if:` takes (grammar 11.4). */
export type StoreCondition = "absent" | "present";

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
  /** `fields:` — what a `merge` changes. */
  readonly fields?: Readonly<Record<string, unknown>>;
  /** `field:` — the field an `append` or an `incr` addresses. */
  readonly field?: string;
  /** `unique:` — an `append` that adds only an item its array lacks. */
  readonly unique?: boolean;
  /** `by:` — what an `incr` adds; `1` when absent. */
  readonly by?: number;
  /** `if:` — the literal condition a `kv` write is applied under. */
  readonly condition?: StoreCondition;
  /** `if_version:` — the version the key must hold for the write to apply. */
  readonly ifVersion?: number;
  /**
   * `ttl:` — how long the key a `kv` write leaves lives, in milliseconds, or
   * `"never"`, which clears an expiry the store's `default_ttl:` would set
   * (PRD resolved q69). Absent means the default, where there is one.
   */
  readonly ttl?: number | "never";
  /**
   * `after:` — a `kv` `list`'s exclusive cursor: the last key of the page
   * before, as its `next` answered it (PRD resolved q70) — or a `history`'s:
   * the last `seq` of the page before, in decimal (PRD resolved q74).
   */
  readonly after?: string;
  /** `include_values:` — a `kv` `list` that answers its keys' values too. */
  readonly includeValues?: boolean;
  /**
   * `keys:` — what a `get_many` reads. The expression's JSON as it evaluated,
   * so a list of strings is what [`getManyKeys`] holds it to rather than what
   * the type says it already is.
   */
  readonly keys?: unknown;
  /**
   * `ops:` — a `batch`'s entries, in the order they apply (PRD resolved q71).
   * [`batchEntries`] holds them to the shape the compiler already decided, so
   * a caller that skipped it is refused by name rather than half applied.
   */
  readonly ops?: readonly BatchEntryParams[];
  /**
   * `idempotency_key:` — the business key a `kv` write's node authored, as its
   * input phase evaluated it (PRD resolved q73). Read by nothing in this
   * module's ops: the site's ledger key is built from it ([`authoredKey`]),
   * and it travels here so the journal's request — what a resume compares —
   * carries it, and the node's trace input shows it.
   */
  readonly idempotencyKey?: string;
}

/**
 * One entry of a `batch`, already evaluated: a `set` or a `delete` of one key,
 * with exactly the parameters the single op of its name takes (grammar 11.4,
 * PRD resolved q71).
 */
export interface BatchEntryParams {
  /** Which write the entry is. */
  readonly op: "set" | "delete";
  /** The key it addresses. */
  readonly key: string;
  /** A `set` entry's whole value. */
  readonly value?: unknown;
  /** `if:` — a `set` entry's literal condition. */
  readonly condition?: StoreCondition;
  /** `if_version:` — the version the key must hold. */
  readonly ifVersion?: number;
  /** `ttl:` — how long the key a `set` entry leaves lives, as a `set`'s is. */
  readonly ttl?: number | "never";
}

/** Where an op was issued from, and what key it carries (grammar 9.4). */
export interface StoreSite {
  /** Which consumption surface ran it (PRD 5.8). */
  readonly via: "node" | "tool";
  /**
   * The idempotency key of grammar 9.4, on the surface that carries one.
   *
   * Present on a store-op **node**'s write and absent everywhere else — see the
   * module header for why a tool-invoked write is not a carrier. Derived — the
   * execution id and the node's flattened instance path — or, on a node that
   * wrote `idempotency_key:`, authored: `author/<store>/<key>` (PRD resolved
   * q73, [`authoredKey`]).
   */
  readonly idempotencyKey?: string;
  /**
   * Where the op was issued from, as grammar 9.4's instance path joined with
   * `/` — a store-op node's own, or an agent execution's and its tool call's
   * frame — which is how the history row a `history: true` store records names
   * its writer (PRD resolved q74, [`HistoryActor`]). Carried on every node
   * whatever its ledger key is, since an authored key names no instance.
   */
  readonly path?: string;
}

/**
 * Who made the write one history row records (PRD resolved q74): a store-op
 * node or a synthesized tool, by its execution's id and the instance path its
 * site carries, or an operator at the project's command line, who belongs to
 * no execution.
 *
 * A tagged object rather than a bare `{ execution, path }` beside the string
 * `cli`, because it is a field of a derived output row and a row's field has
 * one type: `kind` says which writer it was, and `execution` and `path` are
 * there exactly when a node or a tool wrote — `path` only where its site
 * carried one, which every site this compiler emits does.
 */
export interface HistoryActor {
  readonly kind: "node" | "tool" | "cli";
  readonly execution?: string;
  readonly path?: string;
}

/** One history row, as a `history` read answers it (PRD resolved q74). */
export interface HistoryEntry {
  /** Its place in its key's history: monotonic per key, never reused. */
  readonly seq: number;
  /** The write it records: `set`, `delete`, `merge`, `append` or `incr`. */
  readonly op: string;
  /** What the key held before the write — absent where it held nothing. */
  readonly old_value?: unknown;
  /** What it held after — absent where it holds nothing, after a `delete`. */
  readonly new_value?: unknown;
  /** The version the write left — absent where the key holds nothing. */
  readonly version?: number;
  readonly actor: HistoryActor;
  /** When, in epoch milliseconds by the store's clock — the write's own. */
  readonly at: number;
}

/**
 * The first segment of every authored ledger key (PRD resolved q73) — which no
 * execution id is, so no derived key begins with it
 * ([`runtime.refuseReservedExecutionId`]).
 */
export const AUTHORED_NAMESPACE = runtime.AUTHORED_LEDGER_NAMESPACE;

/**
 * The ledger key a store-op node that wrote `idempotency_key:` carries:
 * `author/<store>/<key>`, `<store>` the store's declared name — the binding's
 * `name`, which a dialled ledger's `store` column holds too — and `<key>` the
 * string its input phase evaluated (PRD resolved q73).
 *
 * The store is in it although every ledger is already one store's, because the
 * namespace is the ruling's and a reader of a trace record — which names its
 * store separately — reads the key whole: two stores' authored keys are two
 * keys wherever they are read. The key is the remainder, so a `/` inside it is
 * the author's and moves nothing.
 */
export function authoredKey(store: string, key: string | undefined): string {
  if (typeof key !== "string") {
    throw new Error(
      `a store write to \`${store}\` was handed an \`idempotency_key\` of ${JSON.stringify(key ?? null)}: an authored ledger key is a string, which the compiler types its expression as (PRD resolved q73)`,
    );
  }
  return `${AUTHORED_NAMESPACE}/${store}/${key}`;
}

/** Whether a ledger key is an authored one rather than grammar 9.4's derived one. */
export function authored(idempotencyKey: string | undefined): boolean {
  return idempotencyKey !== undefined && idempotencyKey.startsWith(`${AUTHORED_NAMESPACE}/`);
}

/**
 * When the ledger row a keyed write leaves expires, given the write's clock —
 * or `null`, for one that does not (PRD resolved q73).
 *
 * A derived row never expires: it is kept exactly as long as its execution can
 * retry, and the settlement prune is what removes it ([`pruneLedger`]). An
 * authored row has no execution to settle, so it lives as long as the store's
 * `default_ttl:` says a key lives — the store's, never the write's own `ttl:`,
 * because the row is the business key's and not the value's — and for good on
 * a store that declares none.
 */
function ledgerExpiry(store: StoreBinding, idempotencyKey: string, now: number): number | null {
  if (!authored(idempotencyKey) || store.defaultTtl === undefined) return null;
  return Math.min(now + store.defaultTtl, Number.MAX_SAFE_INTEGER);
}

/**
 * The ops whose answers are one shape: grammar 11.4's catalog gives `merge`,
 * `append` and `incr` one output row (Decision D34), `{ key, applied,
 * version, value, deduped }`.
 */
const PARTIAL_UPDATES: readonly StoreOp[] = ["merge", "append", "incr"];

/**
 * The answer a write whose key the ledger holds is owed: the row it holds,
 * marked `deduped` ([`redelivered`]).
 *
 * An authored key may be named by more than one node, and the row it holds is
 * the answer of the write that took it first — an answer in that write's op's
 * shape. A later write under the key is answered with it, `deduped: true`,
 * whenever the row holds every field the later write's own output declares
 * (PRD resolved q73): the same op; any two of `merge`, `append` and `incr`,
 * whose answers are one shape; and a `set` answered by a partial update's
 * row, which holds every field a `set` answers — `key`, `applied`, `version`
 * — and one more, the `value`, which a `set`'s output does not declare and so
 * is not handed. Every other pair has no answer in the later write's shape: a
 * partial update answered by a `set`'s row has no `value` to answer with, a
 * `delete`'s answer has `deleted` and no `key` where every other single-key
 * write's has the reverse, and a `batch`'s is `results` where a single-key
 * write's is one key's. So that write fails naming both ops, rather than
 * handing the node an answer its own output does not describe. A derived key
 * is one effect site, whose op is the op that wrote its row, so it is answered
 * as it always was.
 */
function answeredFrom(
  store: StoreBinding,
  op: StoreOp,
  idempotencyKey: string,
  held: Readonly<Record<string, unknown>>,
): Applied {
  const wrote = held["op"];
  const row = redelivered(store, op, String(held["result"]));
  if (!authored(idempotencyKey) || wrote === op) return { row, deduped: true };
  const partial = (candidate: unknown): boolean => PARTIAL_UPDATES.some((one) => one === candidate);
  if (partial(wrote) && partial(op)) return { row, deduped: true };
  if (partial(wrote) && op === "set") {
    const answer = { ...row };
    delete answer["value"];
    return { row: answer, deduped: true };
  }
  throw new Error(
    `\`${store.address}\` holds the authored idempotency key ${JSON.stringify(idempotencyKey)} for a \`${String(wrote)}\`, and this write is a \`${op}\`: an authored key is answered by the row of the write that first took it, and a \`${String(wrote)}\`'s row holds no answer in a \`${op}\`'s shape — give the two nodes distinct keys, or make them ops one row answers alike: \`merge\`, \`append\` and \`incr\` answer one another, and a \`set\` takes a partial update's row without its \`value\` (PRD resolved q73)`,
  );
}

/**
 * Who issued an op, as far as what it does depends on it: one of the two
 * consumption surfaces of PRD 5.8 — whose op is recorded, journaled and, on a
 * node's write, keyed — or an **operator** at the project's command line (PRD
 * resolved q72), whose op is none of those ([`runOperatorOp`]).
 *
 * What it decides below is narrow, and the same for an operator as for a node:
 * a partial update its result's schema refuses is a plain `Error` for both, and
 * a [`runtime.ToolCallRefused`] only for a tool, whose caller is a model that
 * can make the call again (Decision D119, [`refused`]).
 */
type Issuer = StoreSite["via"] | "operator";

/**
 * The ops that write, which are the ones a key and a dedupe apply to.
 *
 * Every other op is a **read**: recorded, answered out of the record on a
 * replay, and carrying no idempotency key — a `list` page, PRD resolved q70's
 * `count` and `get_many` and q74's `history` exactly as a `get` (PRD resolved
 * q29). A
 * `batch` is a write, and **one**: its entries are one effect under the node's
 * one key, answered by one ledger row (PRD resolved q71).
 */
function writes(op: StoreOp): boolean {
  return (
    op === "set" ||
    op === "delete" ||
    op === "upsert" ||
    op === "put" ||
    op === "merge" ||
    op === "append" ||
    op === "incr" ||
    op === "batch"
  );
}

/** Whether one op is a `kv` write — the ops PRD resolved q68 versions. */
function kvWrite(store: StoreBinding, op: StoreOp): boolean {
  return store.kind === "kv" && writes(op);
}

// ---------------------------------------------------------------------------
// When a key expires (PRD resolved q69)
// ---------------------------------------------------------------------------

/**
 * The variable that shifts every store's expiry clock, in whole milliseconds.
 *
 * A test's hook rather than a deployment's setting — it is how the store
 * contract moves time past an expiry on every arm without sleeping through it,
 * and read at every op so one process can step its clock between two of them.
 * It shifts the clock an arm already reads rather than replacing it: added to
 * `Date.now()` on the local backends, and to the server's own clock **inside the
 * statement** on a dialled one, so two writers on one server still agree about
 * time with it set. A value that is not an integer is refused rather than read
 * as `0`, which is D50's posture on an environment variable: a test that believes
 * it moved the clock and did not would pass for the wrong reason.
 */
export const STORE_CLOCK_OFFSET = "AGENT_COMPOSE_STORE_CLOCK_OFFSET_MS";

/** What [`STORE_CLOCK_OFFSET`] adds to the clock; `0` where it is unset. */
function clockOffset(written: string | undefined = process.env[STORE_CLOCK_OFFSET]): number {
  if (written === undefined) return 0;
  if (!/^-?[0-9]+$/.test(written.trim())) {
    throw new Error(
      `\`${STORE_CLOCK_OFFSET}\` is set to ${JSON.stringify(written)}, which is not a whole number of milliseconds: it shifts the clock a \`kv\` store's expiry is read against (PRD resolved q69), so give it an integer such as \`60000\`, or unset it`,
    );
  }
  return Number(written.trim());
}

/** The local backends' clock: this process's, in epoch milliseconds. */
function localNow(): number {
  return Date.now() + clockOffset();
}

/**
 * The instant the key a `kv` write leaves expires, given the write's clock —
 * or `null`, for a key that does not.
 *
 * The write's own `ttl:` first, then the store's `default_ttl:`; `ttl: never`
 * is an answer of its own and beats a default, so it is not a missing value
 * the default could fill. Held below `Number.MAX_SAFE_INTEGER` so a `ttl:` of
 * centuries is still an integer every arm's column stores exactly.
 */
function expiryOf(store: StoreBinding, params: StoreParams, now: number): number | null {
  const ttl = params.ttl ?? store.defaultTtl;
  if (ttl === undefined || ttl === "never") return null;
  return Math.min(now + ttl, Number.MAX_SAFE_INTEGER);
}

/**
 * Whether a key whose expiry is `expiresAt` is still alive at `now`.
 *
 * **The** notion of alive, and its one inequality: a key with no expiry is
 * alive, and one with an expiry is alive strictly before it. [`liveClause`] is
 * the same inequality in SQL, for the reads that filter rows in a statement.
 */
function live(expiresAt: unknown, now: number): boolean {
  return expiresAt === null || expiresAt === undefined || Number(expiresAt) > now;
}

/**
 * [`live`] as a clause over a row's `expires_at`, against `now` — a bound `?`
 * on the local backend, and the server's own clock on a dialled one, so that a
 * read there is one statement answered by the clock its writers wrote with.
 */
function liveClause(now: string): string {
  return `(expires_at IS NULL OR expires_at > ${now})`;
}

/** At most how many expired rows one write removes from its partition. */
const SWEEP_PER_WRITE = 100;

/** How many expired rows one statement of a store-wide sweep removes. */
const SWEEP_PER_BATCH = 1_000;

// ---------------------------------------------------------------------------
// Where the data lives
// ---------------------------------------------------------------------------
//
// `DATA_DIRECTORY` and `dataRoot` are declared in `./journal.ts` and re-exported
// at the head of this file. They moved there when the journal arrived, because
// that module is the leaf of the emitted import graph and the two artifacts —
// a project's stores and a project's journal — share one directory. Nothing
// about the layout changed: `<project>/.agent-compose/`, moved whole by
// `AGENT_COMPOSE_DATA_DIR`, derived from this project's own location rather
// than from where a process happened to be started.

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

/**
 * What an execution owns, so [`releaseExecution`] can let it go.
 *
 * Two sets rather than one, because the two are released differently — a
 * database handle is closed, a directory of blobs is removed — and a single set
 * would have to guess which a string was. `databases` holds [`handleKey`]s,
 * `directories` holds absolute paths.
 */
const PER_EXECUTION = new Map<
  string,
  {
    readonly databases: Set<string>;
    readonly directories: Set<string>;
    /**
     * The partitions of a **dialled** store this run owns — rows on a server,
     * deleted when the run ends rather than released with a handle.
     *
     * Each entry is the store's binding and its partition, because both are
     * needed to write the `DELETE` and neither is derivable from the other.
     */
    readonly partitions: Map<string, { readonly store: StoreBinding; readonly scopeKey: string }>;
  }
>();

/** Which database a store's op addresses. */
function handleKey(store: StoreBinding, execution: runtime.ExecutionIdentity): string {
  // U+0000 cannot appear in a store name (grammar 2.1) or in an execution id,
  // so the two halves cannot be confused for one another. It is written as an
  // escape rather than as itself: a source file carrying a raw NUL is a file
  // every diff tool calls binary, and these are meant to be read.
  return store.scope === "execution" ? `${store.name}\u0000${execution.id}` : store.name;
}

/**
 * A store file's schema.
 *
 * `version` is a `kv` key's (PRD resolved q68) and `sequences` is where each
 * partition's version sequence is kept: one row per scope key, holding the
 * last version handed out — so a key deleted and set again takes a version
 * past every one the partition has shown, which `MAX(version)` over what is
 * stored would not. A `vector` store's file has both and uses neither.
 *
 * `expires_at` is a `kv` key's too (PRD resolved q69): the epoch millisecond it
 * expires at, or `NULL` for a key that does not — an integer, so the expiry is
 * a comparison SQLite makes in a statement rather than a parse.
 *
 * `applied` is the idempotency ledger, one row per keyed write, and its
 * `expires_at` is an authored row's (PRD resolved q73): the epoch millisecond
 * it stops answering, under a store's `default_ttl:`, and `NULL` on every
 * other row — a derived one is pruned at its execution's settlement instead.
 *
 * A file an earlier release created has `entries` without one column or both,
 * or an `applied` without its `expires_at`, and [`migrateStoreFile`] adds them.
 *
 * `history` is the per-key history a `history: true` store records (PRD
 * resolved q74), one row per write that landed, in every store file whether or
 * not its store keeps any — a store that does not leaves it empty. `seq` is
 * `INTEGER PRIMARY KEY AUTOINCREMENT`, so it is the row's rowid and no value
 * it ever held is handed out again, a swept row's included; the values are the
 * JSON this module wrote, `NULL` where the key held nothing on that side of the
 * write; and `at` is the write's own clock. Its three indexes are what reads
 * it: `history_key` a page of one key's rows past a cursor, `history_partition_at`
 * a write's bounded sweep of its own partition's rows past the store's
 * retention, and `history_at` the store-wide sweep at open. A file from before
 * PRD resolved q74 has no `history` table, and **`CREATE TABLE IF NOT EXISTS`
 * is the whole of its migration**: a table that is not there has no rows to
 * bring forward and no column to add, so creating it on open — every open
 * runs this schema — is what the earlier release's file needs, with nothing
 * for [`migrateStoreFile`] to do.
 */
const SCHEMA = `
CREATE TABLE IF NOT EXISTS entries (
  scope_key  TEXT NOT NULL,
  key        TEXT NOT NULL,
  value      TEXT NOT NULL,
  metadata   TEXT,
  embedding  TEXT,
  version    INTEGER NOT NULL DEFAULT 0,
  expires_at INTEGER,
  PRIMARY KEY (scope_key, key)
);
CREATE TABLE IF NOT EXISTS applied (
  idempotency_key TEXT PRIMARY KEY,
  op              TEXT NOT NULL,
  result          TEXT NOT NULL,
  expires_at      INTEGER
);
CREATE TABLE IF NOT EXISTS sequences (
  scope_key TEXT PRIMARY KEY,
  last      INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS history (
  seq             INTEGER PRIMARY KEY AUTOINCREMENT,
  scope_key       TEXT NOT NULL,
  key             TEXT NOT NULL,
  op              TEXT NOT NULL,
  old_value       TEXT,
  new_value       TEXT,
  version         INTEGER,
  actor_kind      TEXT NOT NULL,
  actor_execution TEXT,
  actor_path      TEXT,
  at              INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS history_key ON history (scope_key, key, seq);
CREATE INDEX IF NOT EXISTS history_partition_at ON history (scope_key, at);
CREATE INDEX IF NOT EXISTS history_at ON history (at);
`;

/**
 * The indexes an expiry is found through (PRD resolved q69, q73), partial so a
 * key — or a ledger row — that never expires costs none of them anything.
 *
 * `entries_partition_expiry` is what a write's bounded sweep of its own
 * partition reads, and `entries_expiry` what a store-wide one at open reads:
 * each finds the expired rows and only those, so neither sweep's cost depends
 * on how many live rows sit beside them. `applied_expiry` is the same for the
 * ledger's authored rows, which both sweeps reach store-wide — an authored key
 * is a store's, not a partition's. Created after [`migrateStoreFile`] has the
 * columns there, because an index over a column a file from an earlier
 * release lacks is a statement that file refuses.
 */
const EXPIRY_INDEXES = `
CREATE INDEX IF NOT EXISTS entries_partition_expiry ON entries (scope_key, expires_at)
  WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS entries_expiry ON entries (expires_at) WHERE expires_at IS NOT NULL;
CREATE INDEX IF NOT EXISTS applied_expiry ON applied (expires_at) WHERE expires_at IS NOT NULL;
`;

/**
 * Give a store file an earlier release created the columns a version and an
 * expiry need, and the indexes an expiry is found through.
 *
 * `CREATE TABLE IF NOT EXISTS` leaves a table that is already there as it is,
 * so a file from before PRD resolved q68 keeps an `entries` with no `version`,
 * and one from before q69 an `entries` with no `expires_at`. A `version` column
 * added here gives its rows `0`, which no write of this release hands out — the
 * sequence starts past what the partition holds — so a key a `get` answers
 * `version: 0` is one nothing has written since the upgrade, and an
 * `if_version: 0` names exactly it. An `expires_at` column added here gives its
 * rows `NULL`: a key written before expiry existed never expires until a write
 * gives it a `ttl`. A file from before PRD resolved q73 has an `applied` with no
 * `expires_at`, and the column added here gives every row it holds `NULL` —
 * each a derived row, which never expires. Two processes opening one old file
 * at once race to add a column, and the loser's "duplicate column" is the
 * column being there.
 */
function migrateStoreFile(database: Database): void {
  for (const [table, name, definition] of [
    ["entries", "version", "version INTEGER NOT NULL DEFAULT 0"],
    ["entries", "expires_at", "expires_at INTEGER"],
    ["applied", "expires_at", "expires_at INTEGER"],
  ] as const) {
    const columns = database.all(`PRAGMA table_info(${table})`) as Row[];
    if (columns.some((column) => column["name"] === name)) continue;
    try {
      database.exec(`ALTER TABLE ${table} ADD COLUMN ${definition}`);
    } catch (error) {
      if (!String((error as Error | undefined)?.message ?? error).includes("duplicate column")) {
        throw error;
      }
    }
  }
  database.exec(EXPIRY_INDEXES);
}

/**
 * Remove at most [`SWEEP_PER_WRITE`] expired rows from one partition — a
 * write's half of the cleanup schedule (PRD resolved q69), run inside its
 * transaction before it reads its key.
 *
 * Through `entries_partition_expiry`, so it reads the expired rows and nothing
 * else: a partition with a backlog of a million costs a write a hundred
 * deletes, and the next write a hundred more.
 */
function sweepLocalPartition(database: Database, scopeKey: string, now: number): void {
  database.run(
    "DELETE FROM entries WHERE rowid IN (SELECT rowid FROM entries WHERE scope_key = ? " +
      `AND expires_at IS NOT NULL AND expires_at <= ? LIMIT ${SWEEP_PER_WRITE})`,
    [scopeKey, now],
  );
}

/**
 * Remove at most [`SWEEP_PER_WRITE`] expired ledger rows from a store file —
 * the write half of the schedule, read on the ledger (PRD resolved q73): run
 * by a write carrying an authored key, inside its transaction, before it
 * claims its own row.
 *
 * Store-wide rather than one partition's, because an authored key is a
 * store's: its row is no partition's to sweep. Through `applied_expiry`, so it
 * reads the expired rows and nothing else — which are authored ones only,
 * since a derived row never expires.
 */
function sweepLocalLedger(database: Database, now: number): void {
  database.run(
    "DELETE FROM applied WHERE rowid IN (SELECT rowid FROM applied WHERE " +
      `expires_at IS NOT NULL AND expires_at <= ? LIMIT ${SWEEP_PER_WRITE})`,
    [now],
  );
}

/**
 * The instant at or before which a history row of `store` is past its
 * `history_retention:` by the clock `now` — or `undefined` on a store that
 * keeps every row (PRD resolved q74).
 *
 * Computed from the retention the store declares **now**, rather than written
 * on each row as it is recorded, so a retention changed in the composition
 * applies to every row the store already holds.
 */
function historyCutoff(store: StoreBinding, now: number): number | undefined {
  if (store.history !== true || store.historyRetention === undefined) return undefined;
  return now - store.historyRetention;
}

/**
 * Remove at most [`SWEEP_PER_WRITE`] history rows past the store's retention
 * from one partition — a write's half of the cleanup schedule, read on the
 * history (PRD resolved q74), run inside its transaction beside the entries'
 * sweep. Through `history_partition_at`, so it reads the rows past the cut-off
 * and nothing else. A store that keeps every row sweeps nothing.
 */
function sweepLocalHistory(database: Database, store: StoreBinding, scopeKey: string, now: number): void {
  const cutoff = historyCutoff(store, now);
  if (cutoff === undefined) return;
  database.run(
    "DELETE FROM history WHERE seq IN (SELECT seq FROM history WHERE scope_key = ? " +
      `AND at <= ? LIMIT ${SWEEP_PER_WRITE})`,
    [scopeKey, cutoff],
  );
}

/**
 * The store-wide sweeps in flight on every backend, so [`drainStores`] can
 * let each finish before the process goes.
 */
const SWEEPING = new Set<Promise<void>>();

/** Hold one store-wide sweep in [`SWEEPING`] until it ends. */
function holdSweep(sweeping: Promise<void>): void {
  SWEEPING.add(sweeping);
  void sweeping.finally(() => SWEEPING.delete(sweeping));
}

/**
 * Start removing every row of one store file that had expired by `now` — the
 * process-start half of the schedule (PRD resolved q69), started once per
 * process per file, by the file's open: the one [`sweepBoundStores`] makes as a
 * command starts, or the one the process's first op on the file makes.
 *
 * **Behind the ops, never in front of them.** In batches of
 * [`SWEEP_PER_BATCH`], each its own statement and so its own transaction, with
 * a turn of the event loop before each: the op that opened the file runs
 * first, and an op arriving while the sweep is under way waits for at most the
 * one batch in progress — the driver is synchronous, so a batch is one step no
 * op lands inside, and a write's transaction is one no batch lands inside —
 * however large the backlog is. A writer in another process gets in between two
 * batches the same way. [`drainStores`] waits for the rest, so a command that
 * reached the store has removed its backlog by the time it exits.
 *
 * **The ledger's expired rows go the same way** (PRD resolved q73): once the
 * entries are swept, the authored ledger rows whose store's `default_ttl:` has
 * run out by `now` leave in batches of the same size, through `applied_expiry`.
 * **And so do the history rows past the store's `history_retention:`** (PRD
 * resolved q74), last, through `history_at` — on a store that declares one.
 *
 * **Never an op's failure.** Every read and condition already treats an
 * expired row as absent, so a batch the file refused is said on stderr and the
 * sweep stops; the next write to each partition, and the next process, take
 * what it left.
 */
function startLocalSweep(database: Database, store: StoreBinding, now: number): void {
  holdSweep(
    (async () => {
      try {
        const cutoff = historyCutoff(store, now);
        const passes: readonly { readonly table: string; readonly expired: string; readonly by: number }[] = [
          { table: "entries", expired: "expires_at IS NOT NULL AND expires_at <= ?", by: now },
          { table: "applied", expired: "expires_at IS NOT NULL AND expires_at <= ?", by: now },
          ...(cutoff === undefined ? [] : [{ table: "history", expired: "at <= ?", by: cutoff }]),
        ];
        for (const { table, expired, by } of passes) {
          for (;;) {
            await new Promise((resolve) => setTimeout(resolve, 0));
            const removed = database.run(
              `DELETE FROM ${table} WHERE rowid IN (SELECT rowid FROM ${table} WHERE ` +
                `${expired} LIMIT ${SWEEP_PER_BATCH})`,
              [by],
            );
            if (removed.changes < SWEEP_PER_BATCH) break;
          }
        }
      } catch (error) {
        process.stderr.write(
          `warning: \`${store.address}\` could not finish removing its expired keys (${error instanceof Error ? error.message : String(error)}); they stay invisible to every read and condition, and a write to each one's partition removes them (PRD resolved q69)\n`,
        );
      }
    })(),
  );
}

/** Where a `kv` or `vector` store's file lives — one file per store. */
function storeFile(store: StoreBinding): string {
  return path.join(dataRoot(), "stores", `${encodeKey(store.name)}.sqlite`);
}

/**
 * How long an op waits on a locked store file, and how long the retry inside
 * [`migrated`] keeps trying for.
 *
 * `./journal.ts`'s own deadline to the millisecond, and deliberately so: the two
 * files sit in one directory, are locked by one driver's virtual file system,
 * and are left behind by one crash, so two different deadlines would be two
 * different answers to one question.
 */
const LOCK_WAIT_MS = 5_000;

/** How this driver's virtual file system spells a held lock. See the header. */
const LOCK_DIRECTORY = ".lock";

/** Remove a lock no live process is giving back. See the module header. */
function breakStaleLock(file: string): boolean {
  const lock = `${file}${LOCK_DIRECTORY}`;
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

/**
 * One store file, open with its schema applied — waiting out a lock and, past
 * the deadline, breaking the one a killed writer left behind.
 *
 * `./journal.ts`'s `migrated` for the other file in the same directory, and the
 * duplication is the honest shaping: the modules are separate compiler
 * constants with separate schemas, and the shared thing is a fact about the
 * driver rather than a function either of them owns.
 */
async function migrated(
  Database: SqliteModule["Database"],
  file: string,
): Promise<InstanceType<SqliteModule["Database"]>> {
  const deadline = Date.now() + LOCK_WAIT_MS;
  let broke = false;
  let delay = 10;
  for (;;) {
    const database = new Database(file);
    try {
      // **First**, so that every statement after it waits on a locked file
      // rather than failing at once. A `PRAGMA` that arrives after the
      // contended statement is a setting nobody read.
      database.exec(`PRAGMA busy_timeout = ${LOCK_WAIT_MS};`);
      database.exec(SCHEMA);
      migrateStoreFile(database);
      return database;
    } catch (error) {
      database.close();
      if (Date.now() >= deadline) {
        // The deadline is up. Either a lock is still there — in which case its
        // owner is gone under the one-process rule the header states, and it
        // goes, once — or this is a failure waiting cannot fix, and it is the
        // caller's.
        if (broke || !breakStaleLock(file)) throw error;
        broke = true;
        continue;
      }
      await new Promise((resolve) => setTimeout(resolve, delay));
      delay = Math.min(delay * 2, 200);
    }
  }
}

/** The database a store's op runs against, opened and migrated on first use. */
function open(store: StoreBinding, execution: runtime.ExecutionIdentity): Promise<Database> {
  const cacheKey = handleKey(store, execution);
  const held = DATABASES.get(cacheKey);
  if (held !== undefined) return held;

  const opening = (async () => {
    // Read before anything is opened, so a clock offset nobody can read fails
    // this op by name rather than inside [`migrated`]'s retry loop — and only
    // for a `kv` store, the one kind whose keys expire.
    const now = store.kind === "kv" ? localNow() : undefined;
    const { Database } = await sqlite();
    if (store.scope === "execution" || store.backend.provider === "memory") {
      // Nothing on disk: an execution-scoped store dies with the run, and
      // `provider: memory` says so outright. Nothing to contend over either —
      // an in-memory database is this process's alone — so it is opened
      // directly rather than through the deadline [`migrated`] keeps. Nothing
      // to sweep at its open either: it was created empty a moment ago.
      const opened = new Database(":memory:");
      opened.exec(SCHEMA);
      migrateStoreFile(opened);
      return opened;
    }
    const file = storeFile(store);
    fs.mkdirSync(path.dirname(file), { recursive: true });
    const database = await migrated(Database, file);
    // The open is this process's start on the file — [`sweepBoundStores`]'s,
    // or its first op's — which is the second half of PRD resolved q69's
    // schedule: every key whose time had come by now leaves storage, whatever
    // the writes since have swept — behind the ops and between them, never in
    // front of them. A `vector` file has no key that expires, so it is given no
    // clock and swept of nothing.
    if (now !== undefined) startLocalSweep(database, store, now);
    return database;
  })();
  // Registered before the first `await` inside it, so a concurrent caller finds
  // this promise rather than opening a second handle. A failed open is dropped
  // from the cache so the next op tries again instead of inheriting the failure
  // forever.
  DATABASES.set(cacheKey, opening);
  void opening.catch(() => {
    if (DATABASES.get(cacheKey) === opening) DATABASES.delete(cacheKey);
  });
  return opening;
}

/**
 * Note the resources one op's store makes this execution the owner of, so its
 * end can release them (grammar 11.1's `scope: execution` — "dies with the
 * run").
 *
 * Called from [`runStoreOp`] and **outside** the journaled seam, off
 * `store.scope` alone, because inside it this is a registration a replay never
 * performs: a resumed generation whose `put` is answered out of the journal
 * never enters [`blobOp`], so an ownership noted there would be noted by the
 * crashed generation and by nothing afterwards — and the run that *did* end
 * would leave the partition on disk for ever. It costs an op that owns nothing
 * two comparisons, and an op that does two set insertions.
 */
function remember(
  store: StoreBinding,
  execution: runtime.ExecutionIdentity,
  scopeKey: string,
): void {
  if (store.scope !== "execution") return;
  let held = PER_EXECUTION.get(execution.id);
  if (held === undefined) {
    held = { databases: new Set(), directories: new Set(), partitions: new Map() };
    PER_EXECUTION.set(execution.id, held);
  }
  if (store.kind === "blob") {
    held.directories.add(blobRoot(store, scopeKey));
    return;
  }
  if (!opensInProcess(store.backend.provider)) {
    held.partitions.set(`${store.address}\u0000${scopeKey}`, { store, scopeKey });
    return;
  }
  held.databases.add(handleKey(store, execution));
}

/**
 * Let go of everything an execution's `scope: execution` stores held.
 *
 * Called by `runFlow` when a run ends, however it ended. A long-lived process —
 * the generated `serve` app — runs many executions, so an execution-scoped store
 * that stayed open would be a leak *and* a lie: "dies with the run" is the
 * lifetime grammar 11.1 declares.
 *
 * `parked` says the run has **not** ended — its journal row is still open and a
 * resume will replay it (`runtime.staysOpen`). What lives in this process goes
 * either way, because this process is not what the resumed generation will read
 * from; what lives **on disk** stays, because it is. A `scope: execution` `blob`
 * store is a directory (see [`inProcessOnly`]), so removing it here would delete
 * the very world a live `get` past the frontier is promised to find — a replayed
 * `put` is never applied a second time, and nothing would compare unequal
 * (`docs/durability.md` §5). The generation that *ends* the execution removes
 * it, and reaches the same op site to re-register it on the way ([`remember`]).
 * An execution nobody ever resumes leaves its partition behind, exactly as it
 * leaves its row open.
 */
export function releaseExecution(id: string, parked = false): void {
  IN_PROCESS_ONLY.delete(id);
  const held = PER_EXECUTION.get(id);
  if (held === undefined) return;
  PER_EXECUTION.delete(id);
  for (const resource of held.databases) {
    const database = DATABASES.get(resource);
    // Absent where the ownership was noted at an op the journal answered, so
    // nothing was ever opened ([`remember`]): there is no handle to close.
    if (database === undefined) continue;
    DATABASES.delete(resource);
    // The handle is behind a promise (see [`DATABASES`]), so the close is
    // scheduled rather than performed: an op still in flight when the run
    // ended is the one case, and letting it finish beats closing the file
    // underneath it. Both rejections are swallowed — an open that failed has
    // nothing to close, and a close that failed released it anyway.
    void database.then(
      (open) => {
        try {
          open.close();
        } catch {
          // A database that is already closed is one that is already released.
        }
      },
      () => {},
    );
  }
  if (parked) return;
  for (const directory of held.directories) {
    try {
      fs.rmSync(directory, { recursive: true, force: true });
    } catch {
      // Best effort: a temporary directory that outlives the process is a
      // nuisance, and failing a completed run over one would be worse.
    }
  }
  for (const { store, scopeKey } of held.partitions.values()) {
    // A dialled store's `scope: execution` rows are on a server, so what makes
    // them "die with the run" is a `DELETE` rather than a closed handle. Not
    // awaited — this function is synchronous and is called from `runFlow` on the
    // way out — and not retried, for the `rmSync` above's reason. What a failed
    // one leaves is rows under an execution id nothing will address again, which
    // the dialled section's retention note is about.
    try {
      void connectionFor(store)
        .then((connection) => connection.forget(store.name, scopeKey))
        .catch(() => undefined);
    } catch {
      // `connectionFor` refuses a binding with no address, and a run that
      // reached this store already had one; a teardown is not where that is
      // worth raising.
    }
  }
}

// ---------------------------------------------------------------------------
// The ledger a settled execution leaves (PRD resolved q69)
// ---------------------------------------------------------------------------

/**
 * Every store the composition binds, by address — what [`pruneLedger`] visits
 * when an execution settles.
 *
 * Registered by `./graph.ts` as it loads ([`bindsStores`]), so it is the same
 * set in every process this project runs in — the hub, a placement's worker, a
 * resumed generation — and none of them has to have seen a write for a store to
 * be here. That is the point: the process that settles an execution is not
 * always the process that made its keyed writes (a placed node writes a dialled
 * store from its worker, grammar 14.1 rule 5, Decision D131, and a resume
 * replays a placed node there too), so a record of the writes this process made
 * would miss exactly those.
 */
const BOUND = new Map<string, StoreBinding>();

/**
 * Register the stores the composition binds — `./graph.ts`'s one call, made
 * once as it loads, with every `store.*` binding it declares. See [`BOUND`].
 */
export function bindsStores(bindings: readonly StoreBinding[]): void {
  for (const binding of bindings) BOUND.set(binding.address, binding);
}

/**
 * The prunes in flight, so [`drainStores`] lets each finish before the process
 * goes — and before [`releaseStores`] closes the connection one runs over — and
 * [`ledgerPrunesSettled`] can wait for them.
 */
const PRUNING = new Set<Promise<void>>();

/**
 * The settled executions whose prune is waiting on their detached deliveries,
 * by execution id — each the prune it will start ([`Deferred`]).
 *
 * Here rather than only in a promise chain so [`drainStores`] can find them:
 * a command on its way out — a `run` exiting, a `serve` stopped — cuts those
 * deliveries off, and a prune chained behind a delivery the exit kills would
 * never run.
 */
const DEFERRED = new Map<string, Deferred>();

/** One prune [`DEFERRED`] holds: what [`startPrune`] will be handed. */
interface Deferred {
  readonly execution: runtime.ExecutionIdentity;
  readonly held: (() => Promise<boolean>) | undefined;
}

/**
 * The settled executions whose prune [`drainStores`] started **ahead** of
 * their detached deliveries, which this process is letting go of.
 *
 * A keyed write under one of these ids is refused rather than made
 * ([`refuseUnledgered`]): its ledger has been pruned, so a retry of a write
 * its first attempt applied would find no row to answer it and apply a second
 * time. Only a command on its way out adds to it, so it holds the few
 * executions whose deliveries that exit is cutting off, and nothing else.
 */
const CUT_OFF = new Set<string>();

/**
 * Delete the idempotency ledger rows a settled execution left, in every store
 * the composition binds (PRD resolved q69) — started now, or, where `after` is
 * given, once it settles.
 *
 * Every derived grammar 9.4 key begins `<execution id>/`, so the rows are
 * exactly those under that prefix — an authored key's row is under `author/`,
 * which no execution id is, and no prune reaches it (PRD resolved q73): a
 * derived ledger row is a retry's answer, a settled execution
 * never resumes (PRD resolved q65), and nothing will ask for these again. A
 * child execution's rows are under the child's own id, and go when the child
 * settles. A row of an execution the journal still holds open is never
 * touched — the prefix is this execution's alone — so a resume still writes
 * nothing twice (PRD resolved q29).
 *
 * **Nor is one a worker may still be writing under.** A placed node's runner
 * whose dispatch the hub superseded goes on running on its worker, and writes
 * a dialled store under the same keys as the retry that replaced it, so its
 * late write is answered from the retry's row for as long as the row is there
 * (PRD resolved q63). The runtime asks the journal before it calls this and
 * hands no such execution over at all (`runtime.settlesLedgers`); `held`,
 * where it is given, is that question asked again as the prune starts — for
 * an execution a delivery still in flight may have handed a placed node of to
 * a worker meanwhile — and a `true` from it leaves every row where it is.
 *
 * **Every store, not the ones this process wrote** ([`BOUND`]). A `kv` or
 * `vector` file's ledger and a dialled store's are keyed by the idempotency
 * key, with no partition in the key, so each prune is one range on a primary
 * key wherever the write came from, and a store the execution never wrote costs
 * a seek that finds nothing. What is skipped is what cannot hold a row of it: a
 * `scope: execution` store, whose whole partition dies with the run
 * ([`releaseExecution`]); a store file nothing has created; a `memory` store
 * this process never opened; a dialled store whose address variable this
 * process does not hold — the environment partition hands it to every process
 * that reaches the store, the hub's included (`docs/distributed.md` §9.1), so a
 * process without it reached nothing there; and a provider this release does
 * not implement. A `blob` store's ledger is a directory per partition, so it is
 * pruned in the partition this execution's identity addresses, which is the
 * one partition of it the execution can have written.
 *
 * Called by the runtime ([`runtime.settlesLedgers`]) once the execution's row
 * has closed, with `after` the end of the detached deliveries it started where
 * any are still in flight: a crash between the close and the prune leaves rows
 * nothing will ask for, which is harmless, where the other order could leave an
 * open execution — or a delivery still retrying under a settled one's id —
 * without the row that makes its retry answer instead of writing twice.
 *
 * **Behind the outcome, and nothing of it awaited.** This returns at once; the
 * prune is held in [`PRUNING`] (or [`DEFERRED`], while it waits) for
 * [`drainStores`] and nothing else. Every bound store is visited, a store the
 * settling execution never reached included, and one of those can be slow or
 * not answer at all — so a settlement that waited for its prune would hand the
 * reachability of every store the composition binds to the outcome of every
 * run, which is the one thing a prune must not touch. The stores are pruned
 * independently, so one that does not answer delays no other, and a dial that
 * has not completed in [`STORE_DIAL_MS`] fails.
 *
 * **Never the execution's failure.** A store that could not be pruned is one
 * whose rows stay, which costs storage and changes no answer; it is said on
 * stderr naming the store, and every other store is still pruned.
 */
export function pruneLedger(
  execution: runtime.ExecutionIdentity,
  after: Promise<void> | undefined,
  held: (() => Promise<boolean>) | undefined,
): void {
  const deferred: Deferred = { execution, held };
  if (after === undefined) {
    startPrune(deferred);
    return;
  }
  DEFERRED.set(execution.id, deferred);
  const resume = (): void => {
    // Gone where [`drainStores`] has already started it, ahead of the
    // deliveries its exit cut off.
    if (DEFERRED.get(execution.id) !== deferred) return;
    DEFERRED.delete(execution.id);
    startPrune(deferred);
  };
  void after.then(resume, resume);
}

/** Start one settled execution's prune, held in [`PRUNING`] until it ends. */
function startPrune({ execution, held }: Deferred): void {
  // Never the authored namespace's range (PRD resolved q73): an execution
  // with that id is one nothing could have opened or written under
  // ([`runtime.refuseReservedExecutionId`]), so it has no rows to prune — and
  // its prefix would be every authored row's.
  if (runtime.reservedExecutionId(execution.id)) return;
  const prefix = `${execution.id}/`;
  const pruning = (async () => {
    // A worker may still be running a placed node of it, writing under these
    // keys (see [`pruneLedger`]): the rows stay, and are what answer it.
    if (held !== undefined && (await held().catch(() => true))) return;
    await Promise.all([...BOUND.values()].map((store) => pruneStore(store, execution, prefix)));
  })();
  PRUNING.add(pruning);
  void pruning.finally(() => PRUNING.delete(pruning));
}

/** One store's half of [`pruneLedger`]: never a rejection, a failure said. */
async function pruneStore(
  store: StoreBinding,
  execution: runtime.ExecutionIdentity,
  prefix: string,
): Promise<void> {
  if (store.scope === "execution") return;
  try {
    if (store.kind === "blob") {
      // A session-scoped store an execution with no session key cannot have
      // written: every op on it fails naming the key ([`partition`]).
      if (store.scope === "session" && execution.session_key === "") return;
      pruneBlobLedger(store, partition(store, execution), prefix);
      return;
    }
    if (opensInProcess(store.backend.provider)) {
      const held = DATABASES.get(handleKey(store, execution));
      if (held === undefined) {
        if (store.backend.provider === "memory") return;
        if (!fs.existsSync(storeFile(store))) return;
      }
      const database = await (held ?? open(store, execution));
      database.run("DELETE FROM applied WHERE idempotency_key >= ? AND idempotency_key < ?", [
        prefix,
        prefixEnd(prefix),
      ]);
      return;
    }
    if (!DIALLED_KV.includes(store.backend.provider)) return;
    const variable = store.backend.urlEnv;
    if (variable === undefined || (process.env[variable] ?? "") === "") return;
    await (await connectionFor(store)).prune(store.name, prefix);
  } catch (error) {
    process.stderr.write(
      `warning: the ledger rows execution \`${execution.id}\` left in \`${store.address}\` could not be pruned (${error instanceof Error ? error.message : String(error)}); they stay where they are and answer nothing, since a settled execution never resumes (PRD resolved q69)\n`,
    );
  }
}

/**
 * Wait for every ledger prune this process has started to finish — not the
 * ones still waiting on detached deliveries ([`DEFERRED`]), which start when
 * those finish.
 *
 * A settlement does not wait for its prune ([`pruneLedger`]), so this is how a
 * caller that needs to read what one left — a test, or [`drainStores`] —
 * waits for it. Re-read until nothing is left, because a prune can start while
 * another finishes.
 */
export async function ledgerPrunesSettled(): Promise<void> {
  for (;;) {
    const running = [...PRUNING];
    if (running.length === 0) return;
    await Promise.all(running.map((held) => held.catch(() => undefined)));
  }
}

/**
 * Refuse a keyed write under an execution whose ledger [`drainStores`]
 * pruned ahead of its detached deliveries ([`CUT_OFF`]).
 *
 * The ledger row a retry of the write would be answered from is gone, so made
 * here the write could apply a second time; refused, it is a delivery failing
 * on its way out of a process that is ending anyway.
 *
 * **Not an authored key** (PRD resolved q73). It names no execution — its
 * first segment is `author`, which no execution id is — and no prune removes
 * its row, so a write under one is answered by the ledger whatever has been
 * pruned: made on the way out, it applies once like any other.
 */
function refuseUnledgered(store: StoreBinding, idempotencyKey: string | undefined): void {
  if (idempotencyKey === undefined || CUT_OFF.size === 0) return;
  if (authored(idempotencyKey)) return;
  const end = idempotencyKey.indexOf("/");
  const execution = end < 0 ? idempotencyKey : idempotencyKey.slice(0, end);
  if (!CUT_OFF.has(execution)) return;
  throw new Error(
    `\`${store.address}\` refused a keyed write of execution \`${execution}\`: that execution has settled and this process, on its way out, has already deleted its idempotency ledger (PRD resolved q69), so a write made now could not be told from one its first attempt already applied`,
  );
}

/**
 * The least string past every string that begins with `prefix` — which ends
 * `/`, so it is the prefix with that byte moved on to `0`.
 *
 * A range rather than a `LIKE` or a `substr`, because a range is what each
 * arm's primary key on the ledger answers by seeking: the rows under the prefix
 * are one contiguous run of a byte-ordered index, and every ledger key column is
 * byte-ordered (SQLite's default `BINARY`, `COLLATE "C"`, `ascii_bin`).
 */
function prefixEnd(prefix: string): string {
  return `${prefix.slice(0, -1)}0`;
}

/**
 * The `blob` backend's half of [`pruneLedger`]: the markers of one partition
 * whose key begins with `prefix`.
 *
 * A marker is named by its key's digest ([`markerName`]), so the key is read out
 * of the marker itself; a staged `.partial` is a write still in flight, and a
 * marker that does not parse is left for the write that is still making it.
 */
function pruneBlobLedger(store: StoreBinding, scopeKey: string, prefix: string): void {
  const ledger = path.join(blobRoot(store, scopeKey), "applied");
  if (!fs.existsSync(ledger)) return;
  for (const name of fs.readdirSync(ledger)) {
    if (name.endsWith(".partial")) continue;
    const marker = path.join(ledger, name);
    let entry: Partial<LedgerEntry>;
    try {
      entry = JSON.parse(fs.readFileSync(marker, "utf8")) as Partial<LedgerEntry>;
    } catch {
      continue;
    }
    if (typeof entry.key === "string" && entry.key.startsWith(prefix)) {
      fs.rmSync(marker, { force: true });
    }
  }
}

runtime.settlesLedgers(pruneLedger);

/**
 * Start the store-wide sweep of every `kv` store the composition binds — the
 * "next process start" half of PRD resolved q69 (c)'s cleanup schedule — and
 * return at once.
 *
 * Called by `./cli.ts` as a `run` or a `resume` begins its execution, and by
 * `./serve.ts` once the app is listening: every process the project's command
 * line starts. **Every** bound store, not the ones the process will reach,
 * because that is what the schedule promises — a store whose partitions no
 * write reaches again, a cache nobody reads past its expiry, still sheds what
 * expired the next time the deployment starts. The first op a process makes on
 * a store it has not swept starts the same sweep ([`open`], [`RemoteKv`]), so a
 * store this one reaches before its sweep is under way is swept once, not
 * twice.
 *
 * **Behind everything, in front of nothing.** Each store's sweep is the one an
 * open starts — one batch at a time between the ops — and a dial it needs is
 * made here in the background, bounded by [`STORE_DIAL_MS`], so no execution
 * waits for a store it never reaches; [`drainStores`] waits for the rest on
 * a command's way out ([`SWEEPING`]).
 *
 * What is skipped is what holds nothing expired to this process: a
 * `scope: execution` store, whose partitions die with their runs; a `vector`
 * or `blob` store, whose keys never expire; a `memory` store, which a process
 * starts empty; a store file nothing has created; and a dialled store whose
 * address variable this process does not hold, which it could not reach to
 * sweep (`docs/distributed.md` §9.1). A sweep that cannot run is said on
 * stderr, naming the store; every read and condition already treats an expired
 * key as absent, so what it leaves changes no answer.
 */
export function sweepBoundStores(): void {
  for (const store of BOUND.values()) {
    if (store.kind !== "kv" || store.scope === "execution") continue;
    const provider = store.backend.provider;
    if (opensInProcess(provider)) {
      if (provider === "memory" || !fs.existsSync(storeFile(store))) continue;
      // The open starts the file's sweep ([`startLocalSweep`]); it is held as
      // well, so a command letting go before the open has finished still waits
      // for the sweep the open is about to start.
      holdSweep(
        open(store, { id: "", session_key: "" }).then(
          () => undefined,
          (error: unknown) => unswept(store, error),
        ),
      );
      continue;
    }
    if (!DIALLED_KV.includes(provider)) continue;
    const variable = store.backend.urlEnv;
    if (variable === undefined || (process.env[variable] ?? "") === "") continue;
    holdSweep(
      (async () => {
        try {
          await (await connectionFor(store)).sweep(store);
        } catch (error) {
          unswept(store, error);
        }
      })(),
    );
  }
}

/** Say that a process-start sweep of one store could not run. */
function unswept(store: StoreBinding, error: unknown): void {
  process.stderr.write(
    `warning: \`${store.address}\` could not be opened to remove the keys that have expired in it (${error instanceof Error ? error.message : String(error)}); they stay invisible to every read and condition, and a write to each one's partition removes them (PRD resolved q69)\n`,
  );
}

/**
 * Run `work` inside a transaction, rolling back whatever it did if it throws.
 *
 * `BEGIN IMMEDIATE` takes the write lock **before** the first read, so a `kv`
 * write's read of its key, its decision and its commit are one step no other
 * writer lands inside (PRD resolved q68). On this driver that is also what a
 * deferred `BEGIN` would do, and the difference is worth stating rather than
 * leaving to be rediscovered: the virtual file system has one lock, the
 * `<file>.lock` directory (see the module header), and it takes it for every
 * level SQLite asks for — a read's `SHARED` included — and keeps it to the end
 * of the transaction. So a read in one process already waits for another
 * process's open transaction, and no reader holds a lock a writer has to wait
 * on while it upgrades. `IMMEDIATE` is what keeps the claim true of a VFS with
 * real shared locks, where a deferred transaction's read would see what the
 * next commit is about to replace and then fail its upgrade with
 * `SQLITE_BUSY`. What the store contract checks is the property itself: a
 * write arriving while another write's window is held open waits for that
 * commit and builds on it (`store-window.mjs`).
 */
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
  // The disjointness the authored namespace rests on, at the store's own
  // boundary — every op of an execution whose id it would not hold is refused
  // here, whatever called it (PRD resolved q73).
  runtime.refuseReservedExecutionId(execution.id);
  const scopeKey = partition(store, execution);
  const key = params.key;
  // A `get_many`'s keys are a CEL list whose length nothing knew before the
  // node ran, so the most one reads is held here — before the journal, so a
  // refused read is a node failure rather than an effect anybody recorded, and
  // a replay refuses it the same way (PRD resolved q70). A `batch`'s entries
  // are held to their shape at the same point, for the same reason (PRD
  // resolved q71).
  if (op === "get_many") getManyKeys(store, params);
  if (op === "batch") batchEntries(store, params);
  // A `history` read of a store that keeps none, and a cursor that is not
  // one, are refused at the same point for the same reason (PRD resolved q74).
  if (op === "history") historyCursor(store, params);

  // At-least-once, deduped on the key (grammar 9.4). Asked before the effect and
  // recorded with it in one transaction, so an attempt that failed half way
  // leaves nothing for the next one to trip over.
  const idempotencyKey = writes(op) ? site.idempotencyKey : undefined;
  // An authored key is a business key two executions share, and an
  // execution-scoped store's partition is one execution's: the compiler
  // refuses the pair, and so does this, for a caller that is not the compiler
  // (PRD resolved q73).
  if (authored(idempotencyKey) && store.scope === "execution") {
    throw new Error(
      `\`${store.address}\` is \`scope: execution\` and was handed the authored idempotency key ${JSON.stringify(idempotencyKey)}: an execution-scoped store's partition is one run's and dies with it, so no second execution ever writes under the key — declare the store \`scope: session\` or \`scope: global\`, or write under the derived key (PRD resolved q73)`,
    );
  }

  // What `scope: execution` makes this run the owner of, noted **outside** the
  // seam below because a replayed op never enters it (see [`remember`]).
  remember(store, execution, scopeKey);

  // Both halves of PRD 5.8's replay discipline go through the journal, and this
  // is where "replay consumes history, not the live store" stops being a
  // description of the trace and becomes the mechanism: a replayed **read**
  // answers what it answered, and a replayed **write** is not applied a second
  // time — the row the journal holds is the row the first generation wrote,
  // `deduped` included, so the trace entry a resumed run files is the entry the
  // crashed one would have filed (`docs/durability.md` §3.3).
  const answer = await journaled(
    context.effects,
    "store",
    { store: store.address, op, scope: store.scope, partition: scopeKey, via: site.via, params },
    async () => {
      // Live only: a replayed write is answered by the record and reaches no
      // store, so it has no ledger row to miss.
      refuseUnledgered(store, idempotencyKey);
      return await perform(
        store,
        op,
        params,
        scopeKey,
        execution,
        idempotencyKey,
        site.via,
        context.signal,
        // Who a history row this write records names (PRD resolved q74): the
        // surface that ran it, its execution, and the instance path its site
        // carries.
        {
          kind: site.via,
          execution: execution.id,
          ...(site.path === undefined ? {} : { path: site.path }),
        },
      );
    },
    (slot) => inProcessState(store, op, execution, slot),
  );
  // A `kv` write's record carries what it decided — whether it landed, and
  // the version the key holds after it — read off its own answer, so the
  // record and the node's output cannot disagree (`docs/trace.md` §6, PRD
  // resolved q68). A `batch` addresses many keys and leaves many versions, so
  // its record carries neither a `key` nor a `version`: the keys it wrote and
  // the version each took are its answer's `results`, which the record
  // carries whole, as a read's does (PRD resolved q71).
  const version = answer.row["version"];
  record(context, {
    store: store.address,
    op,
    effect: writes(op) ? "write" : "read",
    via: site.via,
    scope: store.scope,
    ...(key === undefined ? {} : { key }),
    ...(writes(op) && op !== "batch" ? {} : { answer: answer.row }),
    ...(kvWrite(store, op)
      ? {
          applied: answer.row["applied"] === true,
          ...(typeof version === "number" ? { version } : {}),
        }
      : {}),
    ...(idempotencyKey === undefined ? {} : { idempotencyKey, deduped: answer.deduped }),
  });
  return answer.row;
}

// ---------------------------------------------------------------------------
// A store the frontier cannot reach back into
// ---------------------------------------------------------------------------

/**
 * Whether this store's data lives **only in the process that opened it**.
 *
 * Exactly [`open`]'s `:memory:` arm: a `scope: execution` `kv`/`vector` store,
 * and any store a target bound to `provider: memory`. A `blob` store is a
 * directory either way, so its data outlives the process that wrote it even at
 * `scope: execution` — which is what [`releaseExecution`] removes when the
 * execution **ends**, and what a resumed generation removes on its way out
 * ([`remember`]). A generation that only *parked* removes nothing, which is what
 * leaves that directory there to be read across the resume
 * (`docs/durability.md` §5).
 *
 * A **dialled** store is the same case as a blob and for the same reason: its
 * rows are on a server, so `scope: execution` fixes when they are deleted rather
 * than where they live, and a resumed generation reads the world its recorded
 * prefix left behind (PRD resolved q63). That is what makes the dialled arms a
 * store a resume may read past its frontier at every scope.
 */
function inProcessOnly(store: StoreBinding): boolean {
  if (store.kind === "blob") return false;
  if (!opensInProcess(store.backend.provider)) return false;
  return store.scope === "execution" || store.backend.provider === "memory";
}

/**
 * Per execution, the in-process stores whose recorded prefix **wrote** to them.
 *
 * Emptied by [`releaseExecution`] with everything else the execution owned.
 */
const IN_PROCESS_ONLY = new Map<string, Set<string>>();

/**
 * Refuse a live op on a store whose contents died with the generation that
 * filled it (`docs/durability.md` §5).
 *
 * The frontier model says a live effect past it acts on the same world the
 * recorded prefix left behind, and for a network, a filesystem or a store on
 * disk it does. For a store that lives in the process it does not: the prefix's
 * writes are answered out of the journal and so are **never re-applied**, and
 * the resumed process holds a database that was created empty a moment ago. The
 * first live read then answers `found: false` about something the execution
 * wrote, routes down a branch the original would never have taken, and reports
 * `completed`. Nothing compares unequal, so §7's two divergences see nothing.
 *
 * So this is the third, and it is decided here because here is the only place
 * both facts are known: that the prefix wrote to this store (a replayed write,
 * noted below), and that this op is past the frontier (`slot.held` is
 * `undefined`). It is a [`ReplayDivergence`] because it has to travel exactly
 * where one travels — past `retry:`, `on_error:`, `on_item_error:` and
 * `detach:`, and without closing the execution's row — and it carries its own
 * opening clause, because the composition is not what disagreed.
 */
function inProcessState(
  store: StoreBinding,
  op: StoreOp,
  execution: runtime.ExecutionIdentity,
  slot: EffectSlot,
): void {
  if (!inProcessOnly(store)) return;
  if (slot.held !== undefined) {
    // Replayed. A write in the prefix is what makes the store's contents
    // unreconstructable; a read is not — it answered out of a store this
    // generation's copy matches, empty for empty.
    if (!writes(op)) return;
    let held = IN_PROCESS_ONLY.get(execution.id);
    if (held === undefined) {
      held = new Set();
      IN_PROCESS_ONLY.set(execution.id, held);
    }
    held.add(store.address);
    return;
  }
  if (IN_PROCESS_ONLY.get(execution.id)?.has(store.address) !== true) return;
  const lifetime =
    store.backend.provider === "memory"
      ? `\`provider: memory\` (${store.backend.from})`
      : `\`scope: execution\``;
  throw new ReplayDivergence(
    slot,
    `this execution's record holds a write to \`${store.address}\`, which is ${lifetime} — its rows live only in the process that opened it. That process is gone, and a replayed write is not applied a second time, so this op would answer out of an empty store rather than out of the world the record left behind. A store a resumed execution reads past the frontier has to outlive the run: \`scope: session\` or \`scope: global\` (\`docs/durability.md\` §5)`,
    "this execution's record cannot be replayed",
  );
}

/** One op's answer, and whether the backend had already applied its key. */
interface Applied {
  readonly row: Record<string, unknown>;
  readonly deduped: boolean;
}

// ---------------------------------------------------------------------------
// What a `kv` write decides (PRD resolved q68)
// ---------------------------------------------------------------------------
//
// One function every arm calls, inside whatever serializes that arm's writes to
// a key: SQLite's `BEGIN IMMEDIATE` locally, a row lock on a dialled server. It
// is handed what the key held when that lock was taken and answers what to do —
// nothing, remove the key, or write this value — and what to answer. So the
// conditions, the three partial updates and the answer rows are written once,
// and "the runtime cannot tell which arm answered" stays true by construction
// of one more thing.

/** What one `kv` key holds, read under the lock its write holds. */
interface Held {
  /** The stored value, parsed — the JSON this module wrote. */
  readonly value: unknown;
  /** The version the write that stored it took. */
  readonly version: number;
}

/**
 * Whether a key holds nothing, which is what `if: absent` asks.
 *
 * One predicate, so that every reading of "absent" — the condition, the
 * declined answer's missing `version`, and a partial update starting from
 * nothing — is this one. A key whose time is up is absent here too (PRD
 * resolved q69): every arm reads the row a write holds through [`heldRow`],
 * which answers `undefined` for one [`live`] says has expired — so the
 * conditions, the decline and the partial updates see an expired key exactly
 * as they see a missing one, and a `get`, a `list`, a `count` and a `get_many`
 * do by the same inequality ([`liveRows`]).
 */
function absent(held: Held | undefined): held is undefined {
  return held === undefined;
}

/**
 * What one stored row holds, as of `now` — or `undefined` where it holds nothing,
 * which is a row that is not there **or** one whose time is up.
 */
function heldRow(row: Row | null | undefined, now: number): Held | undefined {
  if (row === null || row === undefined) return undefined;
  if (!live(row["expires_at"], now)) return undefined;
  return heldOf(row);
}

/** What one `kv` write decided, given what its key held. */
type KvDecision =
  | { readonly effect: "none"; readonly answer: Record<string, unknown> }
  | { readonly effect: "remove"; readonly answer: Record<string, unknown> }
  // Applied, and left the value and the version as they were — an `append` with
  // `unique: true` whose item is already there — but a write that applied
  // refreshes its key's expiry like any other (PRD resolved q69).
  | { readonly effect: "touch"; readonly answer: Record<string, unknown> }
  | {
      readonly effect: "write";
      readonly value: unknown;
      readonly answer: (version: number) => Record<string, unknown>;
    };

/** Whether a write's condition holds of what its key held (grammar 11.4). */
function conditionHolds(params: StoreParams, held: Held | undefined): boolean {
  if (params.condition === "absent") return absent(held);
  if (params.condition === "present") return !absent(held);
  if (params.ifVersion !== undefined) return !absent(held) && held.version === params.ifVersion;
  return true;
}

/**
 * What one `kv` write does, and answers, given what its key held.
 *
 * A condition that does not hold applies nothing and answers `applied: false`
 * with the version the key holds where it holds one — never a failure. A
 * `merge`, `append` or `incr` computes the whole value it would leave and holds
 * it to the store's `value_schema` before anything is written; one the schema
 * refuses throws, and the caller's transaction takes nothing with it. An
 * `append` with `unique: true` whose item is already there changes neither the
 * value nor the version: it is answered as applied — the item is in the array
 * — with the version and the value the key already holds, and its key's expiry
 * is refreshed as every applied write's is (PRD resolved q69).
 */
function decideKvWrite(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  held: Held | undefined,
  via: Issuer,
): KvDecision {
  const key = required(params.key, "key", store);
  // A condition that does not hold answers what the key still holds, on a
  // `delete` exactly as on every other write: the version is what the next
  // compare-and-set needs, and an `if_version:` delete that lost is the case
  // most likely to make one (PRD resolved q68 (b)).
  const declined = absent(held) ? {} : { version: held.version };
  if (op === "delete") {
    if (!conditionHolds(params, held)) {
      return { effect: "none", answer: { deleted: false, applied: false, ...declined } };
    }
    // A delete that applied leaves the key holding nothing, so it answers no
    // version — the same "absent where the key holds nothing" a `get` reads.
    return { effect: "remove", answer: { deleted: !absent(held), applied: true } };
  }
  if (!conditionHolds(params, held)) {
    return { effect: "none", answer: { key, applied: false, ...declined } };
  }
  if (op === "set") {
    return {
      effect: "write",
      value: params.value ?? {},
      answer: (version) => ({ key, applied: true, version }),
    };
  }
  const result = partialUpdate(store, op, key, params, held, via);
  admitted(store, op, key, result.value, via);
  if (!result.changed && !absent(held)) {
    return {
      effect: "touch",
      answer: { key, applied: true, version: held.version, value: result.value },
    };
  }
  return {
    effect: "write",
    value: result.value,
    answer: (version) => ({ key, applied: true, version, value: result.value }),
  };
}

/**
 * The whole value a `merge`, `append` or `incr` would leave at its key.
 *
 * An absent key starts from `{}`: a `merge` then has to supply every field the
 * schema requires, an `append` starts its array from `[]` and an `incr` its
 * number from `0`. A stored value whose field is the wrong shape for the op —
 * a schema that changed under it — is refused naming the field, because the
 * stored value is what this op is about to build on.
 */
function partialUpdate(
  store: StoreBinding,
  op: StoreOp,
  key: string,
  params: StoreParams,
  held: Held | undefined,
  via: Issuer,
): { readonly value: Record<string, unknown>; readonly changed: boolean } {
  const stored = absent(held) ? {} : held.value;
  if (stored === null || typeof stored !== "object" || Array.isArray(stored)) {
    throw refused(store, op, key, `the stored value is ${excerpt(stored)}, which is not an object`, via);
  }
  const base = stored as Record<string, unknown>;
  switch (op) {
    case "merge":
      return { value: { ...base, ...(params.fields ?? {}) }, changed: true };
    case "append": {
      const field = required(params.field, "field", store);
      const items = base[field] ?? [];
      if (!Array.isArray(items)) {
        throw refused(store, op, key, `${field}: the stored value holds ${excerpt(items)}, which is not an array`, via);
      }
      if (params.unique === true && items.some((item) => sameJson(item, params.value))) {
        return { value: base, changed: false };
      }
      return { value: { ...base, [field]: [...items, params.value] }, changed: true };
    }
    case "incr": {
      const field = required(params.field, "field", store);
      const current = base[field] ?? 0;
      if (typeof current !== "number") {
        throw refused(store, op, key, `${field}: the stored value holds ${excerpt(current)}, which is not a number`, via);
      }
      return { value: { ...base, [field]: current + (params.by ?? 1) }, changed: true };
    }
    default:
      throw new Error(`\`${store.address}\` is \`kind: kv\` and takes no \`${op}\``);
  }
}

/** Hold a value a partial update would commit to the store's `value_schema`. */
function admitted(
  store: StoreBinding,
  op: StoreOp,
  key: string,
  value: unknown,
  via: Issuer,
): void {
  const schema = store.value;
  if (schema === undefined) return;
  const parsed = schema.safeParse(value);
  if (parsed.success) return;
  throw refused(store, op, key, runtime.describeIssues(value, parsed.error?.issues ?? []), via);
}

/**
 * A partial update its result refuses: nothing was written.
 *
 * Which class it is depends on who asked, and that is Decision D119's split
 * rather than a choice made here. A store-op **node**'s parameters are the
 * author's, so the op fails the node and its `retry:`/`on_error:` decide. A
 * synthesized **tool**'s are the model's, and a call the store's contract
 * refuses goes back to the model to make again — the same as a call its
 * argument schema refuses — so it is a [`runtime.ToolCallRefused`].
 */
function refused(
  store: StoreBinding,
  op: StoreOp,
  key: string,
  detail: string,
  via: Issuer,
): Error {
  const message = `\`${store.address}\` refused the \`${op}\` at key ${JSON.stringify(key)}: the value it would leave is one the store's \`value_schema\` does not admit — ${detail}. Nothing was written: a \`merge\`, \`append\` or \`incr\` holds the whole value it commits to the schema, what was already stored included (grammar 11.4, PRD resolved q68)`;
  return via === "tool" ? new runtime.ToolCallRefused(message) : new Error(message);
}

// ---------------------------------------------------------------------------
// What a `batch` decides (PRD resolved q71)
// ---------------------------------------------------------------------------
//
// A batch is the shape of one `kv` write with N entries inside its one
// transaction — not N writes. Each arm takes what a write takes **once**: the
// partition's sequence lock (a dialled server's `FOR UPDATE`, SQLite's
// `BEGIN IMMEDIATE`), the ledger check that answers a retry with the stored
// row, the per-write sweep and the clock. Then it walks the entries in order,
// each one exactly a single `set` or `delete`: it reads its key under the lock
// a write of it would take ([`heldRow`]), and [`batchStep`] — which is
// [`decideKvWrite`] — says what that entry does. An entry that lands lands at
// once, inside the transaction, so a later entry naming the same key reads
// what an earlier one left. A `set` entry takes the partition's next version,
// in entry order, and its own expiry ([`expiryOf`] over the entry's `ttl`, else
// the store's `default_ttl:`).
//
// **The first entry that cannot land decides the batch, and nothing lands.** A
// condition that does not hold rolls every earlier entry back to the savepoint
// taken before the first, and the batch answers `applied: false` naming that
// entry's index as `failed` — an answer like a declined `set`'s, so it is the
// ledger row too, and a retry answers it again. An entry's value is checked
// against `value_schema` exactly as its single op's is — by CEL typing at
// `validate`, the check PRD resolved q68 gives a store-op node — and written
// as it evaluated, so no entry is refused here that its `set` would not be. A
// statement that fails throws, and the transaction takes everything with it
// but the error, with no version consumed. A crash anywhere between the first
// entry and the `COMMIT` is a transaction that never committed, on every arm.

/** The most entries one `batch` applies (grammar 11.4, PRD resolved q71). */
const BATCH_MAX_ENTRIES = 100;

/**
 * A `batch`'s entries, held to the shape the compiler already decided: one to
 * [`BATCH_MAX_ENTRIES`] of them, each a `set` with a key and a value or a
 * `delete` with a key, and none carrying a parameter its op does not take.
 *
 * The compiler emits nothing else, so this refuses a caller that skipped it —
 * by name, before the journal and before any store is reached — rather than
 * letting a malformed entry be half of a transaction.
 */
function batchEntries(store: StoreBinding, params: StoreParams): readonly BatchEntryParams[] {
  const entries = params.ops;
  const refuse = (detail: string): Error =>
    new Error(
      `a \`batch\` on \`${store.address}\` was handed ${detail}: a batch applies 1 to ${BATCH_MAX_ENTRIES} entries, each a \`set\` of one key's whole value or a \`delete\` of one key (grammar 11.4, PRD resolved q71)`,
    );
  if (!Array.isArray(entries)) throw refuse("no list of entries");
  if (entries.length === 0 || entries.length > BATCH_MAX_ENTRIES) {
    throw refuse(`${entries.length} entries`);
  }
  entries.forEach((entry: BatchEntryParams, index: number) => {
    if (entry === null || typeof entry !== "object") throw refuse(`an entry ${index} that is not an object`);
    if (entry.op !== "set" && entry.op !== "delete") {
      throw refuse(`an entry ${index} whose op is ${JSON.stringify(entry.op ?? null)}`);
    }
    if (typeof entry.key !== "string") throw refuse(`an entry ${index} with no key`);
    if (entry.op === "set" && entry.value === undefined) throw refuse(`a \`set\` entry ${index} with no value`);
    if (
      entry.op === "delete" &&
      (entry.value !== undefined || entry.condition !== undefined || entry.ttl !== undefined)
    ) {
      throw refuse(`a \`delete\` entry ${index} carrying a value, an \`if:\` or a \`ttl\``);
    }
    if (entry.condition !== undefined && entry.ifVersion !== undefined) {
      throw refuse(`an entry ${index} with two conditions`);
    }
  });
  return entries;
}

/** What one `batch` entry does, given what its key held. */
type BatchStep =
  | { readonly effect: "none" }
  | { readonly effect: "remove" }
  | { readonly effect: "write"; readonly value: unknown };

/**
 * One `batch` entry, decided: [`decideKvWrite`] over what its key held — the
 * very decision a single `set` or `delete` makes, with the same conditions,
 * so an entry cannot mean anything its single op would not.
 *
 * A `set` entry's value is not parsed against the store's `value_schema`
 * here, because a single `set` node's is not: the schema check PRD resolved
 * q71 gives every entry is q68's — CEL typing at `validate` for a store-op
 * node, the emitted Zod at run time for a tool's arguments — and a batch is a
 * node's alone. So an entry is written as its CEL evaluated, exactly as the
 * `set` it spells would be, and a constraint CEL typing does not decide — a
 * bound, a pattern — holds of an entry no more and no less than of that `set`
 * (grammar 11.4's schema evolution).
 */
function batchStep(store: StoreBinding, entry: BatchEntryParams, held: Held | undefined): BatchStep {
  const decision = decideKvWrite(store, entry.op, entry, held, "node");
  switch (decision.effect) {
    case "none":
      return { effect: "none" };
    case "remove":
      return { effect: "remove" };
    case "write":
      return { effect: "write", value: decision.value };
    case "touch":
      // Only an `append` with `unique: true` touches, and no entry is one.
      throw new Error(`a \`batch\` entry on \`${store.address}\` is a \`${entry.op}\`, which never only touches its key`);
  }
}

/**
 * What a batch whose entry `failed` declined answers: nothing applied, no
 * results, and the entry whose condition did not hold — the row a retry is
 * handed again (PRD resolved q71).
 */
function declinedBatch(failed: number): Record<string, unknown> {
  return { applied: false, deduped: false, results: [], failed };
}

/** Two JSON values as one, whatever order an object's keys were written in. */
function sameJson(left: unknown, right: unknown): boolean {
  return canonicalJson(left) === canonicalJson(right);
}

function canonicalJson(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (value !== null && typeof value === "object") {
    const entries = Object.entries(value as Record<string, unknown>)
      .filter(([, held]) => held !== undefined)
      .sort(([left], [right]) => (left < right ? -1 : left > right ? 1 : 0));
    return `{${entries.map(([name, held]) => `${JSON.stringify(name)}:${canonicalJson(held)}`).join(",")}}`;
  }
  return JSON.stringify(value) ?? "null";
}

/** A value as a message quotes it: JSON, and short enough to read. */
function excerpt(value: unknown): string {
  const rendered = JSON.stringify(value) ?? String(value);
  return rendered.length <= 120 ? rendered : `${rendered.slice(0, 117)}...`;
}

/**
 * What a write the ledger already holds is answered with: its first attempt's
 * answer, whole, marked `deduped`.
 *
 * A `kv` write's answer carries `deduped` itself (PRD resolved q68), so the
 * stored one — written `false` by the attempt that applied — is answered
 * `true` here. A ledger row an earlier release wrote holds no `applied`, and
 * every write that release made was unconditional and landed, so it is
 * answered as one that did.
 */
function redelivered(store: StoreBinding, op: StoreOp, result: string): Record<string, unknown> {
  const row = JSON.parse(result) as Record<string, unknown>;
  if (!kvWrite(store, op)) return row;
  return "applied" in row ? { ...row, deduped: true } : { ...row, applied: true, deduped: true };
}

/**
 * Refuse a backend this compiler release does not implement (PRD §7 M3).
 *
 * The set it refuses **narrowed** with PRD resolved q63: `postgres` and `mysql`
 * are implemented for the `kv` kind, behind this same interface and with this
 * same op catalogue, so a store bound to one is answered rather than refused.
 * They are named in the message as the shape the rest will arrive in — one arm
 * per provider under `runStoreOp`, not a second store surface — because an
 * author reading this is being told what exists as much as what does not.
 */
function supported(store: StoreBinding): void {
  if (opensInProcess(store.backend.provider)) return;
  if (DIALLED_KV.includes(store.backend.provider)) return;
  throw new Error(
    `\`${store.address}\` is bound to the \`${store.backend.provider}\` backend (${store.backend.from}), which this compiler release does not implement: the \`postgres\` and \`mysql\` \`kv\` backends are implemented and answer the same ops behind the same interface (PRD resolved q63), and the rest of grammar 14.3's vocabulary — Redis, pgvector, Qdrant, S3 and the rest — lands in that shape behind the store plugin interface in M3 (PRD §7). Bind one of the implemented backends, or build for \`--target local\`, which substitutes SQLite and local disk for every store unconditionally (PRD 5.8)`,
  );
}

async function perform(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  execution: runtime.ExecutionIdentity,
  idempotencyKey: string | undefined,
  via: Issuer,
  signal: AbortSignal,
  actor: HistoryActor,
): Promise<Applied> {
  if (store.kind === "blob") {
    return blobOp(store, op, params, scopeKey, idempotencyKey);
  }
  if (!opensInProcess(store.backend.provider)) {
    // A dialled `kv` store. `vector` and `blob` never arrive here: grammar
    // 14.3's capability check refuses a `postgres`/`mysql` binding on either
    // kind at compile time, and [`supported`] above has already refused every
    // other networked provider.
    return await dialled(store, op, params, scopeKey, idempotencyKey, via, actor);
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

  // `BEGIN IMMEDIATE` is SQLite's serialized writer, and it is what makes a
  // `kv` write's read of its key, its decision and its commit one step that no
  // other writer — another instance of a `map` in this process, or another
  // process on the same file — can land inside (PRD resolved q68).
  return transact(database, () => {
    // The op's one clock, read once inside its transaction — a `kv` op's
    // only, the one kind whose keys and authored ledger rows expire (PRD
    // resolved q69, q73): what its ledger read judges a row by, what its
    // sweeps remove and when what it leaves expires.
    const now = store.kind === "kv" ? localNow() : undefined;
    if (idempotencyKey !== undefined) {
      const seen = (
        now === undefined
          ? database.get("SELECT op, result FROM applied WHERE idempotency_key = ?", [
              idempotencyKey,
            ])
          : database.get(
              `SELECT op, result FROM applied WHERE idempotency_key = ? AND ${liveClause("?")}`,
              [idempotencyKey, now],
            )
      ) as Row | null;
      if (seen !== null) return answeredFrom(store, op, idempotencyKey, seen);
      // An authored key's row may be there and expired, answering nothing: it
      // goes, so this write's claim takes its place — and with it at most a
      // hundred of the store's other expired ledger rows, the write half of the
      // ledger's cleanup schedule (PRD resolved q73).
      if (authored(idempotencyKey) && now !== undefined) {
        sweepLocalLedger(database, now);
        database.run("DELETE FROM applied WHERE idempotency_key = ?", [idempotencyKey]);
      }
    }
    const row =
      store.kind === "kv"
        ? localKv(store, op, params, scopeKey, database, via, now ?? localNow(), actor)
        : tabular(store, op, params, scopeKey, database, vector);
    if (idempotencyKey !== undefined) {
      database.run(
        "INSERT INTO applied (idempotency_key, op, result, expires_at) VALUES (?, ?, ?, ?)",
        [
          idempotencyKey,
          op,
          JSON.stringify(row),
          now === undefined ? null : ledgerExpiry(store, idempotencyKey, now),
        ],
      );
    }
    return { row, deduped: false };
  });
}

/**
 * The `kv` rows of grammar 11.4's catalog on the local backend, inside the
 * transaction [`perform`] opened.
 *
 * A write reads what its key holds, asks [`decideKvWrite`] what that means,
 * and does it — taking the partition's next version where something lands
 * ([`nextLocalVersion`]). The transaction is SQLite's serialized writer, so
 * nothing lands between the read and the write.
 *
 * The clock is read once, inside the transaction, and is the op's whole notion
 * of now (PRD resolved q69): what is alive to its read, what its sweep removes,
 * and when the key it leaves expires. [`perform`] reads it, so the ledger row
 * an authored write leaves is dated by the same instant (PRD resolved q73) —
 * and so is the history row a `history: true` store's write records, after the
 * entry's own statement and inside the same transaction (PRD resolved q74).
 */
function localKv(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  database: Database,
  via: Issuer,
  now: number,
  actor: HistoryActor,
): Record<string, unknown> {
  // A history row, written in this transaction — or nothing, on a store that
  // keeps none ([`historyRow`]).
  const history = (write: HistoryWrite): void => {
    const row = historyRow(store, write, actor, now);
    if (row === undefined) return;
    database.run(`INSERT INTO history (scope_key, ${row.columns}) VALUES (?, ${row.placeholders})`, [
      scopeKey,
      ...row.bound,
    ]);
  };
  switch (op) {
    case "get": {
      const key = required(params.key, "key", store);
      const held = localHeld(database, scopeKey, key, now);
      // A miss answers `found: false` and **no** `value` or `version` at all
      // (Decision D110, PRD resolved q68) — an expired key included, which is
      // absent (PRD resolved q69).
      return held === undefined
        ? { found: false }
        : { value: held.value, found: true, version: held.version };
    }
    // The reads of PRD resolved q70, as statements every arm shares
    // ([`partitionRead`]): SQLite takes them as they are written, and judges
    // a key's expiry by the op's clock, bound (PRD resolved q69).
    case "list":
    case "count":
    case "get_many": {
      const read = partitionRead(store, op, params, {
        from: "entries WHERE scope_key = ?",
        bound: [scopeKey],
        now: { sql: "?", bound: [now] },
      });
      return read.answer(
        read.query === undefined ? [] : (database.all(read.query.sql, [...read.query.bound]) as Row[]),
      );
    }
    // One page of one key's history (PRD resolved q74), as the statement
    // every arm shares ([`historyRead`]), judged by the op's clock, bound.
    case "history": {
      const read = historyRead(store, params, {
        from: "history WHERE scope_key = ?",
        bound: [scopeKey],
        now: { sql: "?", bound: [now] },
      });
      return read.answer(database.all(read.query.sql, [...read.query.bound]) as Row[]);
    }
    case "set":
    case "delete":
    case "merge":
    case "append":
    case "incr": {
      const key = required(params.key, "key", store);
      // A write to a partition is the first half of the cleanup schedule: at
      // most a hundred of its expired rows leave storage, before the key is
      // read so that one of them may be this key (PRD resolved q69) — and, on
      // a store with a `history_retention:`, at most a hundred of its history
      // rows past it (PRD resolved q74).
      sweepLocalPartition(database, scopeKey, now);
      sweepLocalHistory(database, store, scopeKey, now);
      const held = localHeld(database, scopeKey, key, now);
      const decision = decideKvWrite(store, op, params, held, via);
      switch (decision.effect) {
        case "none":
          return { ...decision.answer, deduped: false };
        case "remove":
          database.run("DELETE FROM entries WHERE scope_key = ? AND key = ?", [scopeKey, key]);
          history({ op, key, before: held, after: undefined });
          return { ...decision.answer, deduped: false };
        case "touch":
          database.run("UPDATE entries SET expires_at = ? WHERE scope_key = ? AND key = ?", [
            expiryOf(store, params, now),
            scopeKey,
            key,
          ]);
          history({ op, key, before: held, after: held });
          return { ...decision.answer, deduped: false };
        case "write": {
          const version = nextLocalVersion(database, scopeKey);
          database.run(
            "INSERT INTO entries (scope_key, key, value, version, expires_at) VALUES (?, ?, ?, ?, ?) " +
              "ON CONFLICT (scope_key, key) DO UPDATE SET value = excluded.value, " +
              "version = excluded.version, expires_at = excluded.expires_at",
            [scopeKey, key, JSON.stringify(decision.value), version, expiryOf(store, params, now)],
          );
          history({ op, key, before: held, after: { value: decision.value, version } });
          return { ...decision.answer(version), deduped: false };
        }
      }
      break;
    }
    case "batch":
      return localBatch(store, params, scopeKey, database, now, history);
    default:
      break;
  }
  throw new Error(`\`${store.address}\` is \`kind: kv\` and takes no \`${op}\``);
}

/**
 * A `batch` on the local backend, inside the `BEGIN IMMEDIATE` transaction
 * [`perform`] opened — whose ledger check has already answered a retry — and
 * at the one instant `now` the op reads its clock (PRD resolved q71).
 *
 * The partition's expired rows are swept once, as one write sweeps them, and a
 * savepoint is taken before the first entry, so an entry whose condition does
 * not hold takes every earlier entry back with it while the ledger row the
 * decline is answered by still commits. Each entry that lands is written at
 * once — [`nextLocalVersion`] numbers them in entry order — so a later entry
 * naming the same key reads what the earlier one left. On a `history: true`
 * store each entry that lands records its own history row as it lands, under
 * the same savepoint, so the decline that takes the entries back takes their
 * rows with them (PRD resolved q74).
 */
function localBatch(
  store: StoreBinding,
  params: StoreParams,
  scopeKey: string,
  database: Database,
  now: number,
  history: (write: HistoryWrite) => void,
): Record<string, unknown> {
  const entries = batchEntries(store, params);
  sweepLocalPartition(database, scopeKey, now);
  sweepLocalHistory(database, store, scopeKey, now);
  database.exec("SAVEPOINT batch_entries");
  const results: Record<string, unknown>[] = [];
  for (const [index, entry] of entries.entries()) {
    const held = localHeld(database, scopeKey, entry.key, now);
    const step = batchStep(store, entry, held);
    if (step.effect === "none") {
      database.exec("ROLLBACK TO SAVEPOINT batch_entries");
      database.exec("RELEASE SAVEPOINT batch_entries");
      return declinedBatch(index);
    }
    if (step.effect === "remove") {
      database.run("DELETE FROM entries WHERE scope_key = ? AND key = ?", [scopeKey, entry.key]);
      history({ op: entry.op, key: entry.key, before: held, after: undefined });
      results.push({ key: entry.key });
      continue;
    }
    const version = nextLocalVersion(database, scopeKey);
    database.run(
      "INSERT INTO entries (scope_key, key, value, version, expires_at) VALUES (?, ?, ?, ?, ?) " +
        "ON CONFLICT (scope_key, key) DO UPDATE SET value = excluded.value, " +
        "version = excluded.version, expires_at = excluded.expires_at",
      [scopeKey, entry.key, JSON.stringify(step.value), version, expiryOf(store, entry, now)],
    );
    history({ op: entry.op, key: entry.key, before: held, after: { value: step.value, version } });
    results.push({ key: entry.key, version });
  }
  database.exec("RELEASE SAVEPOINT batch_entries");
  return { applied: true, deduped: false, results };
}

/**
 * What one key of a local store holds at `now`, or `undefined` where it holds
 * nothing — an expired key included ([`heldRow`]).
 */
function localHeld(database: Database, scopeKey: string, key: string, now: number): Held | undefined {
  const found = database.get(
    "SELECT value, version, expires_at FROM entries WHERE scope_key = ? AND key = ?",
    [scopeKey, key],
  ) as Row | null;
  return heldRow(found, now);
}

/**
 * The next version of one local partition's sequence, taken.
 *
 * The row is created on the partition's first write, starting past whatever
 * the partition already holds — `0` for a file an earlier release wrote, whose
 * rows all carry `0` ([`migrateStoreFile`]) — and every later write moves it
 * on by one. Inside [`perform`]'s transaction, so a write that is rolled back
 * gives its number back with everything else.
 */
function nextLocalVersion(database: Database, scopeKey: string): number {
  database.run(
    "INSERT INTO sequences (scope_key, last) VALUES (?, " +
      "(SELECT COALESCE(MAX(version), 0) FROM entries WHERE scope_key = ?)) " +
      "ON CONFLICT (scope_key) DO NOTHING",
    [scopeKey, scopeKey],
  );
  database.run("UPDATE sequences SET last = last + 1 WHERE scope_key = ?", [scopeKey]);
  const held = database.get("SELECT last FROM sequences WHERE scope_key = ?", [scopeKey]) as Row;
  return Number(held["last"]);
}

/**
 * The `vector` rows of grammar 11.4's catalog. A `vector` row never expires,
 * and no `vector` op reads a clock.
 */
function tabular(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  database: Database,
  vector: number[] | undefined,
): Record<string, unknown> {
  switch (op) {
    case "delete": {
      const key = required(params.key, "key", store);
      const result = database.run("DELETE FROM entries WHERE scope_key = ? AND key = ?", [
        scopeKey,
        key,
      ]);
      return { deleted: result.changes > 0 };
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
      // rows SQLite happened to hand back first. By the same key order the rows
      // arrived in ([`byUtf8Bytes`]), so one module has one answer to "which key
      // comes first".
      scored.sort((left, right) =>
        right.score === left.score
          ? byUtf8Bytes(left.id, right.id)
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

// ---------------------------------------------------------------------------
// Reading many keys at once: a page, a count, a `get_many` (PRD resolved q70)
// ---------------------------------------------------------------------------
//
// One implementation of the three, as statements every `kv` arm runs: SQLite
// takes them as they are written and a dialled arm through its placeholder
// dialect, so the cursor, the order a page comes back in, what `next` means and
// which keys a `count` counts are decided once — and "the runtime cannot tell
// which arm answered" stays true of them by construction rather than by review.
// The only thing an arm supplies is where its partition's rows are
// ([`PartitionRows`]).
//
// **The order is UTF-8 byte order on every arm**, and each arm's key column is
// what decides it: SQLite's default `BINARY` collation compares the UTF-8 the
// file holds with `memcmp`, the Postgres arm declares the column `COLLATE "C"`,
// and the MySQL arm `utf8mb4_0900_bin`, a NO PAD binary collation, with
// `max_sort_length` sized past the column so a sort compares the whole key. A
// comparison between the column and a bound value is decided by the column's
// collation on every arm — a column's collation outranks a parameter's or a
// literal's on both servers — so the cursor's `"key" > ?` is the same byte
// comparison `ORDER BY "key"` sorts by, and a page boundary can neither skip a
// key the order put after it nor repeat one it put before. UTF-8 byte order is
// code-point order, which is also [`byUtf8Bytes`] — the order a `blob` `list`
// sorts its file names in.
//
// **A page is keyset, not offset.** It is "the first `limit` keys after the
// cursor", asked for as `limit + 1` so the extra row says whether any remain:
// `next` is present exactly when one does, and is the last key of the page,
// which is the cursor the page after it takes. On SQLite and Postgres the
// cursor is a range scan of the partition's primary key, so a walk costs a
// page per page however deep it has gone; the MySQL arm's primary key is a
// hash (`MYSQL_STORE_SCHEMA`), so its partition index narrows the range on the
// key's leading characters and the page is sorted from there — the same
// answer, at a cost that grows with what is left of the walk. And a write
// landing mid-walk moves no page the walk has yet to read: a key added behind
// the cursor is not seen by this walk, one added ahead of it is, a key deleted
// ahead of it is not, and nothing already answered is answered twice — which
// an offset could promise none of.
//
// **Which rows are live is one fragment, [`liveRows`].** Every read of many
// rows applies it, on every arm, and the single-row reads answer through
// [`heldRow`]. Both are [`live`]'s one inequality — a row with no expiry, or
// one whose expiry is still ahead of the op's clock — which is how PRD
// resolved q69's expiry makes a key whose time is up invisible to every read
// and uncounted by every `count`: a page, its `count`, a `get_many` and a
// `get` cannot come to disagree about which keys a partition holds. The clock
// sits inside the clause: bound on the local backend, the server's own on a
// dialled one, so a read there is one statement judged by the clock its
// writers dated their keys with.

/** The most keys one `get_many` reads (grammar 11.4, PRD resolved q70). */
const GET_MANY_MAX_KEYS = 1000;

/** One statement and what it binds, in `?` placeholders. */
interface Query {
  readonly sql: string;
  readonly bound: readonly Bound[];
}

/**
 * Where one `kv` partition's rows are: a table and the condition that selects
 * the partition in it, in `?` placeholders, with what those bind, and the
 * instant the read judges a key's expiry by — the whole of what differs
 * between the arms for a read of many rows.
 */
interface PartitionRows {
  readonly from: string;
  readonly bound: readonly Bound[];
  /**
   * The op's clock, as an expression and what it binds: the instant the local
   * backend read for the op, bound as a `?`, and the server's own clock
   * shifted by [`STORE_CLOCK_OFFSET`] on a dialled one (PRD resolved q69).
   */
  readonly now: Query;
}

/** A read of many rows, planned: what to ask, and what the rows answer. */
interface PartitionRead {
  /** `undefined` where there is nothing to ask — a `get_many` of no keys. */
  readonly query: Query | undefined;
  readonly answer: (rows: readonly Row[]) => Record<string, unknown>;
}

/**
 * The row filter every multi-row `kv` read applies, on every arm: which
 * stored rows are live at `now`.
 *
 * [`live`]'s inequality, as [`liveClause`] writes it over the op's clock: a
 * row with no expiry, or one whose expiry is still ahead of `now`. "A key whose
 * time is up is invisible to every read" (PRD resolved q69) is true because it
 * is this one place for a `list`, a `count` and a `get_many`, and [`heldRow`]
 * — the same inequality — for a single key, and nowhere else.
 */
function liveRows(now: Query): Query {
  return { sql: liveClause(now.sql), bound: now.bound };
}

/**
 * The `prefix:` filter, as one condition on the key column — the empty prefix
 * when there is none, which every key has.
 *
 * Characters, not JavaScript string length. `SUBSTR` over text counts
 * **characters** on all three servers, and a JavaScript `.length` counts UTF-16
 * code units — so an emoji in the prefix makes the two disagree by one per
 * astral character, and the comparison silently answers with the wrong keys
 * (usually none) for a `prefix:` grammar 11.4 puts no restriction on. Spreading
 * the string iterates code points, which is what the servers count.
 */
function underPrefix(prefix: string | undefined): Query {
  const held = prefix ?? "";
  return { sql: 'SUBSTR("key", 1, ?) = ?', bound: [[...held].length, held] };
}

/**
 * A `get_many`'s keys, held to what grammar 11.4's row says they are: a list
 * of at most [`GET_MANY_MAX_KEYS`] strings.
 *
 * `validate` types the expression as a list of strings, and its **length** is
 * the half no type decides — so the bound is a refusal the node fails with,
 * by its own retry and error policy, naming the count and the bound. The shape
 * is checked too, for the value a `dyn` expression can still produce.
 */
function getManyKeys(store: StoreBinding, params: StoreParams): readonly string[] {
  const keys = params.keys;
  if (!Array.isArray(keys) || keys.some((key) => typeof key !== "string")) {
    throw new Error(
      `the \`keys\` of a \`get_many\` on \`${store.address}\` evaluated to ${excerpt(keys)}, which is not a list of strings (grammar 11.4)`,
    );
  }
  if (keys.length > GET_MANY_MAX_KEYS) {
    throw new Error(
      `a \`get_many\` on \`${store.address}\` was asked for ${keys.length} keys, and one \`get_many\` reads at most ${GET_MANY_MAX_KEYS} (grammar 11.4, PRD resolved q70): read them in slices of ${GET_MANY_MAX_KEYS} or fewer, or walk the store a page at a time with \`list\` and \`after:\``,
    );
  }
  return keys as readonly string[];
}

/** A stored row's value and version, as every `kv` read answers them. */
function heldOf(row: Row): Held {
  return { value: JSON.parse(String(row["value"])) as unknown, version: Number(row["version"]) };
}

/**
 * One of PRD resolved q70's reads of a partition — a `list` page, a `count`,
 * a `get_many` — as the statement an arm runs over `rows` and the answer its
 * result is.
 */
function partitionRead(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  rows: PartitionRows,
): PartitionRead {
  const alive = liveRows(rows.now);
  switch (op) {
    case "list": {
      const limit = required(params.limit, "limit", store);
      const prefix = underPrefix(params.prefix);
      const values = params.includeValues === true;
      const cursor: Query =
        params.after === undefined ? { sql: "1 = 1", bound: [] } : { sql: '"key" > ?', bound: [params.after] };
      const query: Query = {
        sql:
          `SELECT "key"${values ? ", value, version" : ""} FROM ${rows.from} ` +
          `AND ${prefix.sql} AND ${cursor.sql} AND ${alive.sql} ORDER BY "key" LIMIT ?`,
        // One past the page, so the extra row says whether another follows.
        bound: [...rows.bound, ...prefix.bound, ...cursor.bound, ...alive.bound, limit + 1],
      };
      return {
        query,
        answer: (found) => {
          const page = found.slice(0, limit);
          const last = page[page.length - 1];
          return {
            keys: page.map((row) => String(row["key"])),
            // The cursor of the page after this one, there exactly when the
            // extra row was — so a page that ends the partition answers none,
            // and writes nothing (Decision D110).
            ...(found.length > limit && last !== undefined ? { next: String(last["key"]) } : {}),
            ...(values
              ? {
                  entries: page.map((row) => {
                    const held = heldOf(row);
                    return { key: String(row["key"]), value: held.value, version: held.version };
                  }),
                }
              : {}),
          };
        },
      };
    }
    case "count": {
      const prefix = underPrefix(params.prefix);
      return {
        query: {
          sql: `SELECT COUNT(*) AS held FROM ${rows.from} AND ${prefix.sql} AND ${alive.sql}`,
          bound: [...rows.bound, ...prefix.bound, ...alive.bound],
        },
        // `BIGINT` arrives as a string from `pg` and a number elsewhere.
        answer: (found) => ({ count: Number(found[0]?.["held"] ?? 0) }),
      };
    }
    case "get_many": {
      const keys = getManyKeys(store, params);
      // One statement however many keys, each asked for once: a key named
      // twice is answered twice below, out of one row.
      const distinct = [...new Set(keys)];
      return {
        query:
          distinct.length === 0
            ? undefined
            : {
                sql:
                  `SELECT "key", value, version FROM ${rows.from} ` +
                  `AND "key" IN (${distinct.map(() => "?").join(", ")}) AND ${alive.sql}`,
                bound: [...rows.bound, ...distinct, ...alive.bound],
              },
        answer: (found) => {
          const byKey = new Map<string, Held>();
          for (const row of found) byKey.set(String(row["key"]), heldOf(row));
          return {
            // In the order asked, one entry per key asked for. A miss answers
            // `found: false` and no `value` or `version`, which is a `get`'s
            // miss one key at a time (Decision D110, PRD resolved q68).
            entries: keys.map((key) => {
              const held = byKey.get(key);
              return absent(held)
                ? { key, found: false }
                : { key, value: held.value, found: true, version: held.version };
            }),
          };
        },
      };
    }
    default:
      throw new Error(`\`${store.address}\` is \`kind: kv\` and reads no \`${op}\` of many keys`);
  }
}

// ---------------------------------------------------------------------------
// One key's history (PRD resolved q74)
// ---------------------------------------------------------------------------
//
// What a history row holds and how a page of them is read, written once for
// every `kv` arm: the local backend's `history` table and a dialled server's
// `store_history` differ in their name, the columns that select a partition,
// and the clock a read judges retention by — which is exactly what
// [`PartitionRows`] carries — and in nothing else. The module header says what
// records a row and what does not.

/** One write that landed, as its history row records it (PRD resolved q74). */
interface HistoryWrite {
  /** The write: a single op, or a `batch` entry's own `set` or `delete`. */
  readonly op: StoreOp;
  readonly key: string;
  /** What the key held before the write, read under its lock — `undefined` where it held nothing. */
  readonly before: Held | undefined;
  /** What it holds after — `undefined` where the write left it holding nothing. */
  readonly after: Held | undefined;
}

/**
 * The columns a history row is inserted into after the ones that name its
 * partition, in the order [`historyRow`] binds them. `"key"` is quoted because
 * it is a keyword on one server and an identifier on all three.
 */
const HISTORY_COLUMNS = [
  '"key"',
  "op",
  "old_value",
  "new_value",
  "version",
  "actor_kind",
  "actor_execution",
  "actor_path",
  "at",
] as const;

/**
 * One write's history row, as the columns it is inserted into after its
 * partition's own, their placeholders and what they bind — or `undefined` on a
 * store that keeps no history, whose writes record nothing (PRD resolved q74).
 *
 * The values are the JSON this module writes into the entry itself, `NULL`
 * where the key held nothing on that side of the write; the version is the one
 * the write left, `NULL` where it left the key holding nothing; and `at` is the
 * write's own clock, the one it judged its key by and dated the entry with.
 */
function historyRow(
  store: StoreBinding,
  write: HistoryWrite,
  actor: HistoryActor,
  now: number,
): { readonly columns: string; readonly placeholders: string; readonly bound: readonly Bound[] } | undefined {
  if (store.history !== true) return undefined;
  return {
    columns: HISTORY_COLUMNS.join(", "),
    placeholders: HISTORY_COLUMNS.map(() => "?").join(", "),
    bound: [
      write.key,
      write.op,
      write.before === undefined ? null : JSON.stringify(write.before.value),
      write.after === undefined ? null : JSON.stringify(write.after.value),
      write.after === undefined ? null : write.after.version,
      actor.kind,
      actor.execution ?? null,
      actor.path ?? null,
      now,
    ],
  };
}

/**
 * A `history` read's cursor, held to what one is: the empty string or nothing
 * for the first page, else a page's `next` — the last `seq` of that page, a
 * whole number in decimal — and refused on a store that keeps no history (PRD
 * resolved q74).
 *
 * The compiler refuses a `history` on a store without `history: true`, so the
 * store refusal is for a caller that is not the compiler. A cursor is CEL, so
 * what it evaluates to is a run-time fact: one that is not a cursor fails the
 * node by name before anything is read, as a `get_many` past its bound does.
 */
function historyCursor(store: StoreBinding, params: StoreParams): number {
  if (store.kind !== "kv" || store.history !== true) {
    throw new Error(
      `\`${store.address}\` keeps no history: its definition does not declare \`history: true\`, so no write to it has recorded a row for a \`history\` read to answer (grammar 11.1, PRD resolved q74)`,
    );
  }
  const after = params.after;
  if (after === undefined || after === "") return 0;
  if (!/^[0-9]+$/.test(after) || !Number.isSafeInteger(Number(after))) {
    throw new Error(
      `the \`after\` of a \`history\` on \`${store.address}\` evaluated to ${JSON.stringify(after)}, which is not a history cursor: a cursor is the \`next\` a page answered — the last \`seq\` of that page, a whole number in decimal — or the empty string for the first page (PRD resolved q74)`,
    );
  }
  return Number(after);
}

/**
 * One page of one key's history, as the statement an arm runs over `rows` and
 * the answer its result is (PRD resolved q74) — the `list` page's shape over a
 * key's rows rather than a partition's keys.
 *
 * The first `limit` rows after the cursor, oldest first, asked for as
 * `limit + 1` so the extra row says whether more remain: `next` is present
 * exactly then, and is the page's last `seq`. A row past the store's
 * `history_retention:` by the op's clock is not read, by the cut-off
 * [`historyCutoff`] sweeps by — the server's own clock on a dialled arm, the
 * one the rows were dated with.
 */
function historyRead(
  store: StoreBinding,
  params: StoreParams,
  rows: PartitionRows,
): { readonly query: Query; readonly answer: (found: readonly Row[]) => Record<string, unknown> } {
  const key = required(params.key, "key", store);
  const limit = required(params.limit, "limit", store);
  const after = historyCursor(store, params);
  const retained: Query =
    store.historyRetention === undefined
      ? { sql: "1 = 1", bound: [] }
      : { sql: `at > ${rows.now.sql} - ?`, bound: [...rows.now.bound, store.historyRetention] };
  return {
    query: {
      sql:
        `SELECT seq, op, old_value, new_value, version, actor_kind, actor_execution, actor_path, at ` +
        `FROM ${rows.from} AND "key" = ? AND seq > ? AND ${retained.sql} ORDER BY seq LIMIT ?`,
      bound: [...rows.bound, key, after, ...retained.bound, limit + 1],
    },
    answer: (found) => {
      const page = found.slice(0, limit);
      const last = page[page.length - 1];
      return {
        entries: page.map(historyEntry),
        // The cursor of the page after this one, there exactly when the extra
        // row was — so the last page answers none, and writes nothing
        // (Decision D110).
        ...(found.length > limit && last !== undefined ? { next: String(Number(last["seq"])) } : {}),
      };
    },
  };
}

/** One stored history row, as a `history` read answers it. */
function historyEntry(row: Row): HistoryEntry {
  const text = (held: unknown): string | undefined =>
    held === null || held === undefined ? undefined : String(held);
  const before = text(row["old_value"]);
  const after = text(row["new_value"]);
  const version = text(row["version"]);
  const execution = text(row["actor_execution"]);
  const path = text(row["actor_path"]);
  return {
    // `BIGINT` arrives as a string from `pg` and a number elsewhere.
    seq: Number(row["seq"]),
    op: String(row["op"]),
    ...(before === undefined ? {} : { old_value: JSON.parse(before) as unknown }),
    ...(after === undefined ? {} : { new_value: JSON.parse(after) as unknown }),
    ...(version === undefined ? {} : { version: Number(version) }),
    actor: {
      kind: String(row["actor_kind"]) as HistoryActor["kind"],
      ...(execution === undefined ? {} : { execution }),
      ...(path === undefined ? {} : { path }),
    },
    at: Number(row["at"]),
  };
}

// ---------------------------------------------------------------------------
// The dialled `kv` backends: Postgres and MySQL (PRD resolved q63)
// ---------------------------------------------------------------------------
//
// One implementation of the op catalogue, not two. Everything grammar 11.4's
// `kv` rows mean — which partition an op addresses, what a miss answers, the key
// order a `list` comes back in, and the idempotency ledger of grammar 9.4 — is
// written once, below, and a [`StoreDriver`] differs only in how it carries a
// statement to a server and a row back. That is the same shape `./journal.ts`
// takes for the same reason (PRD resolved q62), and it is what makes "the
// runtime cannot tell which arm answered" true by construction rather than by
// review: `runStoreOp` above reaches this through the one seam in [`perform`],
// and a store-op node, a synthesized tool and the trace record they file are all
// the same code on every backend.
//
// # What is deliberately *not* copied from the journal
//
// **The writer guard.** A journal has exactly one writer (`docs/durability.md`
// §2, PRD resolved q42) and its remote arms take a session-scoped lock to say
// so. A store is the opposite by design: grammar 14.1 rule 5 already governs who
// may reach one, and two processes reaching a dialled store are **two readers of
// one store** rather than a forked state — which is the whole reason that rule
// admits them. So nothing here holds a lock past one op, and two writers at
// different partitions never meet.
//
// # What one write does lock (PRD resolved q68)
//
// A write is a read-modify-write of its key — a condition reads what the key
// holds, a partial update builds on it, and every write takes the partition's
// next version — so each one is a transaction that takes two row locks before
// it reads anything: the partition's **sequence row**
// (`SELECT … FOR UPDATE` on `store_sequences`), and then the key's entry. The
// first is what serializes every write to one partition, across connections
// and across processes; the second is the row lock the ruling names, and the
// order — sequence, then entry, always — is what keeps two writers from ever
// waiting on each other in a circle. A lock is held for one op's few
// statements and given back at its `COMMIT`. What is left of last-write-wins is
// exactly what D149 now says it is: a plain unconditional `set` replaces
// whatever the write before it left, and two of them leave the value of
// whichever committed second. A composition that needs more than one key to
// move together is asking for a transaction across nodes, which is not what a
// `kv` store is.
//
// **The keepalive and reap-window machinery.** Those exist on the journal to
// bound how long a dead host's *guard* locks a live one out. With no guard there
// is nothing to reap: a connection this process loses is one refused op (the
// node fails under its `retry:`/`on_error:`), and the connection itself is
// dropped from [`DIALLED`] so the retry dials a fresh one. That last half is the
// other place the journal is deliberately not copied — a journal keeps its
// faulted connection *because* redialling would take the writer guard back
// behind the record's back, while a store has no guard and a redial is only a
// socket. Without it a single lost socket — a managed failover, an idle-socket
// reaper — would refuse every op on that store for the life of the process, and
// a `serve` is exactly the deployment that has no next run.
//
// # What *is* carried over, and why
//
// The session hygiene seven review rounds of the journal work paid for, because
// every one of those failures is a store's too:
//
//  * **`error` listeners before anything else.** Both drivers emit `error` on
//    the connection when the far end goes away with no command in flight, and an
//    `EventEmitter` that emits `error` with nothing listening ends the process —
//    a `serve` killed that way loses every in-flight execution's in-process
//    state to a server closing an idle socket.
//  * **Byte-wise key collation.** A `kv` key is the composition's own string and
//    two keys differing in case are two keys. MySQL's default collation is
//    case-**insensitive**, so `PRIMARY KEY` would fold them into one row and a
//    `get` would answer another key's value. See [`MYSQL_STORE_SCHEMA`] for the
//    second half of that, which is padding.
//  * **A payload column sized past 64 KiB.** MySQL's `TEXT` holds 64 KiB and the
//    arm sets `STRICT_TRANS_TABLES`, so a larger value is an error rather than a
//    truncation — and a `value:` this size is a write the other arms take.
//  * **`ANSI_QUOTES` and `autocommit` asserted at session open**, and a schema
//    named on the address rather than assumed.
//
// # What the columns hold
//
// A `kv` value is stored the way the local backend stores it: as the **JSON text
// this module produced**, in a `TEXT`/`LONGTEXT` column, parsed back with
// `JSON.parse`. Not a `JSONB`/`JSON` column, which is the tempting choice and
// the wrong one — those normalize what they are given (key order, whitespace,
// numeric spelling), so a `value:` would come back a different document than
// the one `value_schema` was checked against and two backends would disagree
// about what round-tripped. The column is text because the interface's payload
// is text.
//
// # Expiry and retention
//
// The module header's notes on expiry and on the idempotency ledger apply
// verbatim, with the clock moved: every expiry a dialled store reads or writes
// is computed by the **server's** clock (`StoreDialect.clock`), so two processes
// whose own clocks disagree still agree about which keys are alive. A write
// reads that clock once, inside its transaction, after its locks — so the
// instant it judges its key by and the instant it writes an expiry from are one
// — and a read is one statement that carries the clock in its own `WHERE`.
//
// And with one addition a dialled backend has and a file does not. A
// `scope: execution` store's rows really do outlive the process, so
// [`releaseExecution`] deletes that execution's partition when the run **ends**
// (and leaves it where the run only parked, which is the world a resume is
// promised). A delete that could not be made leaves rows under an execution id
// nothing will ever address again; it is not retried, because failing a
// completed run over cleanup would be worse, and `DELETE FROM store_entries
// WHERE scope_key LIKE 'execution/%'` is the operator's broom.

/** The providers this release dials for a `kv` store (PRD resolved q63). */
const DIALLED_KV: readonly BackendProvider[] = ["postgres", "mysql"];

/** A value a statement binds to a `?` — `null` for a key that never expires. */
type Bound = string | number | null;

/**
 * How one server spells the things the statements below differ in.
 *
 * Seven, and each is a spelling rather than a meaning: the ops are **one**
 * implementation of grammar 11.4 rather than two that have to be kept in step,
 * which is the whole reason this interface is narrow. An entry that decided
 * something would be a place two backends could come to mean different things,
 * and the conformance suite would be proving it rather than the servers. The
 * first three are PRD resolved q63's; the next three are q69's, because an
 * expiry needs the one thing a statement cannot carry portably — the server's
 * own clock — and a bounded `DELETE`, which the two servers spell differently;
 * the seventh is that bounded `DELETE` read on the ledger, whose authored rows
 * expire too (PRD resolved q73); and the eighth is the same `DELETE` read on
 * the per-key history, whose rows a `history_retention:` bounds (PRD resolved
 * q74).
 * The schema is not here for the same reason it is not a statement: a DDL is
 * how a backend spells the *shape* these statements need, which really is
 * per-backend, so each arm holds its own and runs it at open.
 */
interface StoreDialect {
  /**
   * The statements as written, with their `?` placeholders in this server's
   * spelling. Only Postgres needs one: it numbers its parameters.
   */
  readonly bind: (sql: string) => string;
  /**
   * The clause that makes an insert overwrite the row already there, column by
   * column.
   *
   * Everything after the statement's `VALUES (…)` list, which is more than the
   * conflict clause on one arm: MySQL's row alias (`AS excluded`) sits there
   * too, and a hook that could only return the conflict half would have made
   * the shared statement carry a dialect.
   */
  readonly overwrite: (target: string, columns: readonly string[]) => string;
  /**
   * …and the clause that makes it do **nothing** — the ledger's half, and the
   * one the whole dedupe rests on.
   */
  readonly ignore: (target: string, noop: string) => string;
  /**
   * The server's clock, as an expression answering an integer of epoch
   * milliseconds (PRD resolved q69).
   *
   * The server's rather than this process's, because a dialled store is
   * multi-writer by design: an expiry two processes computed from two clocks
   * would be one each believed and neither shared. Every statement that reads or
   * writes an expiry reads this, plus [`STORE_CLOCK_OFFSET`]'s shift.
   */
  readonly clock: string;
  /**
   * A `DELETE` of at most `limit` expired rows of **one** partition. Binds the
   * store, the partition and `now`, in that order.
   *
   * Both sweeps are this statement, and both run it inside a transaction that
   * holds the partition's sequence lock — a write's sweep under the lock its
   * write already took, and each batch of a store-wide sweep under the lock it
   * takes first, exactly as a write would ([`RemoteKv`]'s `#sweepStep`). So no
   * other write to the partition is in flight while it runs, and it cannot be
   * half of a deadlock with one: whoever holds the sequence row holds nothing
   * the other is waiting for.
   */
  readonly sweepPartition: (limit: number) => string;
  /**
   * A `DELETE` of at most `limit` expired rows of one store's idempotency
   * ledger — authored rows, the only kind that expire (PRD resolved q73).
   * Binds the store and `now`, in that order.
   *
   * Store-wide rather than one partition's, because an authored key is a
   * store's; and run only under the store's **authored lock**
   * ([`AUTHORED_LOCK`]), which every write carrying an authored key takes
   * after its partition's sequence row and before it reads the ledger, and
   * which each ledger batch of a store-wide sweep takes alone — so no other
   * statement that touches an authored row is in flight beside it, and it
   * cannot be half of a deadlock with one.
   */
  readonly sweepLedger: (limit: number) => string;
  /**
   * A `DELETE` of at most `limit` history rows of **one** partition at or
   * before a cut-off — those past the store's `history_retention:` (PRD
   * resolved q74). Binds the store, the partition and the cut-off, in that
   * order.
   *
   * [`sweepPartition`]'s statement read on the history, and run under the same
   * lock for the same reason: only ever inside a transaction that holds the
   * partition's sequence row — a write's, which inserts the partition's history
   * rows under it, or a store-wide sweep batch's, which takes it first — so no
   * other statement writing those rows is in flight beside it.
   */
  readonly sweepHistory: (limit: number) => string;
  /**
   * An idempotency key as this server's ledger column holds it — a spelling,
   * not a different key (PRD resolved q73).
   *
   * Every derived key is ASCII — an execution id and a path of identifiers
   * and decimals — but an **authored** one is a string the composition
   * evaluated, and a column sized for the derived ones may hold only ASCII
   * (MySQL's: `MYSQL_STORE_SCHEMA`). An arm whose column takes any string
   * spells every key as it is; one whose column does not spells it reversibly
   * and injectively into what the column holds, leaving every key that is
   * already ASCII and holds no `%` byte for byte as it was — so a derived row
   * an earlier release claimed is found by the same string, and a prefix of a
   * key spells as a prefix of its spelling, which is what the settlement
   * prune's range rests on ([`prefixEnd`]). Applied to every key a statement
   * binds; the trace and the site carry the key as written.
   */
  readonly ledgerKey: (idempotencyKey: string) => string;
}

/**
 * The partition name of a dialled store's **authored lock** — a row of
 * `store_sequences` that is no partition's (PRD resolved q73).
 *
 * A real partition is `global`, `session/<encoded key>` or `execution/<id>`
 * ([`partition`]), so `author` is never one. Its row holds no sequence anyone
 * reads; it is what serializes every statement that touches a store's authored
 * ledger rows, across connections and processes: a write under an authored key
 * locks it after its own partition's sequence row — always in that order, so
 * two writers never wait on each other in a circle — and the ledger half of a
 * store-wide sweep locks it alone. Two partitions of one store writing one
 * authored key therefore meet here rather than at the ledger's primary key,
 * and the second finds the first's committed row and is answered with it.
 */
const AUTHORED_LOCK = "author";

/** What one server's connection does, and the whole of it. */
interface StoreDriver {
  readonly dialect: StoreDialect;
  /** Every row a query answers. */
  all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]>;
  /** One statement, and how many rows it changed. */
  run(sql: string, parameters: readonly Bound[]): Promise<number>;
  /** Give the connection back. */
  close(): Promise<void>;
}

/** `ON CONFLICT … DO UPDATE`, which Postgres speaks. */
function standardOverwrite(target: string, columns: readonly string[]): string {
  return `ON CONFLICT ${target} DO UPDATE SET ${columns
    .map((column) => `${column} = excluded.${column}`)
    .join(", ")}`;
}

/** …and `ON CONFLICT … DO NOTHING`. */
function standardIgnore(target: string, _noop: string): string {
  return `ON CONFLICT ${target} DO NOTHING`;
}

/** `?`, unchanged — MySQL takes it as written. */
function positionalBind(sql: string): string {
  return sql;
}

/**
 * What a statement is refused with once a dialled store's connection has been
 * lost.
 *
 * **In the invariant half rather than once per arm**, and that placement is a
 * correctness requirement rather than tidiness. This module is assembled from
 * this half plus one arm per provider the target's stores bind
 * (`codegen::stores`), so a helper declared in *each* arm is two top-level
 * declarations of one name in the `src/stores.ts` of a target that binds one
 * store to `postgres` and another to `mysql`: `tsc` calls it TS2393 and Node
 * refuses the module outright at import, because a top-level `function` in an ES
 * module is lexically declared. The sentence was identical on both arms in any
 * case — only the provider differed — so the provider is a parameter and the
 * arms share the one declaration.
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
 *
 * `where` is the **variable name** the address was read from, never the address:
 * every message this module raises about a connection names the line an operator
 * can change, and a store URL is a credential.
 */
function storeConnectionLost(provider: BackendProvider, where: string, cause: unknown): Error {
  const detail = cause instanceof Error ? cause.message : String(cause);
  return new Error(
    `this project lost its connection to the \`${provider}\` store backend at \`\${${where}}\`: ${detail}. It is not redialled inside the op that failed — a write whose connection died is one nothing can say landed or did not — so this op fails and the node's own \`retry:\` decides what happens next; a retry carries the idempotency key its first attempt carried, which is what makes it apply once (grammar 9.4, PRD 5.8), and it runs over a connection dialled again rather than over this one`,
  );
}

/**
 * The arms this build emitted, by provider.
 *
 * Empty until an arm appended to this module registers itself, which `build`
 * does for exactly the providers this project's stores bind (`codegen::stores`).
 * A project built for a provider with no entry here is a compiler bug rather
 * than a deployment's mistake, and [`dialled`] says so by name.
 */
const STORE_BACKENDS: Partial<
  Record<BackendProvider, (url: string, where: string, lost: () => void) => Promise<StoreDriver>>
> = {};

/**
 * How long a dial of a store server may take before it fails: ten seconds, on
 * both arms — each hands it to its driver as that driver's own connect
 * deadline (`connectionTimeoutMillis`, `connectTimeout`), which ends the socket
 * rather than abandoning it.
 *
 * A bound, because this module dials servers nothing asked it to reach: a
 * settled execution's ledger prune visits every store the composition binds
 * ([`pruneLedger`]) and a process's start sweeps every one of them
 * ([`sweepBoundStores`]), so a server that accepts a connection and never
 * answers would otherwise hold a command's exit for as long as the socket
 * lived. Neither waits in front of an outcome; this is what bounds what
 * [`drainStores`] waits for. An op's own dial fails the same way, which is
 * the node's error under its `retry:` rather than a run that never ends.
 * Ten seconds is `mysql2`'s own default, so the MySQL arm's behaviour is
 * unchanged and the Postgres arm's is the same; it is long past any server
 * answering a handshake, a cold-starting one included.
 */
const STORE_DIAL_MS = 10_000;

/**
 * Every dialled connection this process holds, by the server it addresses.
 *
 * Keyed on the provider and the **variable name** rather than on the address:
 * two stores whose entries name one variable are one server, and a key that was
 * the resolved URL would put a credential in a map key for nothing. Two stores
 * naming two variables that happen to point at one server get two connections,
 * which costs a socket and changes no answer — there is no guard to contend
 * over.
 *
 * The map holds the **promise**, for [`DATABASES`]' reason: a `map` dispatches
 * its instances concurrently, and two of them reaching one store in the same
 * turn would otherwise both dial, leaving one connection behind unclosed.
 */
const DIALLED = new Map<string, Promise<RemoteKv>>();

/**
 * The connection one dialled store's ops run over, opened on first use.
 *
 * The address is read at the op rather than at import, like every other
 * credential this runtime spends: a composition with a store it never reaches
 * does not demand a variable, and a launch that is short one is already refused
 * by `./env.ts` with the site that asked for it.
 */
function connectionFor(store: StoreBinding): Promise<RemoteKv> {
  const variable = store.backend.urlEnv;
  if (variable === undefined) {
    throw new Error(
      `\`${store.address}\` is bound to the \`${store.backend.provider}\` backend (${store.backend.from}) and that entry names no \`url:\`: a provider that dials out has nowhere to go without one (grammar 4.3, 14.3). Give the entry a \`url: \${SOME_VARIABLE}\`, or bind a backend this target opens in process`,
    );
  }
  const address = process.env[variable];
  if (address === undefined || address === "") {
    throw new Error(
      `\`${variable}\` is unset, and it is where \`${store.address}\` is (${store.backend.from}): a store on a \`${store.backend.provider}\` backend is a connection this process opens before its first op (grammar 4.3, PRD resolved q15)`,
    );
  }
  const cacheKey = `${store.backend.provider}\u0000${variable}`;
  const held = DIALLED.get(cacheKey);
  if (held !== undefined) return held;
  const open = STORE_BACKENDS[store.backend.provider];
  if (open === undefined) {
    throw new Error(
      `\`${store.address}\` is bound to the \`${store.backend.provider}\` backend (${store.backend.from}) and this project carries no driver for one: \`build\` emits the arm each store's backend binds, so a project this happens to was not built from this composition's deploy layer`,
    );
  }
  // What an arm calls the moment its connection reports a fault.
  //
  // Neither driver reconnects on its own and an arm's fault is permanent once
  // set, so a faulted connection left here would refuse every op on this store —
  // in this execution and in every later one — until the process restarted. The
  // entry goes, the socket is given back, and the next op dials a fresh
  // connection; the idempotency key of grammar 9.4 is what makes the retry that
  // runs over it apply once, which is what the arms' own message promises. See
  // the module header on what is deliberately not copied from the journal.
  let dialling: Promise<RemoteKv> | undefined;
  let faulted = false;
  let dropped = false;
  const lost = (): void => {
    faulted = true;
    const held = dialling;
    // Nothing to drop yet: the fault arrived while the dial was still in flight,
    // and the line below this one answers it once there is. An arm cannot report
    // one before its first `await`, so this is belt and braces rather than a
    // path anybody has taken.
    if (held === undefined) return;
    // **Dropped once.** An arm reports a lost socket from two places — the
    // statement it died under, and the driver's own `error` event — and one loss
    // is both of them: the statement settles first and the event follows when
    // the socket ends. Without this the second call would queue a second close
    // on a connection already closing.
    if (dropped) return;
    dropped = true;
    if (DIALLED.get(cacheKey) === held) DIALLED.delete(cacheKey);
    // Queued behind whatever is still in flight on it, which is `RemoteKv`'s
    // own serialization; a close that fails is a socket the server reaps.
    void held.then(async (connection) => await connection.close()).catch(() => undefined);
  };
  // The **name** goes with the address, because every message an arm raises
  // about this connection has to name the line an operator can change — and the
  // address itself is a credential this module never prints.
  const provider = store.backend.provider;
  dialling = (async () => {
    const started = Date.now();
    try {
      return new RemoteKv(await open(address, variable, lost));
    } catch (error) {
      // A dial the driver gave up on at its deadline says so in the driver's
      // words — `Connection terminated`, `connect ETIMEDOUT` — which name
      // neither the deadline nor the store; this sentence names both.
      if (Date.now() - started < STORE_DIAL_MS) throw error;
      throw new Error(
        `the \`${provider}\` store backend at \`\${${variable}}\` did not answer a dial within ${STORE_DIAL_MS / 1000} seconds (${error instanceof Error ? error.message : String(error)})`,
        { cause: error },
      );
    }
  })();
  // Registered before the first `await` inside it, so a concurrent caller finds
  // this promise rather than dialling a second time. A failed dial is dropped
  // from the cache so the next op tries again instead of inheriting the failure
  // for the life of the process.
  DIALLED.set(cacheKey, dialling);
  const dialled = dialling;
  void dialling.catch(() => {
    if (DIALLED.get(cacheKey) === dialled) DIALLED.delete(cacheKey);
  });
  // See [`lost`]: a fault reported before there was a promise to drop.
  if (faulted) lost();
  return dialling;
}

/** One op against a dialled `kv` store (PRD resolved q63). */
async function dialled(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  idempotencyKey: string | undefined,
  via: Issuer,
  actor: HistoryActor,
): Promise<Applied> {
  const connection = await connectionFor(store);
  return await connection.apply(store, op, params, scopeKey, idempotencyKey, via, actor);
}

/**
 * One server's connection, with the whole `kv` catalogue over it.
 *
 * **Every op runs on one queue.** `#serial` is `SqlJournal`'s mechanism and is
 * here for the sharper version of its reason: a keyed write is a `BEGIN`, an
 * effect and a ledger insert, and a second op interleaving its statements into
 * that transaction on the same connection would join a transaction it knows
 * nothing about. A `map` dispatches its instances concurrently, so that is the
 * ordinary case rather than an edge one. Two *processes* are still concurrent —
 * that is the multi-writer posture — and what serializes their writes to one
 * key is the server's row locks (see [`#write`]).
 */
class RemoteKv {
  readonly #driver: StoreDriver;

  /** The tail of the queue every op chains onto. */
  #queue: Promise<unknown> = Promise.resolve();

  /**
   * The store-wide sweeps this connection has started, by store name — the
   * process-start half of PRD resolved q69's schedule, started once per store
   * per connection, by [`sweep`] or by the first op on the store — each the
   * promise of its end.
   *
   * Per connection rather than per process because a connection is what a
   * dialled store's "open" is: a process starting is its first connection, and
   * a connection redialled after a loss sweeps again, which costs a statement
   * that finds nothing. Each is held in [`SWEEPING`] too, which is what
   * [`drainStores`] waits on before [`releaseStores`] closes the connection,
   * so the connection keeps no wait of its own.
   */
  readonly #sweeps = new Map<string, Promise<void>>();

  /** Set once [`close`] is asked for, so a sweep queues no batch behind it. */
  #closing = false;

  constructor(driver: StoreDriver) {
    this.#driver = driver;
  }

  /**
   * Start one store's store-wide sweep over this connection unless it has one
   * already, and answer the promise of its end — what a process start asks of
   * every bound store ([`sweepBoundStores`]).
   */
  sweep(store: StoreBinding): Promise<void> {
    if (!this.#sweeps.has(store.name)) this.#startSweep(store);
    return this.#sweeps.get(store.name) ?? Promise.resolve();
  }

  /** Run one op against this server, dedupe included. */
  apply(
    store: StoreBinding,
    op: StoreOp,
    params: StoreParams,
    scopeKey: string,
    idempotencyKey: string | undefined,
    via: Issuer,
    actor: HistoryActor,
  ): Promise<Applied> {
    return this.#serial(() => this.#apply(store, op, params, scopeKey, idempotencyKey, via, actor));
  }

  /**
   * Delete one execution-scoped partition, when its run has ended — its
   * entries, its ledger, its history (PRD resolved q74) and its version
   * sequence, which nothing will address again.
   *
   * One transaction, under the partition's sequence lock taken first, as a
   * write and a sweep batch take it ([`#sweepBatch`]): another process's
   * store-wide sweep can reach this partition's expired rows at the same
   * moment, and two `DELETE`s reaching one row through two indexes are the
   * deadlock that lock exists to rule out. A partition no write ever reached
   * has no sequence row, and then no sweep has anything of it to take either.
   */
  forget(store: string, scopeKey: string): Promise<unknown> {
    return this.#serial(async () => {
      await this.#run("BEGIN", []);
      try {
        await this.#all(
          "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ? FOR UPDATE",
          [store, scopeKey],
        );
        for (const table of ["store_entries", "store_applied", "store_history", "store_sequences"]) {
          await this.#run(`DELETE FROM ${table} WHERE store = ? AND scope_key = ?`, [
            store,
            scopeKey,
          ]);
        }
        await this.#run("COMMIT", []);
      } catch (error) {
        await this.#run("ROLLBACK", []).catch(() => undefined);
        throw error;
      }
    });
  }

  /**
   * Delete the ledger rows a settled execution left in one store, whichever
   * partitions they are in: every idempotency key under `prefix`, which is
   * `<execution id>/` (PRD resolved q69, [`pruneLedger`]).
   *
   * A range on the ledger's primary key — `(store, idempotency_key)` on both
   * servers, with no partition in it — so the server seeks to the run of rows
   * under the prefix rather than reading anything else; one statement, because
   * the rows are this settled execution's alone and nothing else is writing
   * them.
   */
  prune(store: string, prefix: string): Promise<unknown> {
    // The prefix as the column spells it: a spelling keeps a prefix a prefix,
    // and `/` spells as itself, so the range is the same range ([`ledgerKey`]).
    const spelled = this.#driver.dialect.ledgerKey(prefix);
    return this.#serial(async () => {
      await this.#run(
        "DELETE FROM store_applied WHERE store = ? AND idempotency_key >= ? AND idempotency_key < ?",
        [store, spelled, prefixEnd(spelled)],
      );
    });
  }

  /**
   * One page of a partition's live entries with their expiry — what an
   * operator's `export` walks (PRD resolved q72, [`operatorPage`]).
   *
   * A read like [`#read`]'s: one statement, no transaction, the server's own
   * clock in its `WHERE` — so a page sees exactly the keys a `get` would — and
   * the first op on the store over this connection starts its store-wide
   * sweep behind it, as every op's does.
   */
  page(store: StoreBinding, scopeKey: string, params: OperatorPageParams): Promise<OperatorPage> {
    return this.#serial(async () => {
      if (!this.#sweeps.has(store.name)) this.#startSweep(store);
      const offset = clockOffset();
      const read = operatorPageRead(params, {
        from: "store_entries WHERE store = ? AND scope_key = ?",
        bound: [store.name, scopeKey],
        now: { sql: `(${this.#driver.dialect.clock}) + ?`, bound: [offset] },
      });
      return read.answer(await this.#all(read.query.sql, read.query.bound));
    });
  }

  /**
   * The instant this server says it is, shifted as every expiry it judges is
   * ([`STORE_CLOCK_OFFSET`]) — what an operator's `import` turns a line's
   * `expires_at` into a remaining `ttl` against (PRD resolved q72).
   */
  clock(): Promise<number> {
    return this.#serial(() => this.#now());
  }

  /** Give the connection back. */
  close(): Promise<void> {
    this.#closing = true;
    return this.#serial(() => this.#driver.close());
  }

  /** One step of the queue. See the class doc. */
  #serial<T>(work: () => Promise<T>): Promise<T> {
    const next = this.#queue.then(work, work);
    // The tail must not reject: it is what the *next* caller chains onto, and an
    // unhandled rejection on this runtime's Node floor ends the process. The
    // failure still reaches this caller through `next`.
    this.#queue = next.then(
      () => undefined,
      () => undefined,
    );
    return next;
  }

  #all(sql: string, parameters: readonly Bound[]): Promise<Record<string, unknown>[]> {
    return this.#driver.all(this.#driver.dialect.bind(sql), parameters);
  }

  #run(sql: string, parameters: readonly Bound[]): Promise<number> {
    return this.#driver.run(this.#driver.dialect.bind(sql), parameters);
  }

  /**
   * The op, and — on a write — the locks and the ledger of grammar 9.4 around
   * it.
   *
   * A **read** opens no transaction at all: one statement is atomic on both
   * servers, and a `BEGIN` around it would buy a round trip and nothing else.
   * Every **write** is [`#write`]'s, keyed or not: a tool's write carries no
   * idempotency key, and is still a read-modify-write of its key that the
   * other writers of its partition must not land inside.
   */
  async #apply(
    store: StoreBinding,
    op: StoreOp,
    params: StoreParams,
    scopeKey: string,
    idempotencyKey: string | undefined,
    via: Issuer,
    actor: HistoryActor,
  ): Promise<Applied> {
    // The first op on a store over this connection starts its store-wide
    // sweep — **behind** this op, at the tail of the queue it is running on,
    // never in front of it (see [`#startSweep`]).
    if (!this.#sweeps.has(store.name)) this.#startSweep(store);
    if (!writes(op)) {
      return { row: await this.#read(store, op, params, scopeKey), deduped: false };
    }
    const created = new Set<string>();
    for (;;) {
      const answered =
        op === "batch"
          ? await this.#batch(store, params, scopeKey, idempotencyKey, actor)
          : await this.#write(store, op, params, scopeKey, idempotencyKey, via, actor);
      if (!("missing" in answered)) return answered;
      // A row the write locks was not there: the partition's sequence row, or
      // the store's authored lock (PRD resolved q73). Each is created on the
      // first write that needs it, and a partition's is deleted only when an
      // execution-scoped partition's run ends, so a second miss of the same
      // row is a server that answers an insert it then does not show — not a
      // race worth waiting on.
      if (created.has(answered.missing)) {
        throw new Error(
          `\`${store.address}\` could not lock its version sequence for partition ${JSON.stringify(answered.missing)}: the row was created and then not found, so this server is not keeping what this store writes (PRD resolved q68)`,
        );
      }
      created.add(answered.missing);
      await this.#createSequence(store, answered.missing);
    }
  }

  /**
   * Take the locks a write holds before it reads anything — its partition's
   * sequence row and, for a write under an authored key, the store's authored
   * lock after it ([`AUTHORED_LOCK`]) — and answer the partition's last
   * version, or which of the two rows is not there to lock, having rolled the
   * transaction back.
   */
  async #lockWrite(
    store: StoreBinding,
    scopeKey: string,
    idempotencyKey: string | undefined,
  ): Promise<number | { readonly missing: string }> {
    const sequence = await this.#all(
      "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ? FOR UPDATE",
      [store.name, scopeKey],
    );
    if (sequence.length === 0) {
      await this.#run("ROLLBACK", []);
      return { missing: scopeKey };
    }
    if (authored(idempotencyKey)) {
      const lock = await this.#all(
        "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ? FOR UPDATE",
        [store.name, AUTHORED_LOCK],
      );
      if (lock.length === 0) {
        await this.#run("ROLLBACK", []);
        return { missing: AUTHORED_LOCK };
      }
    }
    return Number(sequence[0]?.["last"]);
  }

  /**
   * The ledger half of a keyed write, read under its locks: the answer a key
   * the ledger holds alive is owed, or `undefined` where it holds none —
   * having, for an authored key, removed the row that was there and expired,
   * and at most a hundred of the store's other expired ledger rows with it
   * (PRD resolved q73).
   */
  async #ledgerAnswer(
    store: StoreBinding,
    op: StoreOp,
    idempotencyKey: string,
    now: number,
  ): Promise<Applied | undefined> {
    const spelled = this.#driver.dialect.ledgerKey(idempotencyKey);
    const seen = await this.#all(
      `SELECT op, result FROM store_applied WHERE store = ? AND idempotency_key = ? AND ${liveClause("?")}`,
      [store.name, spelled, now],
    );
    const held = seen[0];
    if (held !== undefined && held["result"] !== undefined && held["result"] !== null) {
      return answeredFrom(store, op, idempotencyKey, held);
    }
    if (authored(idempotencyKey)) {
      await this.#run(this.#driver.dialect.sweepLedger(SWEEP_PER_WRITE), [store.name, now]);
      await this.#run(
        "DELETE FROM store_applied WHERE store = ? AND idempotency_key = ? AND expires_at <= ?",
        [store.name, spelled, now],
      );
    }
    return undefined;
  }

  /**
   * Create a partition's sequence row where it has none — **outside** any
   * transaction, so a write or a sweep rolled back never takes a row other
   * writers are queued behind with it — starting past every version the
   * partition already holds. A concurrent creator's row wins and this one
   * changes nothing.
   */
  async #createSequence(store: StoreBinding, scopeKey: string): Promise<void> {
    await this.#run(
      "INSERT INTO store_sequences (store, scope_key, last) VALUES (?, ?, " +
        '(SELECT COALESCE(MAX(version), 0) FROM store_entries WHERE store = ? AND scope_key = ?)) ' +
        this.#driver.dialect.ignore("(store, scope_key)", "last = last"),
      [store.name, scopeKey, store.name, scopeKey],
    );
  }

  /**
   * One write, in one transaction: lock, dedupe, read, decide, write, claim,
   * commit — or `undefined` where the partition has no sequence row yet.
   *
   * **The locks come first, and always in one order.** The partition's sequence
   * row (`store_sequences`) is locked before anything is read, so every write
   * to one partition — from this connection's queue, another connection, or
   * another process — runs one at a time from here to its `COMMIT`; and the
   * key's own entry is locked next, which is the row lock PRD resolved q68
   * names. Sequence first, entry second, on every write, is what makes the two
   * locks unable to deadlock: nobody holding an entry lock waits for a
   * sequence row. A row a lock finds is read at its latest committed version
   * on both servers — that is what `FOR UPDATE` reads — so the decision is made
   * against what the previous writer committed rather than against a snapshot
   * taken before it. The new row is created **outside** this transaction
   * ([`#apply`]), so a write rolled back never takes a sequence row other
   * writers are queued behind with it.
   *
   * **The dedupe is the server's, not this module's** (PRD resolved q63): a
   * repeated write carrying the key its first attempt carried applies **once**
   * even when the two attempts are two processes. Under the sequence lock the
   * second finds the first's committed ledger row and is answered with it,
   * whole — `applied` and `version` included (PRD resolved q68) — and the claim
   * is still an insert on the ledger's primary key whose conflict clause does
   * nothing, so a writer that reached this partition without the lock (an
   * earlier release, mid-upgrade) cannot make a second application land either:
   * a claim that conflicts rolls this attempt's effect back and answers with
   * the row that won. The effect and its ledger row are one transaction, so an
   * attempt that failed half way leaves neither.
   *
   * A partial update whose result the schema refuses throws out of
   * [`decideKvWrite`], and the `ROLLBACK` takes nothing with it but the locks.
   *
   * **A write under an authored key** (PRD resolved q73) takes one lock more,
   * the store's authored lock, after the sequence row and before anything is
   * read ([`#lockWrite`]): an authored key is a store's rather than a
   * partition's, so two writers of it may hold two partitions' sequence rows,
   * and this is where the second waits for the first's commit — and then finds
   * its ledger row. The ledger is read by the store's clock, so an authored row
   * whose `default_ttl:` has run out answers nothing ([`#ledgerAnswer`]).
   *
   * **On a `history: true` store the write's history row is inserted here
   * too** (PRD resolved q74), after the entry's own statement and before the
   * ledger claim, dated by the same server clock — inside the transaction, so
   * a claim that conflicts and rolls back, a decision that throws, and a
   * connection that dies before the `COMMIT` all take it with them. The rows
   * are inserted under the partition's sequence lock, which is what orders one
   * key's rows by its writes' commits.
   */
  async #write(
    store: StoreBinding,
    op: StoreOp,
    params: StoreParams,
    scopeKey: string,
    idempotencyKey: string | undefined,
    via: Issuer,
    actor: HistoryActor,
  ): Promise<Applied | { readonly missing: string }> {
    const key = required(params.key, "key", store);
    await this.#run("BEGIN", []);
    let row: Record<string, unknown>;
    try {
      const locked = await this.#lockWrite(store, scopeKey, idempotencyKey);
      if (typeof locked !== "number") return locked;
      const last = locked;
      // The server's clock, read once and after the locks: the instant this
      // write judges its key and its ledger row by, sweeps by and dates the
      // key and the row it leaves from (PRD resolved q69, q73).
      const now = await this.#now();
      if (idempotencyKey !== undefined) {
        const answered = await this.#ledgerAnswer(store, op, idempotencyKey, now);
        if (answered !== undefined) {
          await this.#run("ROLLBACK", []);
          return answered;
        }
      }
      // A write to a partition is the first half of the cleanup schedule: at
      // most a hundred of its expired rows leave storage — under the
      // partition's sequence lock, so no other write to it is in flight — before
      // the key is read, so that one of them may be this key.
      await this.#run(this.#driver.dialect.sweepPartition(SWEEP_PER_WRITE), [
        store.name,
        scopeKey,
        now,
      ]);
      await this.#sweepHistory(store, scopeKey, now, SWEEP_PER_WRITE);
      const found = await this.#all(
        'SELECT value, version, expires_at FROM store_entries WHERE store = ? AND scope_key = ? AND "key" = ? FOR UPDATE',
        [store.name, scopeKey, key],
      );
      const held = heldRow(found[0], now);
      const decision = decideKvWrite(store, op, params, held, via);
      switch (decision.effect) {
        case "none":
          row = { ...decision.answer, deduped: false };
          break;
        case "remove":
          await this.#run(
            'DELETE FROM store_entries WHERE store = ? AND scope_key = ? AND "key" = ?',
            [store.name, scopeKey, key],
          );
          await this.#history(store, scopeKey, { op, key, before: held, after: undefined }, actor, now);
          row = { ...decision.answer, deduped: false };
          break;
        case "touch":
          await this.#run(
            'UPDATE store_entries SET expires_at = ? WHERE store = ? AND scope_key = ? AND "key" = ?',
            [expiryOf(store, params, now), store.name, scopeKey, key],
          );
          await this.#history(store, scopeKey, { op, key, before: held, after: held }, actor, now);
          row = { ...decision.answer, deduped: false };
          break;
        case "write": {
          const version = last + 1;
          await this.#run(
            "UPDATE store_sequences SET last = ? WHERE store = ? AND scope_key = ?",
            [version, store.name, scopeKey],
          );
          await this.#run(
            'INSERT INTO store_entries (store, scope_key, "key", value, version, expires_at) VALUES (?, ?, ?, ?, ?, ?) ' +
              this.#driver.dialect.overwrite('(store, scope_key, "key")', [
                "value",
                "version",
                "expires_at",
              ]),
            [
              store.name,
              scopeKey,
              key,
              JSON.stringify(decision.value),
              version,
              expiryOf(store, params, now),
            ],
          );
          await this.#history(
            store,
            scopeKey,
            { op, key, before: held, after: { value: decision.value, version } },
            actor,
            now,
          );
          row = { ...decision.answer(version), deduped: false };
          break;
        }
      }
      if (idempotencyKey !== undefined) {
        const claimed = await this.#claim(store, scopeKey, idempotencyKey, op, row, now);
        if (claimed === 0) {
          await this.#run("ROLLBACK", []);
          return await this.#claimedElsewhere(store, op, idempotencyKey);
        }
      }
    } catch (error) {
      await this.#run("ROLLBACK", []).catch(() => undefined);
      throw error;
    }
    await this.#run("COMMIT", []);
    return { row, deduped: false };
  }

  /**
   * One `batch`, in one transaction: [`#write`]'s shape with its entries
   * inside it — or `undefined` where the partition has no sequence row yet
   * (PRD resolved q71).
   *
   * The partition's sequence row is locked first, the ledger asked once (a
   * retry is answered with the stored row, whole), the clock read once and the
   * partition swept once, all as one write does; then a savepoint, and each
   * entry in order — its key's row locked `FOR UPDATE` and read, decided by
   * [`batchStep`], and written at once, so a later entry naming the same key
   * reads what an earlier one left. Sequence first and entries after, on every
   * batch and every write, is the order that keeps two of them from waiting on
   * each other in a circle, however their keys overlap. The versions the `set`
   * entries take are the sequence's next ones in entry order, moved on in one
   * statement once every entry has landed.
   *
   * An entry whose condition does not hold rolls the entries back to the
   * savepoint and the batch answers its decline, which the ledger row then
   * holds; a lost connection or a server that fails a statement throws, and
   * the `ROLLBACK` takes every entry, the versions they would have taken and
   * the sweep with it. A connection that dies between two entries — the
   * process killed, the session ended by the server — is a transaction that
   * never reached its `COMMIT`, which the server rolls back the same way.
   *
   * On a `history: true` store each entry that lands records its own history
   * row as it lands, under the savepoint (PRD resolved q74) — so a declined
   * batch's rollback to the savepoint takes the rows with the entries, and a
   * batch that never commits leaves none.
   */
  async #batch(
    store: StoreBinding,
    params: StoreParams,
    scopeKey: string,
    idempotencyKey: string | undefined,
    actor: HistoryActor,
  ): Promise<Applied | { readonly missing: string }> {
    const entries = batchEntries(store, params);
    await this.#run("BEGIN", []);
    let row: Record<string, unknown>;
    try {
      // The locks a single write takes, in its order — an authored key's
      // included (PRD resolved q73) — and its ledger read by its clock.
      const locked = await this.#lockWrite(store, scopeKey, idempotencyKey);
      if (typeof locked !== "number") return locked;
      const first = locked;
      const now = await this.#now();
      if (idempotencyKey !== undefined) {
        const answered = await this.#ledgerAnswer(store, "batch", idempotencyKey, now);
        if (answered !== undefined) {
          await this.#run("ROLLBACK", []);
          return answered;
        }
      }
      await this.#run(this.#driver.dialect.sweepPartition(SWEEP_PER_WRITE), [
        store.name,
        scopeKey,
        now,
      ]);
      await this.#sweepHistory(store, scopeKey, now, SWEEP_PER_WRITE);
      await this.#run("SAVEPOINT batch_entries", []);
      let last = first;
      let failed: number | undefined;
      const results: Record<string, unknown>[] = [];
      for (const [index, entry] of entries.entries()) {
        const found = await this.#all(
          'SELECT value, version, expires_at FROM store_entries WHERE store = ? AND scope_key = ? AND "key" = ? FOR UPDATE',
          [store.name, scopeKey, entry.key],
        );
        const held = heldRow(found[0], now);
        const step = batchStep(store, entry, held);
        if (step.effect === "none") {
          failed = index;
          break;
        }
        if (step.effect === "remove") {
          await this.#run(
            'DELETE FROM store_entries WHERE store = ? AND scope_key = ? AND "key" = ?',
            [store.name, scopeKey, entry.key],
          );
          await this.#history(
            store,
            scopeKey,
            { op: entry.op, key: entry.key, before: held, after: undefined },
            actor,
            now,
          );
          results.push({ key: entry.key });
          continue;
        }
        last += 1;
        await this.#run(
          'INSERT INTO store_entries (store, scope_key, "key", value, version, expires_at) VALUES (?, ?, ?, ?, ?, ?) ' +
            this.#driver.dialect.overwrite('(store, scope_key, "key")', [
              "value",
              "version",
              "expires_at",
            ]),
          [
            store.name,
            scopeKey,
            entry.key,
            JSON.stringify(step.value),
            last,
            expiryOf(store, entry, now),
          ],
        );
        await this.#history(
          store,
          scopeKey,
          { op: entry.op, key: entry.key, before: held, after: { value: step.value, version: last } },
          actor,
          now,
        );
        results.push({ key: entry.key, version: last });
      }
      if (failed !== undefined) {
        await this.#run("ROLLBACK TO SAVEPOINT batch_entries", []);
        row = declinedBatch(failed);
      } else {
        if (last !== first) {
          await this.#run("UPDATE store_sequences SET last = ? WHERE store = ? AND scope_key = ?", [
            last,
            store.name,
            scopeKey,
          ]);
        }
        row = { applied: true, deduped: false, results };
      }
      await this.#run("RELEASE SAVEPOINT batch_entries", []);
      if (idempotencyKey !== undefined) {
        const claimed = await this.#claim(store, scopeKey, idempotencyKey, "batch", row, now);
        if (claimed === 0) {
          await this.#run("ROLLBACK", []);
          return await this.#claimedElsewhere(store, "batch", idempotencyKey);
        }
      }
    } catch (error) {
      await this.#run("ROLLBACK", []).catch(() => undefined);
      throw error;
    }
    await this.#run("COMMIT", []);
    return { row, deduped: false };
  }

  /**
   * Record one write's history row, inside the transaction the write holds
   * open — or nothing, on a store that keeps no history ([`historyRow`], PRD
   * resolved q74). `seq` is the server's (`BIGSERIAL`, `AUTO_INCREMENT`),
   * taken here, under the partition's sequence lock the write holds.
   */
  async #history(
    store: StoreBinding,
    scopeKey: string,
    write: HistoryWrite,
    actor: HistoryActor,
    now: number,
  ): Promise<void> {
    const row = historyRow(store, write, actor, now);
    if (row === undefined) return;
    await this.#run(
      `INSERT INTO store_history (store, scope_key, ${row.columns}) VALUES (?, ?, ${row.placeholders})`,
      [store.name, scopeKey, ...row.bound],
    );
  }

  /**
   * Remove at most `limit` of one partition's history rows past the store's
   * `history_retention:` by `now` — on a store that declares one (PRD resolved
   * q74). Called only under the partition's sequence lock: a write's half of
   * the schedule, beside the entries' own sweep.
   */
  async #sweepHistory(store: StoreBinding, scopeKey: string, now: number, limit: number): Promise<void> {
    const cutoff = historyCutoff(store, now);
    if (cutoff === undefined) return;
    await this.#run(this.#driver.dialect.sweepHistory(limit), [store.name, scopeKey, cutoff]);
  }

  /**
   * Claim a keyed write's ledger row — the whole answer, and the instant an
   * authored row stops answering (PRD resolved q73) — and answer how many rows
   * the claim inserted: `0` where another attempt's row is already there.
   */
  async #claim(
    store: StoreBinding,
    scopeKey: string,
    idempotencyKey: string,
    op: StoreOp,
    row: Record<string, unknown>,
    now: number,
  ): Promise<number> {
    return await this.#run(
      "INSERT INTO store_applied (store, scope_key, idempotency_key, op, result, expires_at) " +
        `VALUES (?, ?, ?, ?, ?, ?) ${this.#driver.dialect.ignore(
          "(store, idempotency_key)",
          "idempotency_key = idempotency_key",
        )}`,
      [
        store.name,
        scopeKey,
        this.#driver.dialect.ledgerKey(idempotencyKey),
        op,
        JSON.stringify(row),
        ledgerExpiry(store, idempotencyKey, now),
      ],
    );
  }

  /**
   * The answer a claim that conflicted is owed: the row that won it.
   *
   * Read after the rollback rather than inside the transaction, because the row
   * it is looking for is another transaction's committed one.
   */
  async #claimedElsewhere(store: StoreBinding, op: StoreOp, idempotencyKey: string): Promise<Applied> {
    const held = await this.#all(
      "SELECT op, result FROM store_applied WHERE store = ? AND idempotency_key = ?",
      [store.name, this.#driver.dialect.ledgerKey(idempotencyKey)],
    );
    const result = held[0]?.["result"];
    if (held[0] === undefined || result === undefined || result === null) {
      throw new Error(
        `\`${store.address}\` refused a write under an idempotency key its ledger holds and then could not read what that key answered; the ledger row and the effect it dedupes are written in one transaction, so this server is not the one that took the write (grammar 9.4, PRD 5.8)`,
      );
    }
    return answeredFrom(store, op, idempotencyKey, held[0]);
  }

  /** The `kv` reads of grammar 11.4's catalogue, as this server answers them. */
  async #read(
    store: StoreBinding,
    op: StoreOp,
    params: StoreParams,
    scopeKey: string,
  ): Promise<Record<string, unknown>> {
    // A read is one statement, so it carries the server's clock in its own
    // `WHERE` rather than asking for it first: an expired key is not found,
    // listed, counted or read by a `get_many`, by the clock every writer of
    // this store dated it with (PRD resolved q69).
    const offset = clockOffset();
    const clock: Query = { sql: `(${this.#driver.dialect.clock}) + ?`, bound: [offset] };
    const alive = liveClause(clock.sql);
    switch (op) {
      case "get": {
        const key = required(params.key, "key", store);
        const found = await this.#all(
          'SELECT value, version FROM store_entries WHERE store = ? AND scope_key = ? AND "key" = ? ' +
            `AND ${alive}`,
          [store.name, scopeKey, key, offset],
        );
        const value = found[0]?.["value"];
        // A miss answers `found: false` and **no** `value` or `version` at
        // all, which is the same row the local backend answers (Decision D110,
        // PRD resolved q68).
        return value === undefined || value === null
          ? { found: false }
          : {
              value: JSON.parse(String(value)) as unknown,
              found: true,
              version: Number(found[0]?.["version"]),
            };
      }
      // The reads of PRD resolved q70, as the statements every arm shares
      // ([`partitionRead`]), over this server's placeholder dialect — and its
      // clock, so a page, a `count` and a `get_many` judge a key's expiry by
      // the same instant a `get` does.
      case "list":
      case "count":
      case "get_many": {
        const read = partitionRead(store, op, params, {
          from: "store_entries WHERE store = ? AND scope_key = ?",
          bound: [store.name, scopeKey],
          now: clock,
        });
        return read.answer(read.query === undefined ? [] : await this.#all(read.query.sql, read.query.bound));
      }
      // One page of one key's history (PRD resolved q74), as the statement
      // every arm shares ([`historyRead`]), judging retention by the clock
      // the rows were dated with.
      case "history": {
        const read = historyRead(store, params, {
          from: "store_history WHERE store = ? AND scope_key = ?",
          bound: [store.name, scopeKey],
          now: clock,
        });
        return read.answer(await this.#all(read.query.sql, read.query.bound));
      }
      default:
        throw new Error(`\`${store.address}\` is \`kind: ${store.kind}\` and takes no \`${op}\``);
    }
  }

  /**
   * The server's clock, shifted by [`STORE_CLOCK_OFFSET`], in epoch
   * milliseconds — by the shift as it reads now, or as a caller read it
   * earlier and hands in.
   */
  async #now(offset: number = clockOffset()): Promise<number> {
    const answered = await this.#all(`SELECT (${this.#driver.dialect.clock}) AS now`, []);
    const now = Number(answered[0]?.["now"]);
    if (!Number.isFinite(now)) {
      throw new Error(
        `this store backend answered ${JSON.stringify(answered[0]?.["now"] ?? null)} for its own clock, which is not a number of milliseconds: a \`kv\` key's expiry is computed by the server's clock (PRD resolved q69)`,
      );
    }
    return now + offset;
  }

  /**
   * Start removing every row of one store that had expired by the instant the
   * op starting it was made — the sweep a connection's first op on a store
   * starts, which is the second half of PRD resolved q69's schedule.
   *
   * **Behind the ops, never in front of them.** One batch ([`#sweepStep`]) is
   * one step of this connection's queue, and the next batch is queued only when
   * the one before it has finished — at the tail, behind every op that arrived
   * meanwhile. So the op that started the sweep runs first, and an op arriving
   * while it is under way waits for at most the one batch in progress, however
   * large the backlog is. [`drainStores`] waits for the rest, so a command
   * that reached the store has removed its backlog by the time it exits; a
   * connection that is closed or lost first stops between two batches.
   *
   * It ends when no row of the store had expired by that instant, or when the
   * same partition has twice in a row yielded nothing to a batch — rows held
   * by a writer that does not take the sequence lock, which only an earlier
   * release's writer mid-upgrade is — and the next write to that partition
   * sweeps those instead. A write only ever moves an expiry past the instant it
   * was made at, so the rows the sweep is after only ever become fewer.
   *
   * **Never an op's failure.** Every read and condition already treats an
   * expired row as absent, so a batch the server refused is said on stderr and
   * the sweep stops; a connection that is really gone fails the next op's own
   * statement, with the sentence that names it.
   */
  #startSweep(store: StoreBinding): void {
    // The shift as the op starting this reads it — a test moves the clock for
    // the length of one op — so the sweep judges "expired" by that op's instant.
    const written = process.env[STORE_CLOCK_OFFSET];
    let finish = (): void => undefined;
    const finished = new Promise<void>((resolve) => {
      finish = resolve;
    });
    this.#sweeps.set(store.name, finished);
    holdSweep(finished);
    let now: number | undefined;
    let stalled: string | undefined;
    const step = (): void => {
      this.#serial(async () => {
        if (this.#closing) return false;
        now ??= await this.#now(clockOffset(written));
        const swept = await this.#sweepStep(store, now);
        if (swept === undefined) return false;
        if (swept.removed > 0) {
          stalled = undefined;
          return true;
        }
        if (stalled === swept.scopeKey) return false;
        stalled = swept.scopeKey;
        return true;
      }).then(
        (more) => (more && !this.#closing ? step() : finish()),
        (error: unknown) => {
          process.stderr.write(
            `warning: \`${store.address}\` could not finish removing its expired keys (${error instanceof Error ? error.message : String(error)}); they stay invisible to every read and condition, and a write to each one's partition removes them (PRD resolved q69)\n`,
          );
          finish();
        },
      );
    };
    step();
  }

  /**
   * One batch of a store-wide sweep: the partition holding the store's oldest
   * row expired by `now`, and at most [`SWEEP_PER_BATCH`] of that partition's
   * expired rows — or `undefined` where no row had expired by `now`.
   *
   * **Under the partition's sequence lock, taken first, as a write takes it**
   * ([`#write`]). That is what makes a sweep unable to deadlock a write on
   * either server. A store-wide `DELETE` reaching its rows through the
   * store-wide expiry index would lock them in an order no write shares — on
   * MySQL it holds each row's clustered record and then needs the entry it has
   * in the partition's own expiry index, which a write's sweep of that
   * partition locks the other way round — and the server would break the cycle
   * by failing one of them, the write as often as not. Here, whichever of a
   * sweep batch and a write holds the sequence row holds nothing the other is
   * waiting for, so one simply waits for the other's commit. The partition is
   * found by a read that locks nothing, through the store-wide expiry index, so
   * it reads one expired row and no live one.
   */
  async #sweepStep(
    store: StoreBinding,
    now: number,
  ): Promise<{ readonly scopeKey: string; readonly removed: number } | undefined> {
    const oldest = await this.#all(
      "SELECT scope_key FROM store_entries WHERE store = ? AND expires_at <= ? ORDER BY expires_at LIMIT 1",
      [store.name, now],
    );
    const found = oldest[0]?.["scope_key"];
    if (found !== undefined && found !== null) {
      const scopeKey = String(found);
      return { scopeKey, removed: await this.#sweepBatch(store, scopeKey, now) };
    }
    // The entries are done: then the ledger's expired rows, authored ones all
    // (PRD resolved q73), found by a read that locks nothing and removed a
    // batch at a time under the store's authored lock.
    const ledger = await this.#all(
      "SELECT idempotency_key FROM store_applied WHERE store = ? AND expires_at <= ? ORDER BY expires_at LIMIT 1",
      [store.name, now],
    );
    if (ledger.length > 0) {
      return { scopeKey: AUTHORED_LOCK, removed: await this.#sweepLedgerBatch(store, now) };
    }
    // The ledger is done too: then the history rows past the store's
    // `history_retention:` (PRD resolved q74), on a store that declares one —
    // the partition holding the oldest found by a read that locks nothing,
    // through `store_history_at`, and a batch of its rows removed under that
    // partition's sequence lock, exactly as an entries batch is.
    const cutoff = historyCutoff(store, now);
    if (cutoff === undefined) return undefined;
    const history = await this.#all(
      "SELECT scope_key FROM store_history WHERE store = ? AND at <= ? ORDER BY at LIMIT 1",
      [store.name, cutoff],
    );
    const partition = history[0]?.["scope_key"];
    if (partition === undefined || partition === null) return undefined;
    const scopeKey = String(partition);
    return {
      // Labelled apart from the same partition's entries batch, so a stalled
      // history batch is told from a stalled entries one: no real partition
      // begins `history:` ([`partition`]).
      scopeKey: `history:${scopeKey}`,
      removed: await this.#sweepBatch(
        store,
        scopeKey,
        cutoff,
        this.#driver.dialect.sweepHistory(SWEEP_PER_BATCH),
      ),
    };
  }

  /**
   * One batch of the ledger half of a store-wide sweep, in its own transaction
   * under the store's authored lock, taken alone ([`AUTHORED_LOCK`]): at most
   * [`SWEEP_PER_BATCH`] ledger rows expired by `now` (PRD resolved q73).
   *
   * An expired ledger row is an authored one, written by a write that held
   * the lock, so the lock row is there; a batch that finds none removes
   * nothing, and the step that asked ends the sweep the way a stalled
   * partition does.
   */
  async #sweepLedgerBatch(store: StoreBinding, now: number): Promise<number> {
    await this.#run("BEGIN", []);
    try {
      const lock = await this.#all(
        "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ? FOR UPDATE",
        [store.name, AUTHORED_LOCK],
      );
      if (lock.length === 0) {
        await this.#run("ROLLBACK", []);
        return 0;
      }
      const removed = await this.#run(this.#driver.dialect.sweepLedger(SWEEP_PER_BATCH), [
        store.name,
        now,
      ]);
      await this.#run("COMMIT", []);
      return removed;
    } catch (error) {
      await this.#run("ROLLBACK", []).catch(() => undefined);
      throw error;
    }
  }

  /**
   * One partition's batch of [`#sweepStep`], in its own transaction under the
   * partition's sequence lock.
   *
   * A partition holding an expired row always has a sequence row: only a write
   * of this release sets an expiry, and it does so holding that row. The one
   * way to find none is a race with [`forget`], which takes the same lock and
   * deletes the entries in the same transaction as the row — so a batch that
   * finds none removes nothing, and the next step asks again.
   *
   * `statement` is the bounded `DELETE` the batch makes — the entries' own by
   * default, or the history's past a retention cut-off (PRD resolved q74),
   * which binds the cut-off where the entries' binds `now`: a history row is
   * only ever written under the same lock, so the same argument holds of it.
   */
  async #sweepBatch(
    store: StoreBinding,
    scopeKey: string,
    now: number,
    statement: string = this.#driver.dialect.sweepPartition(SWEEP_PER_BATCH),
  ): Promise<number> {
    await this.#run("BEGIN", []);
    try {
      const sequence = await this.#all(
        "SELECT last FROM store_sequences WHERE store = ? AND scope_key = ? FOR UPDATE",
        [store.name, scopeKey],
      );
      if (sequence.length === 0) {
        await this.#run("ROLLBACK", []);
        return 0;
      }
      const removed = await this.#run(statement, [store.name, scopeKey, now]);
      await this.#run("COMMIT", []);
      return removed;
    } catch (error) {
      await this.#run("ROLLBACK", []).catch(() => undefined);
      throw error;
    }
  }
}

/**
 * Finish what is still running over the stores, and close nothing: every
 * settled execution's ledger prune ([`pruneLedger`]), and every store-wide
 * sweep this process started ([`SWEEPING`]) — on SQLite and over each
 * connection alike (PRD resolved q69).
 *
 * The wait every command makes on its way out: [`releaseStores`]'s first step
 * for a `run` or a `resume`, and the whole of what `./serve.ts` does with its
 * stores when a `serve` is stopped. A stop is the only point a `serve` meets
 * where nothing comes after: a prune it leaves unfinished, or one still
 * waiting on a delivery, is one no process will ever run, since a settled
 * execution is never recovered (PRD resolved q65) — and a `serve` stopped on
 * every deploy would leave every such execution's rows, so the ledger would
 * grow with history after all. That wait is here, on the way out, rather than
 * in front of an outcome or a first op, where it would stall whatever the
 * command had come to do; a dial either one needs is bounded by
 * [`STORE_DIAL_MS`], so a server that does not answer holds the exit for that
 * long and no longer.
 *
 * **Nothing is closed**, which is what a `serve` needs of it: its executions
 * still in flight keep running while it stops — it exits with them open, and
 * the next start recovers each (`./serve.ts`) — and a store op one of them
 * made over a connection closed under it would fail its node, which could
 * settle an execution `failed` that the stop was meant to leave open.
 *
 * **A prune still waiting on detached deliveries starts now** ([`DEFERRED`]).
 * Neither `agent-compose run` nor a stopped `serve` waits for an `agent.*` or
 * `tool.*` delivery an execution started — the exit cuts it off — so a prune
 * chained behind one would never run, and every such execution would leave its
 * rows for ever. Once this process is gone no delivery of a settled execution
 * retries anywhere (PRD resolved q65), so the rows answer nothing; what is left
 * is the delivery still running *here* until the exit, and a keyed write it
 * makes under that execution from now on is refused rather than made without
 * its ledger row ([`CUT_OFF`]).
 *
 * Re-read until nothing is left, because a prune can start while another
 * finishes — and, in a `serve`, because an execution still in flight can
 * settle while this waits.
 */
export async function drainStores(): Promise<void> {
  for (;;) {
    for (const [id, deferred] of DEFERRED) {
      DEFERRED.delete(id);
      CUT_OFF.add(id);
      startPrune(deferred);
    }
    const running = [...PRUNING, ...SWEEPING];
    if (running.length === 0) return;
    await Promise.all(running.map((held) => held.catch(() => undefined)));
  }
}

/**
 * Close every dialled connection this process opened.
 *
 * Called by `./cli.ts` on the way out of a `run` or a `resume`, beside
 * `releaseJournal()` and for that call's reason: a socket left open is a process
 * that does not exit. A `serve` holds them for as long as it serves, and its
 * stop makes the same wait without the close ([`drainStores`]).
 *
 * Idempotent, and never the caller's failure: a connection that could not be
 * closed is one the server will reap, and the command that asked has already
 * done its work.
 *
 * **What is still running over the stores goes first** ([`drainStores`]), so a
 * command's settled executions leave no ledger rows and every store it swept
 * has lost what had expired there by the time it exits (PRD resolved q69).
 */
export async function releaseStores(): Promise<void> {
  await drainStores();
  const held = [...DIALLED.values()];
  DIALLED.clear();
  await Promise.all(
    held.map(async (dialling) => {
      try {
        await (await dialling).close();
      } catch {
        // A connection that could not be closed is one the server reaps.
      }
    }),
  );
}

// ---------------------------------------------------------------------------
// An operator's reads and writes (PRD resolved q72)
// ---------------------------------------------------------------------------
//
// `agent-compose store` reads and writes one `kv` store with no flow written:
// the compiler builds this project and launches its command line's `store`
// verb (`./cli.ts`), which reaches the store through the three entry points
// below. They share everything an op **is** with [`runStoreOp`] — the
// partition a scope and an identity derive ([`partition`]), the arm the target
// bound and its connection, the one liveness filter, the conditions, the
// version sequence, the expiry a write leaves, the sweeps — because they call
// the same [`perform`] and the same reads. So what an operator reads is what a
// flow in that partition reads, and a key an operator writes is one the next
// flow finds.
//
// What they bypass is exactly the **effect** machinery, and nothing else. An
// operator's op belongs to no execution, so it is not journaled (no
// `journaled` seam, no row in the journal), files no trace record, carries no
// idempotency key — so a write is never deduped and leaves no ledger row — owns
// no partition for [`releaseExecution`] to release ([`remember`]), and is
// asked no replay question ([`inProcessState`]). A write is therefore recorded
// in one place only: on a store that declares `history: true`, its history row
// (PRD resolved q74), written in its own transaction like every write's, with
// the actor `{ kind: "cli" }` — no execution made it, so the row names none.
// On a store that keeps no history it is recorded nowhere. An operator also
// reads that history, one key at a time, with the `history` op (`store log`).
//
// Two kinds of store cannot be reached this way, and [`operatorRefusal`] says
// so by name rather than answering out of the wrong place: a `vector` or
// `blob` store, since the command is a `kv` surface; and a store whose rows
// live only in the process that opened them ([`inProcessOnly`]) — a
// `scope: execution` store on a backend this process opens itself, or one a
// target bound to `provider: memory` — since the command is another process.

/** One store an operator addresses, and which of its partitions (PRD resolved q72). */
export interface OperatorTarget {
  readonly store: StoreBinding;
  /** `--session <key>`: the session identity a `scope: session` store partitions by. */
  readonly session?: string;
  /** `--execution <id>`: the execution a `scope: execution` store's partition belongs to. */
  readonly execution?: string;
}

/**
 * The ops an operator issues one at a time: grammar 11.4's `kv` rows the
 * command line mirrors — `history` is `store log`'s (PRD resolved q74).
 */
export type OperatorOp = "get" | "set" | "delete" | "list" | "count" | "history";

/** One page of an operator's walk of a partition — what `export` reads. */
export interface OperatorPageParams {
  readonly prefix?: string;
  /** The exclusive cursor, as a `list`'s `after:` (PRD resolved q70). */
  readonly after?: string;
  readonly limit: number;
}

/** One live entry, with its expiry, as an export writes it. */
export interface OperatorEntry {
  readonly key: string;
  readonly value: unknown;
  readonly version: number;
  /** The instant it expires, in epoch milliseconds by the store's clock — `null` where it does not. */
  readonly expiresAt: number | null;
}

/** A page of [`OperatorEntry`]s, and the cursor of the next where more remain. */
export interface OperatorPage {
  readonly entries: readonly OperatorEntry[];
  readonly next?: string;
}

/**
 * Every store the composition binds, in the order `./graph.ts` declared them —
 * what the command line resolves an address against, and lists when one does
 * not resolve.
 */
export function boundStores(): readonly StoreBinding[] {
  return [...BOUND.values()];
}

/**
 * Why an operator cannot address `store` from the command line, or
 * `undefined` where one can (see the section header).
 */
export function operatorRefusal(store: StoreBinding): string | undefined {
  if (store.kind !== "kv") {
    return `\`${store.address}\` is a \`${store.kind}\` store, and the store command reads and writes \`kv\` stores: a \`${store.kind}\` store is reached through its own ops, from a flow (grammar 11.4, PRD resolved q72)`;
  }
  if (!inProcessOnly(store)) return undefined;
  const lifetime =
    store.backend.provider === "memory"
      ? `bound to \`provider: memory\` (${store.backend.from})`
      : `\`scope: execution\` on the \`${store.backend.provider}\` backend (${store.backend.from})`;
  return `\`${store.address}\` is ${lifetime}, so its rows live only in the memory of the process that opened them — each execution's own — and this command is another process, with nothing there to read or write. A target that binds the store to \`postgres\` or \`mysql\` keeps those rows on a server, where this command reaches them (grammar 14.3, PRD resolved q63)`;
}

/** The identity and the partition an operator's op addresses, refused where it cannot be reached. */
function operatorScope(target: OperatorTarget): {
  readonly identity: runtime.ExecutionIdentity;
  readonly scopeKey: string;
} {
  const { store } = target;
  const refusal = operatorRefusal(store);
  if (refusal !== undefined) throw new Error(refusal);
  supported(store);
  if (store.scope === "execution" && (target.execution ?? "") === "") {
    throw new Error(
      `\`${store.address}\` is \`scope: execution\`, so an operator names the execution whose partition it addresses (PRD resolved q72)`,
    );
  }
  // The identity a flow instance carries, with only the half this store's scope
  // reads filled in — so [`partition`] derives the partition the runtime would.
  const identity: runtime.ExecutionIdentity = {
    id: target.execution ?? "",
    session_key: target.session ?? "",
  };
  return { identity, scopeKey: partition(store, identity) };
}

/**
 * One op an operator issues against one partition, answered with the op's own
 * row — exactly the row a store-op node's output carries (grammar 11.4).
 *
 * [`perform`] with **no idempotency key**, and nothing around it: no journal,
 * no trace record, no execution's ownership (see the section header). A write
 * runs the same read-modify-write a node's does, under the same lock, so its
 * condition is decided against what the key holds as it commits and it takes
 * the partition's next version — and a `set` is held to nothing here: the
 * command line holds its value to the store's `value_schema` before it gets
 * this far, as a node's `value:` is held to it at `validate`. On a store that
 * keeps history the write records its row as actor `cli` (PRD resolved q74).
 */
export async function runOperatorOp(
  target: OperatorTarget,
  op: OperatorOp,
  params: StoreParams,
): Promise<Record<string, unknown>> {
  const { identity, scopeKey } = operatorScope(target);
  const answer = await perform(
    target.store,
    op,
    params,
    scopeKey,
    identity,
    undefined,
    "operator",
    // Nothing aborts an operator's op: the signal is a `vector` op's, for the
    // embedding a `kv` op never computes.
    new AbortController().signal,
    { kind: "cli" },
  );
  return answer.row;
}

/**
 * One page of a partition's live entries, with each one's expiry — what an
 * operator's `export` walks, page after page, in the UTF-8 byte order a `list`
 * answers in.
 *
 * A read: no transaction, no record, and the one liveness filter every read
 * applies, so a key that has expired is not exported. On a dialled store it is
 * one statement carrying the server's clock ([`RemoteKv`]'s `page`).
 */
export async function operatorPage(
  target: OperatorTarget,
  params: OperatorPageParams,
): Promise<OperatorPage> {
  const { identity, scopeKey } = operatorScope(target);
  const { store } = target;
  if (!opensInProcess(store.backend.provider)) {
    return await (await connectionFor(store)).page(store, scopeKey, params);
  }
  const database = await open(store, identity);
  const read = operatorPageRead(params, {
    from: "entries WHERE scope_key = ?",
    bound: [scopeKey],
    now: { sql: "?", bound: [localNow()] },
  });
  return read.answer(database.all(read.query.sql, [...read.query.bound]) as Row[]);
}

/**
 * The instant the store's clock reads — the server's on a dialled store, this
 * process's otherwise, shifted by [`STORE_CLOCK_OFFSET`] either way — which is
 * what an operator's `import` turns a line's absolute `expires_at` into a
 * remaining `ttl` against (PRD resolved q69, q72).
 */
export async function operatorClock(target: OperatorTarget): Promise<number> {
  operatorScope(target);
  const { store } = target;
  if (!opensInProcess(store.backend.provider)) return await (await connectionFor(store)).clock();
  return localNow();
}

/**
 * The statement one page of [`operatorPage`] runs, on every arm, and what its
 * rows answer.
 *
 * A `list` page with its values ([`partitionRead`]) plus the one column a
 * `list` does not answer: `expires_at`. One row past the page, so the extra
 * row says whether another follows.
 */
function operatorPageRead(
  params: OperatorPageParams,
  rows: PartitionRows,
): { readonly query: Query; readonly answer: (found: readonly Row[]) => OperatorPage } {
  const prefix = underPrefix(params.prefix);
  const cursor: Query =
    params.after === undefined ? { sql: "1 = 1", bound: [] } : { sql: '"key" > ?', bound: [params.after] };
  const alive = liveRows(rows.now);
  return {
    query: {
      sql:
        `SELECT "key", value, version, expires_at FROM ${rows.from} ` +
        `AND ${prefix.sql} AND ${cursor.sql} AND ${alive.sql} ORDER BY "key" LIMIT ?`,
      bound: [...rows.bound, ...prefix.bound, ...cursor.bound, ...alive.bound, params.limit + 1],
    },
    answer: (found) => {
      const page = found.slice(0, params.limit);
      const last = page[page.length - 1];
      return {
        entries: page.map((row) => {
          const held = heldOf(row);
          const expires = row["expires_at"];
          return {
            key: String(row["key"]),
            value: held.value,
            version: held.version,
            // `BIGINT` arrives as a string from `pg` and a number elsewhere.
            expiresAt: expires === null || expires === undefined ? null : Number(expires),
          };
        }),
        ...(found.length > params.limit && last !== undefined ? { next: String(last["key"]) } : {}),
      };
    },
  };
}

/** One row of the `blob` backend's idempotency ledger (grammar 9.4). */
interface LedgerEntry {
  /** The key this row was applied under — see [`markerName`]. */
  readonly key: string;
  /** The op it was applied by, which is what the SQLite ledger's column holds. */
  readonly op: StoreOp;
  /** What that op answered, which is what a later attempt is answered with. */
  readonly row: Record<string, unknown>;
}

/**
 * A ledger row's file name: the digest of the key rather than the key.
 *
 * A blob **value** key is percent-encoded and refused when it is too long for a
 * file name ([`blobName`]), because `list` reads those names back and has to
 * answer with the keys that were written. Nothing reads these back: this
 * directory is a set of markers, asked only "is this key in it". So the name can
 * be a digest, and it has to be — an idempotency key of grammar 9.4 is a path
 * through the execution (`<execution>/<node>/<attempt>/…`, one frame per
 * enclosing `map`), which is composed rather than written and grows without a
 * bound the author controls. Encoding it directly makes a deep enough fan-out
 * fail with `ENAMETOOLONG` **after** the effect has landed and before the marker
 * that would dedupe it — the one ordering that turns a retry into a second
 * write. The key itself is written *inside* the row, so a reader of the ledger
 * still sees what was applied.
 */
function markerName(idempotencyKey: string): string {
  return crypto.createHash("sha256").update(idempotencyKey, "utf8").digest("hex");
}

/**
 * Compare two keys the way SQLite's `ORDER BY key` does: by UTF-8 bytes.
 *
 * `list` is one op of grammar 11.4's catalogue and the two backends answer it
 * from different machinery — a `SELECT … ORDER BY key` for `kv` and `vector`, a
 * sort of file names for `blob`. JavaScript's own `<` on strings compares UTF-16
 * **code units**, which orders a surrogate pair (U+1F600 → 0xD83D …) *before*
 * U+FF00, while UTF-8 bytes order it after. Each backend would be internally
 * deterministic and the two would still disagree — and with `limit:`, which
 * grammar 11.4 makes required, disagree about which keys come back at all. So
 * the file-name sort is the SQL one, spelled out.
 */
function byUtf8Bytes(left: string, right: string): number {
  const encoder = new TextEncoder();
  const a = encoder.encode(left);
  const b = encoder.encode(right);
  const shared = Math.min(a.length, b.length);
  for (let index = 0; index < shared; index += 1) {
    const difference = a[index]! - b[index]!;
    if (difference !== 0) return difference;
  }
  return a.length - b.length;
}

/**
 * A `blob` store's partition on disk: everything one scope of it holds.
 *
 * Named rather than joined at its two call sites, because the two have to agree
 * exactly: [`blobOp`] writes under it and [`remember`] registers it for removal
 * when an execution-scoped run ends, and a directory removed by one path and
 * written by another is a lifetime that only looks kept.
 */
function blobRoot(store: StoreBinding, scopeKey: string): string {
  return path.join(dataRoot(), "blobs", encodeKey(store.name), scopeKey);
}

/** The `blob` rows of grammar 11.4's catalog, over a directory of files. */
function blobOp(
  store: StoreBinding,
  op: StoreOp,
  params: StoreParams,
  scopeKey: string,
  idempotencyKey: string | undefined,
): Applied {
  const root = blobRoot(store, scopeKey);
  const values = path.join(root, "values");
  const types = path.join(root, "types");
  const ledger = path.join(root, "applied");

  if (idempotencyKey !== undefined) {
    const marker = path.join(ledger, markerName(idempotencyKey));
    if (fs.existsSync(marker)) {
      const held = JSON.parse(fs.readFileSync(marker, "utf8")) as LedgerEntry;
      return { row: held.row, deduped: true };
    }
    const row = blobEffect(store, op, params, values, types);
    // Published by rename, which is atomic: a marker a reader finds is a marker
    // written whole, so a later attempt never parses half a row back as the
    // answer the first one gave. What a rename cannot give this backend is the
    // transaction the SQLite kinds have — see this module's header for what a
    // process that dies between the effect and the marker leaves behind.
    fs.mkdirSync(ledger, { recursive: true });
    const staged = `${marker}.${process.pid}.partial`;
    const entry: LedgerEntry = { key: idempotencyKey, op, row };
    fs.writeFileSync(staged, JSON.stringify(entry));
    fs.renameSync(staged, marker);
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
        .sort(byUtf8Bytes);
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
 *
 * One parameter of the catalog is **not** here, and its absence is the rule
 * rather than an omission: a `blob put`'s `content_type:` is a literal on the
 * node surface (grammar 8.8) rather than an expression, so grammar 11.5's
 * synthesized `put` offers `key` and `value` alone and the schema this parses
 * against is `.strict()`. A model has no way to send one, and a line reading for
 * it would be a mapping no call can reach.
 *
 * Three arguments are the tool surface's own spelling of a node parameter
 * (`check::model::store_tool_input` says why): the conditions arrive as `if`
 * and `if_version` with "no condition" spelled as their defaults, the empty
 * string and `-1`; and an `append`'s field and item arrive together as `item`,
 * an object naming one array field. A call that names both conditions, or an
 * `item` naming other than one field, is refused here and goes back to the
 * model (Decision D119) — before the store sees it, so it leaves no store
 * record.
 *
 * `site` is the call's place in the agent's loop, which a `history: true`
 * store's history row names a tool's write by (PRD resolved q74): the agent
 * execution's instance path and the call's own frame — `<tool name>/<call
 * ordinal>`, grammar 9.4's spelling of a tool frame ([`runtime.toolInstancePath`]).
 * A caller that hands none records a row with no path.
 */
export async function runStoreTool(
  store: StoreBinding,
  op: StoreOp,
  args: Record<string, unknown>,
  context: RunContext,
  site?: runtime.ToolCallSite,
): Promise<Record<string, unknown>> {
  const condition = args["if"] === undefined || args["if"] === "" ? undefined : String(args["if"]);
  const version =
    args["if_version"] === undefined || Number(args["if_version"]) === NO_TOOL_VERSION
      ? undefined
      : Number(args["if_version"]);
  if (condition !== undefined && version !== undefined) {
    throw new runtime.ToolCallRefused(
      `\`${store.name}_${op}\` was called with both \`if\` and \`if_version\`, and a write takes one condition: \`if\` asks whether the key holds a value and \`if_version\` which version it holds — send \`if_version\` alone to ask both`,
    );
  }
  let item: { readonly field: string; readonly value: unknown } | undefined;
  if (op === "append") {
    const named = Object.entries((args["item"] ?? {}) as Record<string, unknown>);
    const [only] = named;
    if (named.length !== 1 || only === undefined) {
      throw new runtime.ToolCallRefused(
        `\`${store.name}_append\` was called with an \`item\` naming ${named.length === 0 ? "no field" : `${named.length} fields`}: name exactly one array field, with the one item to add to it — \`{"item": {"<field>": <item>}}\``,
      );
    }
    item = { field: only[0], value: only[1] };
  }
  const params: StoreParams = {
    ...(args["key"] === undefined ? {} : { key: String(args["key"]) }),
    ...(item !== undefined
      ? { field: item.field, value: item.value }
      : args["value"] === undefined
        ? {}
        : { value: args["value"] }),
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
    ...(args["fields"] === undefined
      ? {}
      : { fields: args["fields"] as Record<string, unknown> }),
    ...(args["field"] === undefined ? {} : { field: String(args["field"]) }),
    ...(args["unique"] === undefined ? {} : { unique: args["unique"] === true }),
    ...(args["by"] === undefined ? {} : { by: Number(args["by"]) }),
    ...(condition === undefined ? {} : { condition: condition as StoreCondition }),
    ...(version === undefined ? {} : { ifVersion: version }),
  };
  const path =
    site === undefined ? undefined : runtime.toolInstancePath(site, `${store.name}_${op}`).join("/");
  try {
    return await runStoreOp(store, op, params, context, {
      via: "tool",
      ...(path === undefined ? {} : { path }),
    });
  } catch (error) {
    // A refusal the journal replayed comes back as a plain `Error` carrying the
    // class's name (`./journal.ts`'s `replayedFailure`), and the agent loop
    // reads the class to decide whether a call goes back to the model — so a
    // resume would otherwise end the node the run it resumes carried on from.
    if (
      error instanceof Error &&
      !(error instanceof runtime.ToolCallRefused) &&
      error.name === "ToolCallRefused"
    ) {
      throw new runtime.ToolCallRefused(error.message);
    }
    throw error;
  }
}

/**
 * The `if_version` a synthesized write is called with to ask for no version
 * condition — `check::model::STORE_TOOL_NO_VERSION`, the argument's default.
 */
const NO_TOOL_VERSION = -1;
