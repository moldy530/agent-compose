# agent-compose — Execution Journal

**Journal version:** 1
**Status:** Normative for the journal a compiled project writes and the replay it reads back
**Companion artifacts:** [`docs/trace.md`](trace.md) (the keying this shares), [`docs/grammar.md`](grammar.md) §9.4 (idempotency keys), §14.7 (the slot that binds the backend), [`docs/distributed.md`](distributed.md) (the wire the dispatch ledger of §3.8 serves), [`prd.md`](../prd.md) §5.8, §5.11, resolved questions 26–29, 62

An execution of a compiled graph survives the process that started it. This
document defines how: what is written, where, under what key, what a resumed
execution consumes and what it re-issues, and what a reader may rely on across
compiler releases.

PRD resolved q26 fixes the mechanism and rules out the obvious alternative:

> A journal + replay of the record this runtime already keeps, **not** a
> LangGraph checkpointer. […] the trace format was designed as a replay log, and
> durability is making the runtime write it as one and read it back.

resolved q27 fixes where it lives — "a deploy-target slot, exactly as
`storage_backends` are"; resolved q28 fixes the scope — `serve` auto-recovers,
`run` journals but is resumed explicitly, triggers fire once; resolved q29 fixes
what replay executes — "read-only up to the frontier"; and resolved q62 fills the
slot, with `sqlite`, `postgres` and `mysql` behind one interface (§10).

**Everything in this document but §2 and §10 is backend-invariant.** The record,
its keys, the frontier, the recovery verbs and `JOURNAL_VERSION` are the same
whichever backend a target bound, and the runtime cannot tell which it got. §2 is
where the durability contract is stated per backend, because "committed" and
"one writer" are the two things a file and a server mean differently; §10 is
where the binding is.

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense.

**Where the fields are implemented.** Every record type here is a TypeScript
interface in the emitted `src/journal.ts`, and so is every statement that reads
or writes one: there is **one** implementation of this document, and a backend
supplies a connection, a schema and a writer guard beneath it. The invariant half
of that module is byte-identical in every project a given compiler release builds
— like `src/runtime.ts` and `src/stores.ts` beside it — and a project whose
target binds a remote provider carries that arm appended to it and no other
(§10, `docs/grammar.md` §14.7).

---

## Table of contents

1. [The journal is not the trace](#1-the-journal-is-not-the-trace)
2. [Where it lives, and what a crash can leave](#2-where-it-lives-and-what-a-crash-can-leave) — the
   [backend-invariant guarantees](#21-what-every-backend-guarantees) and the
   per-backend contract: [SQLite](#22-sqlite),
   [Postgres and MySQL](#23-postgres-and-mysql)
3. [What is recorded](#3-what-is-recorded) — including
   [callback deliveries](#37-a-callback-delivery),
   [placement dispatches](#38-a-placement-dispatch) and
   [coding-harness runs](#39-a-coding-harness-run)
4. [Keys](#4-keys)
5. [Replay, and the frontier](#5-replay-and-the-frontier)
6. [Recovery: `serve` and `resume`](#6-recovery-serve-and-resume)
7. [Divergence](#7-divergence)
8. [Privacy posture](#8-privacy-posture)
9. [What the trace of a resumed execution looks like](#9-what-the-trace-of-a-resumed-execution-looks-like)
10. [Backends, and what binds what](#10-backends-and-what-binds-what)
11. [Stability](#11-stability)
12. [Out of v1 scope](#12-out-of-v1-scope)

---

## 1. The journal is not the trace

They share a keying discipline and nothing else, and the difference is a
requirement rather than an implementation detail.

`docs/trace.md` §11 keeps four things **out** of the trace: provider
transcripts (a model's completion has no field), what a model sent a tool, what
a tool answered on two of its three surfaces, and what a human answered. §11.1
adds that no resolved `${ENV}` value appears anywhere in it. That is what makes
a trace an artifact a reader may ship to an observability system.

Replay needs exactly those payloads, byte for byte, or it cannot answer a
recorded effect without re-issuing it. So the journal is a **second artifact**
with the same sensitivity as this project's stores: private recovery data, held
beside the stores, never rendered and never uploaded (§8).

What the two share is **identity**. An effect is addressed by the instance path
of `docs/grammar.md` §9.4 — the frames `docs/trace.md` §8 describes,
`<node id>/<traversal ordinal>` per node crossed from the root instance down,
plus `/<item index>` under a `map` and a `<tool name>/<call ordinal>` frame
under a flow-as-tool call. A key read out of a trace and a key read out of a
journal name the same effect site. This document invents no second addressing
scheme; §4 is the one thing it adds, and it is a suffix.

## 2. Where it lives, and what a crash can leave

**The target says where, and the composition says nothing** — resolved q27's
slot, filled by resolved q62 and spelled in `docs/grammar.md` §14.7:

```yaml
# deploy/<target>.yml
journal:
  provider: postgres      # sqlite | postgres | mysql
  url: ${JOURNAL_URL}     # required by postgres and mysql; sqlite takes none
```

A target that declares no `journal:` binds **`sqlite`**, under every target and
not only `local`. `deploy/local.yml` may not declare the block at all: `local`
binds the file unconditionally, so a `provider:` written there would never be
consulted (`docs/grammar.md` §14, Decisions D87, D148).

### 2.1 What every backend guarantees

Three statements hold on all three, and the rest of this document rests on them:

* **Atomicity is per record.** One effect is one `INSERT`, committed before it is
  answered. A process that dies mid-write leaves the row absent and never half
  present: nothing any backend holds can parse as a complete record that is not
  one.
* **The crash window is the network's, not the store's.** The row is written when
  the effect *answers*, so an effect that happened with no row for it is possible
  on every backend and is re-executed on replay. That is the at-least-once
  compromise `docs/grammar.md` §9.4's idempotency keys exist for — a repeated
  write carries the key the first attempt carried, so a receiver that dedupes
  still does — and it is stated here rather than promised away.
* **One process at a time writes a project's journal**, which on a mesh is the
  hub and only the hub: a worker streams its effect records home and never opens
  this connection (resolved q42). That stays a *rule* — neither lock below is a
  plan for two writers — and each backend puts a real one under it.

**Concurrency inside the process** is one serializing queue in `src/journal.ts`,
on every backend. A `serve` runs many executions at once, and several of the
statements here are a read and a write that have to be one step: a delivery's
ordinal is allocated and then inserted under, a dispatch is read before it is
parked, a settle reads the status its own update is about to change. The
synchronous SQLite driver gave that for free by blocking the event loop; a
backend reached over a socket cannot, so it is held explicitly and one reading of
those statements is right for all three.

### 2.2 SQLite

One file per project, beside the project's stores:

```text
<project>/.agent-compose/journal.sqlite
```

`AGENT_COMPOSE_DATA_DIR` moves the whole directory, exactly as it moves the
stores. The path is derived from the emitted project's own location rather than
from the process's working directory, and it is **stable across `run`, `serve`
and `resume`** of one project and one target — which is what makes
`agent-compose resume <execution>` find the execution a crashed `run` left
behind. Retention is deleting the file (resolved q27: "one file to delete").

`PRAGMA synchronous = FULL` is what makes "committed" mean "on the disk".
SQLite's **rollback journal** is what backs that; a write-ahead log is
deliberately not asked for, because this driver's virtual file system does not
implement one — the pragma is accepted and leaves the mode at `delete`, so asking
would be a line that reads like a guarantee and is not one. Atomicity per
statement is the same either way.

The writer guard is the **file lock**. `node-sqlite3-wasm` takes SQLite's
exclusive lock by creating `<file>.lock` as a directory and gives it back by
removing it, so a second process on one journal meets `SQLITE_BUSY` rather than
interleaving into pages the first has not committed. `PRAGMA busy_timeout` is set
before any statement that can contend, and the open is retried under a deadline
for the statements the pragma does not cover.

**A lock does not die with its owner**, and that matters here more than
anywhere: a process killed *inside* a write never reaches the `rmdir`, and the
directory it leaves would refuse every later open of that journal — the resume
of the very execution the crash interrupted, and every future run of the
project. So a lock still held after a deadline has been waited out is removed
and the open retried once. Under the rule above that is a lock whose owner is
gone, and every write here is a single statement, so a live owner never holds
one for anything like that long. The **rollback journal** the same crash leaves
is not touched: SQLite recovers it on the next open, which is what makes the
interrupted write leave no half-written row.

The consequence a reader should hold on to is §12's: a second process does not
merely queue on a SQLite journal — it eventually goes in.

The project's **stores** are opened the same way, and for the same reason
(`src/stores.ts`). They sit in the same directory, under the same driver and the
same one-process rule, and one crash leaves locks on both — so a store that did
not break a stale one would be a second artifact a single interrupted write can
render permanently unopenable, and the first thing it would refuse is the live
op a resume makes past its frontier (§5).

### 2.3 Postgres and MySQL

A connection rather than a file, at the address the deploy file's `url:` names as
an `${ENV}` reference — never a literal, so `validate` never sees a URL and the
artifact carries the variable's *name* (`docs/grammar.md` §4.3, §14.7; presence
is checked at launch, resolved q15). The driver is `pg` for Postgres and `mysql2`
for MySQL, pinned exactly and only into a project whose target binds it.

"Committed" is the **server's** commit: each statement is its own transaction and
the server has written it before it answers. The crash window above is unchanged
and is, if anything, plainer here — it is the round trip.

**Neither half of that sentence is a default, so each arm states it on its own
session.** MySQL's `autocommit` is what makes a statement a transaction, and
`mysql2` never sends it: on a server configured with `autocommit = 0` — a dynamic
system variable, settable globally or through `init_connect` — the schema's DDL
would still land, because DDL commits implicitly, and every record after it would
join one transaction with no `COMMIT` anywhere in the journal. Nothing would read
wrong while the process lived, since its own reads come back over the same
connection and see their own uncommitted rows; the loss would arrive at the one
moment the journal exists for, when the hub died and the server rolled the whole
transaction back. Postgres' `synchronous_commit` is the other half: set `off` —
per cluster in `postgresql.conf`, per database with `ALTER DATABASE … SET`, per
role with `ALTER ROLE … SET` — an insert answers before its write-ahead-log
record is flushed, so a crash or a power loss discards effects the run has
already treated as recorded and a replay re-issues the model calls and tool
invocations behind them. So the MySQL arm sends `SET SESSION autocommit = 1` and
the Postgres arm `SET synchronous_commit = on`, both before any record, and both
read back beside the reap window below. Unlike that window these are **not**
best-effort: a server that will not make the promise is refused the journal
rather than warned about, because the window costs a takeover minutes and this
costs the record. It is the same rule §10 states for `sql_mode` — a session
setting this journal's statements rest on is stated by the arm rather than
assumed of the server.

The writer guard is a **session-scoped advisory lock**, taken on the one
connection the process holds and held for as long as that connection lasts —
which for a `serve` is days, because the connection is heartbeaten and a
connection lost under a live process is a refusal rather than a redial (below):
`pg_try_advisory_lock` on Postgres, `GET_LOCK` on MySQL. It is defence in depth
under §2.1's one-writer rule rather than a second design, and a second opener is
**refused by name** — the message says that one process at a time writes a
project's journal, and that a `serve` which is up has already recovered every
execution it holds open, so one is finished through its
`POST /executions/:id/resume` route rather than beside it.

**The guard is per database, which is the same scope as the journal itself.** A
journal is the tables in one database, and everything pointed at that database
shares them — so the lock is taken under one constant name rather than under
anything derived from the project. It is *not* per server: `staging` and `prod`
routinely live in two databases of one managed instance, they share no table, and
neither has any business refusing the other. Postgres gives that for free, since
an advisory lock is scoped to the database the session connected to. MySQL's
user-level locks are keyed on the name alone across the whole server, so the
MySQL arm qualifies the name with the schema it is connected to — a digest of
`DATABASE()`, because MySQL refuses a lock name over 64 characters and a database
name may be 64 by itself. A `mysql://` URL that names no database is refused at
open for that reason among others: there is no schema to hold the tables or to
scope the guard to.

**They are the right primitive for the same reason SQLite's lock is the wrong
one: the server drops them when the session ends.** A hub killed on a machine
that is still running leaves nothing holding its journal — the kernel closes its
socket, the server ends the session, and the machine that takes over walks
straight in. There is no corpse to break, which is why neither arm has SQLite's
stale-lock rule and neither needs one. The `try` form is deliberate too: a
blocking acquire would leave an `agent-compose resume` typed beside a live
`serve` hanging on a lock that process may hold for days, rather than being told
what is happening.

**A host that vanishes closes nothing, and that is the case cross-host recovery
is for.** A crash, a power loss or a partition leaves the server holding a
session whose peer will never speak again, and a server left at its own defaults
finds that out very late: Linux's `tcp_keepalive_time` is two hours, and MySQL's
`wait_timeout` is eight. For that whole window the dead hub's guard would refuse
the live one — the `serve` restarted on a fresh machine that §10 says recovers
every open execution. So each arm **shortens the window for its own session** and
keeps a heartbeat on it, which bounds that lockout to **five minutes**:

| | how the server is told the peer is gone | set on |
|---|---|---|
| Postgres | `tcp_keepalives_idle` = 60s, `tcp_keepalives_interval` = 20s, `tcp_keepalives_count` = 12 | the session, at open |
| MySQL | `wait_timeout` = 300s | the session, at open |
| both | a round trip every 30s, so a `serve` that journals nothing all afternoon is still a live session | the held connection |

Postgres' keepalive probes are answered by the peer's **kernel**, so a hub wedged
behind a long step keeps its session and only a machine that is really gone loses
one. MySQL has no per-session equivalent, so its window is measured on the
application's own silence — which is what the heartbeat, an order of magnitude
below it, is there to fill. Five minutes rather than five seconds because the two
errors are not symmetric: a guard nobody holds costs a takeover a few minutes,
and a guard taken from a hub that is still writing costs the record. A server
that will not take the setting is **warned about on stderr** and opened anyway;
the guard then falls back to that server's own schedule.

**The window is read back rather than assumed to have taken**, because on this
one a `SET` that succeeds is not a setting that applied. Postgres' assign hooks
for the three keepalive GUCs call `pq_setkeepalives*` and discard the result, so
a platform without `TCP_KEEPIDLE` logs a server-side notice and still answers the
client, and a Unix-domain-socket connection is a documented no-op that also
answers — in both cases `SHOW tcp_keepalives_idle` reads back `0`, which is the
only place the difference is visible. `SET SESSION wait_timeout` on MySQL cannot
fail at all. So each arm asks the session what it is really carrying and warns on
a mismatch; without that read, the warning above is a line nothing could ever
print, and the bound in this table would be a claim rather than a check. It is
also the difference between an operator who is told to wait five minutes and
waits two hours, and one who is told which of the two they have.

Both are acquired **without a timeout**, and neither is ever released by
statement. A release a crash can skip would be a lock outliving its owner, which
is the whole failure this choice avoids.

**The guard is taken before the schema is created**, so two processes opening one
fresh journal at the same time have one creator rather than two racing
`CREATE TABLE IF NOT EXISTS` runs — which Postgres documents as not atomic
against a concurrent creator, and which would surface as a catalog error instead
of the refusal above.

**A connection lost under a live process is not redialled.** Every statement
after it is refused by name, because the guard went with the connection and
another hub may already have taken the journal over; a silent redial would take
the guard back and put two writers into one record. Each arm listens for its
driver's `error` event from the moment there is a connection to listen to — an
`EventEmitter` that emits `error` with nothing listening ends the process, and an
idle connection a server closes is the ordinary way that happens.

**Retention is the operator's `DELETE`.** "One file to delete" is SQLite's line
and stays SQLite's; a journal in a shared database is rows in tables an operator
prunes, and binding the payloads of §8 into one is the deploy file's explicit,
operator-owned choice.

The two live surfaces are therefore kept **apart** rather than serialized:

* an execution a live `serve` is holding is one `serve` has already recovered
  (§6.1 — it recovers *every* open execution at start), and it is finished
  through `POST /executions/:id/resume`;
* `agent-compose resume` is for an execution **no live process is running** —
  the one a crashed `run` left behind, which is the case resolved q28 gives it.

Running `agent-compose resume` against an execution a live `serve` is replaying
is outside the promise. Both generations would run past the frontier at once,
issuing live effects into one record.

**What a crash can still leave** is an effect that happened with no row for it:
the row is written when the effect answers, and the window between the two is
not closable by anything on this side of the network. A replay re-executes such
an effect. That is the at-least-once compromise `docs/grammar.md` §9.4's
idempotency keys exist for — a repeated write carries the key the first attempt
carried, so a receiver that dedupes still does — and it is stated here rather
than promised away.

## 3. What is recorded

Every effect, and completeness is the invariant: **an effect that is not
journaled is one a replay re-executes.** A reader verifies it the way the
implementation enforces it — by finding the call sites. Every effect goes
through `journaled(…)` or `EffectRecorder.claim(…)` in the emitted sources, and
there are exactly nine of them, in five kinds:

| kind | site in the emitted project | what the record holds |
|---|---|---|
| `model` | `callModel` in `src/runtime.ts` | §3.1 |
| `tool` | `runExec`, `runBuiltin`, `runHttp`, `callFunction`, `callModule` in `src/runtime.ts` | §3.2 |
| `store` | `runStoreOp` in `src/stores.ts` | §3.3 |
| `human` | `runHuman` in `src/runtime.ts` | §3.4 |
| `harness` | `runCoder` in `src/runtime.ts` | §3.9 |

Three constructs record nothing of their own, and need none: a `flow:` node, a
`map` dispatch, and a flow-as-tool call are *instantiations*, not effects. What
they run is a graph whose nodes reach the sites above under the
instantiation's own instance path — which is exactly how they trace
(`docs/trace.md` §8) — so their effects are journaled without a record for the
boundary itself.

That inventory is held **mechanically**, and by two tests in
`crates/compose-core/src/codegen/journal.rs` that read it from opposite ends.

`every_effect_site_reaches_the_journal_and_the_document_names_them_all` reads it
from the seams: each of the nine functions above is read out of the emitted
modules, and the test fails when one does not reach the journal, when this table
does not name it, or when a tenth site exists that this table does not.

`nothing_in_the_emitted_runtime_calls_the_world_except_under_a_journaled_seam`
reads it from the **primitives**, which is the direction the first cannot see.
A surface added to `src/runtime.ts` that calls the world and is not journaled is
a replay that issues it twice — and it is not one of the nine, contains no
`journaled(` and no `.claim(`, and is in no table, so nothing else in the
repository would notice. So every call to the world in every emitted constant
module is required to sit in a declaration the nine transitively reach.

What counts as a call to the world is **derived rather than listed**, because a
list of spellings is only as complete as the last person to extend it: `spawn(`
is not `spawnSync(`, and `fs.writeFileSync` is not `fs.appendFileSync`. Each
module's imports are read instead, and every specifier has to be classified as
one whose surface *is* the world — in which case every binding it introduces is
a primitive, whatever member of it is called — or one that reaches nothing
outside the process. A specifier in neither class fails the test until somebody
says which it is, and the platform globals that arrive with no import (`fetch`
and the transports beside it) are named in the test. A handful of sites are
exempt and each is named there with its reason: `releaseExecution`, which is
grammar 11.1's `scope: execution` lifetime; `attemptDelivery`, which is a
callback delivery and is journaled by the ledger of its own that §3.7 defines;
`writeTrace`, which is the command's document rather than the graph's effect
(§9); and the journal's own storage, which is the record a replay reads.

Whatever its kind, one effect's record carries the same five things: its **key**
(§4), the **request identity** the key's effect was issued under (§7), the
**outcome** — the answer or the failure, in full — the instant it was recorded
at, and one flag.

The flag is `refused`, and it is the one thing a record cannot say at the moment
it is written: whether the generation that recorded this effect went on to
**refuse its own answer** against the contract the node declared. The seam has
not seen the schema and the schema has not seen the answer, so the parse that
refuses a live answer marks its record as it raises, and a resume that meets
that answer again reads the mark. §7 is what it decides. It is `false` on every
record of a run that went as its composition expected.

**When the mark is down**, since a parse is synchronous and a mark is a write to
a journal that may be a network away: it is *issued* before the mismatch is
raised and **owed by the execution**, and the seam waits for what an execution
owes before it appends that execution's next record. So the ordering a later
generation reads is the one that decides anything — the mark is on the record
before the record of the attempt the mismatch set off, on every backend. A
process killed inside the gap between the two writes leaves the record unmarked,
which §2's window covers and §7 decides the safe way: a resume reports a
divergence naming the step rather than replaying silently past it. A mark the
journal refuses outright is not silent — it fails the node that would otherwise
have written past it, and says so on stderr.

### 3.1 A model call

One record per call `callModel` made, whichever way it ended. It holds the
answer the loop accepted **and** every `ModelCall` record the ladder filed on
the way to it — the failovers a route spent (`docs/trace.md` §7), and the
refusal that ended a call no member answered. Both halves are needed: without
the answer a replay cannot continue, and without the records the resumed
generation's trace would say the node called no model at all.

It also holds `served`, the route member that answered, in its own right rather
than as the last of those records. Where the node is collecting them the two are
the same call and the field is redundant; where it is **not**, it is the only
account of who answered — a detached `map` delivery runs with the node's
collectors detached (Decision D94, §3.2), so its record's list of filed calls is
empty by construction and a replay that read the answerer off the tail of that
list would fail a resume whose composition nobody had touched.

A call that **failed** is recorded as a failure and replays as one: the resumed
generation re-raises an error with the recorded class name and message, so the
node's own `retry:` ladder and `on_error:` decide exactly what they decided
before. What a replay cannot restore is the platform `cause` chain beneath it,
which `docs/trace.md` §11 already keeps out of the trace and which only the
human report prints.

The **request identity** (§7) is the `model.*` addressed, the system prompt, the
conversation as it stood, the tool names offered, the pinned tool where there is
one, and — where any member of the ladder declares one — the **server-tool
suites** of the providers behind it (`docs/grammar.md` §12.1, Decision D122).

A server tool is not an effect of its own and gets no record and no key: it runs
on the provider's side, *inside* the call, and its `server_tool_use` block and
the result the provider paired with it are part of the answer this record
already holds. So a resumed generation replays the whole turn, search and all,
and nothing reaches the network for it.

Its **suite** is in the request identity for the same reason the client tool
names are: it is part of what the model was offered. Per ladder member, because
each provider in a route declares its own array and which tools were on offer
depends on which member served the call. The consequence is the one resolved q29
intends and is worth stating outright: **a resumed execution whose provider
gained or lost a server tool between generations diverges at the first model
call.** That is correct. The recorded answer was produced by a model with a
different tool surface, and handing it to a graph that would now ask differently
is exactly the silent re-keying q29 refuses — so the resume fails, naming the
step, rather than continuing on an answer to a question this build no longer
asks.

The key is **omitted entirely** from the identity where no member declares a
suite, which is what keeps every composition that predates it deriving the
identity it already derived: a journal written by an earlier build still
replays, and `JOURNAL_VERSION` does not move for it (§11.2).

**The conversation carries which wire wrote it**, on a turn that came off one
of the two surfaces that answer with a list of objects. The Messages and
Responses vocabularies are disjoint, and a failover ladder may cross them, so an
assistant turn records the wire that produced its blocks and each surface
replays only its own — rendering the other's turn from the same reading it
renders its own composed turns from. This is in the identity because it is in
the conversation, and it is written **only** for a Responses turn: the Messages
wire was the only surface that ever produced blocks, so leaving its turns
untagged keeps every identity an earlier build derived, on the same terms as the
paragraph above (§11.2).

Nothing else about the Responses wire is in the identity. The runtime sends no
`previous_response_id` and holds no continuity token, so what a resumed
generation replays is the recorded answer's own items — ids and all, which is
why it derives the same identity only by going on with the *kept* answer rather
than a re-reading of it (§11.1).

### 3.2 A tool execution

One record per `exec:`, `http:`, `function:` or `module:` invocation, at either
of the two surfaces a tool has — a node's own activity, and a tool an agent's
model called. The record holds the value the binding answered, before the
declared `output:` schema parses it; a failure is recorded and replayed as §3.1
replays one.

A **`module:` binding** (`docs/grammar.md` §6.1, PRD resolved q48) is one of
these and nothing more: authored TypeScript running in this process rather than
a subprocess or a request, journaled so that a resumed execution does not run it
a second time. Its request identity is the tool's address, the file the binding
names, the `env:` it declared as written, and the input the graph built. What is
**not** in it is the file's contents — an implementation that changed is a
different artifact hash (PRD resolved q49) rather than a different effect key,
which is the posture a `function:` registration already takes: the identity is
what the *composition* says, and the composition says which file, not what is
in it.

The request identity is the binding **as the composition wrote it** — the
`${ENV}` references unresolved — plus the input the graph built. That is
`docs/trace.md` §11.1's discipline read where the reader is a replay: one
composition derives one identity whatever environment it runs in, so an
execution journaled on one machine is not reported as divergent on another for
having a different `${TOOLBIN}`.

The binding **whole**, field for field: an `exec:`'s `command:`, `args:`,
`cwd:`, `env:` and `expect_exit:`, an `http:`'s `method:`, `url:`, `headers:`
and `expect_status:`, and on both the shape the declared `output:` decodes into.
Each of them changes what the call is or what its answer means, so a binding
that moved in any one of them is a call this run does not make and a recorded
answer is not its.

A **built-in tool** (`docs/grammar.md` §5.5, §6.1, Decision D135) is one more
record of this kind, and is journaled for the reason the kind exists: a `bash`
that appended a line to a file appended it once, whatever number of process
generations the execution takes. A resumed generation is handed the recorded
stdout and stderr and runs no command — the record is consumed exactly as an
`exec:` tool's is, and replay is where a built-in and a hand-rolled `exec:`
tool are indistinguishable.

Its request identity is the **binding** as the composition wrote it — which
built-in, its `workspace:` unresolved, and `builtin.bash`'s `timeout:`, `env:`
and `inherit_env:` as written — plus the arguments the model chose. Unresolved
for the reason above; whole for the other one: a workspace that moved is a
different directory to have worked in, a timeout that moved is a different bound
to have survived, and an environment that moved is a different program, so none
is a call this run makes under the recorded key.

The one thing a replay does not restore is the **shell session**. A
`builtin.bash` shell lives for the agent node activity that opened it, and a
replayed command is a command that did not run — so a generation that resumes
past the frontier opens a fresh shell in the workspace, whatever the recorded
prefix had `cd`ed into. The recorded answers are still the recorded answers;
what changed is where the *next* live command starts, which is the same thing a
node `retry:` does to a session and for the same reason. A composition whose
later commands depend on a directory an earlier one moved to should say so in
each command — `cd build && make` rather than a `cd` a resume may not have
made. What the record holds is
the tool's whole answer — a file's contents, a command's output — which is §8's
posture and not the trace's: `docs/trace.md` §11 keeps a tool's answer out of the
trace, and this record is private recovery data that a `run --format json` never
carries.

A **detached** `map` delivery (`docs/grammar.md` §8.6 rule 7) is journaled like
any other effect under the dispatch's own instance path. Its outcome is never
observed by the join (Decision D94) and nothing about the parent's trace
changes; what the record buys is that a replay does not deliver it twice.

A detached **`flow.*`** delivery is also where the trace sink's second event
class comes from (`docs/trace.md` §1.4, PRD resolved q64): when it settles it
ships an envelope of its own. That envelope is a delivery on the parent's
ledger (§3.7) and nothing else — no lifecycle row, no journal of its own, no
record this section does not already write — because the delivery is not an
execution: its effects stay journaled under the parent's id, and replay is
untouched. Its recovery is the parent's. A generation that resumes the parent
re-runs the delivery, replaying what this section holds, and the delivery
re-ships on **that** settlement under a new delivery id — so the envelope is
at-least-once across recovery, and a receiver dedupes it on
`(parent_execution, idempotency_key)` rather than on the delivery id. A
delivery that meets a divergence ships nothing (§7: a divergence fires
nothing), and one still in flight when a settled parent's process dies is not
recovered, because nothing resumes a settled execution. A divergence raised
inside one is the exception to rule 7's "nothing it does can delay the enclosing
flow instance" — §7 makes a divergence un-absorbable by any policy, and
`detach:` is a policy — so it is held against the execution and fails it: at the
next effect any branch of the run reaches, or on the way out of `runFlow` for a
run with none left.

What no *flow instance* waits for is the delivery itself, which is rule 7. The
**execution** waits on the way out, on the two occasions where a delivery still
in flight changes what the run has to decide.

The first is a run whose lifecycle row stays **open** (§3.6) — parked at a
`human` pause, or stopped by a divergence. Such a run is one a resume replays,
and a delivery still in flight when the process walks away is an effect with no
row, which the resumed generation issues a second time. That is not §2's window
and not a crash: `agent-compose run`'s exit `3` is a documented way to stop, so
the repeat would be systematic rather than a race.

The second is a **resumed** generation, whatever it ended as, and there the wait
is about the outcome rather than the record. Whether the row stays open is
decided by whether a divergence was raised, a delivery is the one place one can
be raised with nothing to throw it to, and only a generation consuming a record
can raise one at all — a first generation has nothing to disagree with. A
reading taken while a delivery was still working would be a reading taken before
the execution had an answer: the row closed `completed` over a delivery the
record describes and nobody made, or the `scope: execution` partition of §5
removed under the resume it was being kept for.

A **first** generation that ended waits for nothing, because nothing will replay
it — and a divergence cannot arise in one.

### 3.3 A store op

One record per op, reads and writes alike, holding what the op answered and
whether the backend had already applied its idempotency key. This is where
PRD 5.8's replay discipline — "reads are recorded; replay consumes history, not
the live store" — stops describing the trace and becomes the mechanism:

* a replayed **read** answers what it answered, without touching the store;
* a replayed **write** is **not applied a second time**. The row the journal
  holds is the row the first generation wrote, `deduped` included, so the
  `StoreRecord` a resumed run files is the record the crashed one would have
  filed.

The request identity is the store's address, the op, the scope and the partition
it addressed, which surface ran it, and the op's evaluated parameters.

### 3.4 A human wait

One record per **settled** wait, holding how it settled and — where somebody
answered — the answer itself. A wait that expired records that it expired; the
route it takes is the composition's `on_timeout:` and is read off the node
rather than off the record.

It holds **all three of the wait's instants** — when it began, when it would
have expired, and when it stopped waiting — because all three describe the
generation that held the pause and none of them describes the one that reads the
record back. A resumed generation that dated `pausedAt` by its own clock and
took `settledAt` from the record would file an entry whose answer arrives before
its question, which is the opposite of what §9 promises a reader.

A replayed answer is **parsed against the node's `output:` again**, by the same
schema the delivery that recorded it was held to. An answer a narrowed `output:`
no longer admits is §7's second divergence and is raised as one: no other reader
would catch it — a human answer reaches no result parse — and the resume would
otherwise end reporting a value the composition refuses.

A settlement the journal **cannot record** fails the `human` node with the
write's own error. The answer is not offered back to whoever gave it — the turn
was spent, and a pause may not settle twice — and the run stops there rather
than going on from a wait its own record does not hold, which is a wait the
resume would put to the person a second time.

An answer is **acknowledged no sooner than it is recorded**. The surfaces that
take one — `POST /executions/:id/resume`'s `202`, and the `taken.` the terminal
of a `run` prints — answer once the record above is down, not when the wait was
settled in this process's memory. Settling a wait is synchronous and appending
its record is not; on a remote journal (§10) the difference is a network round
trip, and a hub whose host dies inside it is recovered on another machine, finds
no record at that wait's key, re-parks it by the paragraph below, and puts to the
person a question they have already been told was answered. A *refusal* carries
no such promise and needs none: every refusal on both surfaces consumes nothing.

An **unsettled** wait records nothing, and that is the whole of re-parking: a
resumed execution reaching a wait the journal does not hold parks under the same
wait id — the id is the node's instance path (`docs/grammar.md` §9.4), so it is
deterministic and identical across process generations — and a resume request
that arrives after the restart finds it.

**A wait a placed node opened is this record too**, written by the hub rather
than by the process that asked. `docs/distributed.md` §3.4 makes a pause the
third way a dispatch ends: the worker settles its dispatch with the wait it
opened, the hub plants that wait on its own board and journals the answer *here*,
under the effect key the worker's own recorder claimed — and the node re-enters
dispatch, so the redispatch's `effect_history` carries this record and the replay
above is what consumes it. Every rule of this section reaches it unchanged,
because it is the same record: its three instants are the wait's own rather than
the reading generation's — when the hub began holding the question, when it
would have stopped waiting, and when it stopped — an unsettled wait records
nothing and is re-derived, and a settlement the journal cannot record fails the
node.

All three are the **hub's** clock, and the paragraph above is why: a record that
took `pausedAt` off the wire — the worker's reading of its own clock — and
`settledAt` off this one would file, on a worker running ahead of the hub, an
entry whose answer arrives before its question. So the wait is dated where it is
planted, exactly as a local one is dated where it is parked, and the pair a
reader is shown is the node's `timeout:` apart on either side of the wire (PRD
resolved q46's parity bar). What the worker's clock said is not lost: it is on
the settled dispatch row (§3.8), which is the record of what that machine did.

This is the record that makes §1 concrete. `docs/trace.md` §11 says outright
that what a human answered "is not here"; it is here.

### 3.5 The execution's lifecycle row

One row per execution, written before the graph is streamed:

| field | meaning |
|---|---|
| `id` | the execution id — what `resume` takes and what the trace's envelope carries |
| `flow` | the flow's typed address |
| `trigger` | what started it: `manual` for `agent-compose run` and for a `manual` trigger, an `http` trigger's own name where one did. **Recorded and never dispatched on** — resolved q28: recovery replays executions that exist, it does not re-fire the trigger that created them. It is *read* for one thing besides diagnosis: this execution's resume and status routes enforce the `auth:` of the trigger that started it, and a `serve` restarted while somebody was thinking has no other way to know which that was (`docs/grammar.md` §13.3, PRD resolved q32) |
| `inputs` | the invocation's inputs, as the flow's `inputs:` parsed them |
| `sessionKey` | the session identity `scope: session` stores key off (`docs/grammar.md` §11.3) |
| `callback` | where this execution's lifecycle webhooks go, for an `async` `http` trigger that asked for one (`docs/grammar.md` §13.3) — absent for every other invocation. Recorded because the process that *finishes* an execution need not be the one that started it (§6.1), and resolved when the request arrives rather than when the run ends, which is what makes that possible. The URL only: the request it came out of is not kept. §3.7 is what is delivered to it |
| `status` | `open`, `completed` or `failed` — §3.6 |
| `journalVersion` | the version at the head of this document |
| `startedAt`, `endedAt`, `error` | when, and why it failed |

### 3.6 What `status` means

Three values, and which one a run leaves behind is what decides whether it can
be resumed:

* **`completed`** — the run produced its `outputs:`. Nothing left to replay.
* **`failed`** — the run produced no answer and was not holding a question: a
  node failed and the composition's own `on_error:` decided the run. Replaying
  it would re-derive the same failure from the same record, so `resume` refuses
  it by name. A **divergence** is not one of these and never closes a row (§7).
* **`open`** — the run has not ended. Two ways to be open, and both are exactly
  what recovery is for: the process died, or the run reached a `human` pause
  with nobody to answer it (`docs/grammar.md` §8.7) and ended `3`. The second is
  the reason an interrupt does not close the row.

### 3.7 A delivery

**Not an effect, and that is the whole shape of it.** A **delivery** is
something the execution's lifecycle owes somebody outside it, delivered as one
POST. There are two kinds, and they differ in who asked rather than in how they
are worked:

* a **callback**, which an `http` trigger's `callback:` subscribed to — every
  quiescence that opened new pauses, and settle (`docs/grammar.md` §13.3, PRD
  resolved q34);
* a **trace sink** export, which the deploy layer's `trace_sink:` subscribed to
  for every execution the deployment settles (`docs/grammar.md` §14.5, PRD
  resolved q50). One per settled execution, carrying the trace envelope
  `docs/trace.md` §2 specifies or the OTLP/JSON §12 maps it to — and, the
  sink's second event class, one per settled **detached `flow.*` delivery**,
  carrying that delivery's own envelope (`docs/trace.md` §1.4, PRD resolved
  q64). A delivery's envelope is a row on the ledger of the execution it ran
  under, because a detached delivery is not an execution: it has no lifecycle
  row and no journal of its own. It is on that ledger and not in that
  execution's **report**: the status route's `deliveries`, which every `parked`
  and `settled` webhook carries (`docs/grammar.md` §13.3), lists the
  execution's own deliveries and leaves a delivery's envelope out — it is not
  one of the execution's lifecycle events, and it is journaled while the
  execution may still be running or parked.

Nothing in the composition dispatches either: the graph does not know the
subscription exists, no instance path addresses it, and no replay ever consumes
it. So a delivery is **not** an `EffectKind`, gets no §4 key, and never appears
in the frontier §5 defines. It is a second ledger beside the effects, and what
it owes is durability rather than replay.

What is recorded, and in this order (PRD resolved q35, q50):

| field | meaning |
|---|---|
| `execution`, `ordinal` | who it is about, and which of that execution's lifecycle events it is. The ordinal is **monotonically increasing per execution across every kind and event** and is allocated in the journal, so a restart cannot reuse one |
| `id` | `<execution_id>:<ordinal>` — the `X-AgentCompose-Delivery` header, and what a receiver dedupes on. The same on every attempt |
| `kind` | `callback` or `trace_sink` — who asked. A row written before the ledger recorded this is a `callback`, which is the only kind that existed then |
| `event` | `parked` or `settled`. A `trace_sink` row is always `settled`: it is the record of a run that has stopped — or, for a detached delivery's envelope, of a delivery that has. The two are told apart by the body's head (`docs/trace.md` §1.4) rather than by a column, and a settle's "is this execution already exported?" asks about the execution's own export only |
| `trigger` | the trigger whose `callback_auth:` signs this delivery and whose `callback_allow:` admits its URL (`docs/grammar.md` §13.3). On the delivery rather than read off the execution's lifecycle row when it is picked up, because one delivery has no lifecycle row to read: the `settled` journaled for a run that failed **before** it was journaled at all. A start that could not name that row's trigger could neither send it nor end it, and `pending` is neither of the two ends below. A `trace_sink` row carries none — the identity it signs with is the deploy layer's own, so there is nothing for a later start to look up |
| `url` | the callback URL the request payload named, resolved when the request arrived (§3.5); or, on a `trace_sink` row, the address the deploy file wrote |
| `body` | the exact bytes every attempt POSTs. Bytes rather than a value, because a signature is over what is sent: a body re-serialized on a later attempt, or in a later process, would be a second delivery wearing the first one's id |
| `pauses` | which pauses a `parked` delivery reported, by wait id; empty on a `settled` one. What keeps a **recovered** execution from re-announcing a question already asked — it re-parks under the same wait ids (§6.1), so a parking fires only where a quiescence opened a pause this set does not hold |
| `status` | `pending`, `delivered`, `refused` or `exhausted` — below |
| `attempts` | every attempt so far: when it was made, whether the receiver took it, and what it answered |
| `intendedAt`, `settledAt`, `detail` | when the intent was recorded — what the schedule is measured from — when it stopped being `pending`, and why |

**The intent is recorded before the first attempt.** That is what makes delivery
at-least-once rather than at-most-once: a process that dies mid-attempt leaves a
row a later start finishes, under the id the receiver dedupes on. The
consequence is the one a receiver has to be built for and `docs/grammar.md`
§13.3 states where it meets one — **a delivery can arrive twice, and two
deliveries can arrive out of order** — so receivers dedupe on
`X-AgentCompose-Delivery` and order on `X-AgentCompose-Ordinal`, never on
arrival.

**The retry schedule is normative**: five attempts, at **`+0s`, `+15s`, `+60s`,
`+240s` and `+600s` from the intent**. Only a network error or a non-2xx status
is retried; a `2xx` is delivered and stops the schedule. A `3xx` is one of those
non-2xx statuses rather than a hop to take: a delivery follows no redirect, so
the allowlist bounds where it lands and not merely where it was aimed
(`docs/grammar.md` §13.3). The offsets are
measured from the recorded intent rather than from the last attempt, which is
what lets a restart resume a delivery where it left off — a row with two
attempts on it resumes at the third offset, due at `intendedAt + 60s`, which may
already be in the past.

**One attempt waits ten seconds for a receiver**, and that bound is normative
too. A schedule is only bounded if its attempts are: a receiver that completes
the connection and never answers would otherwise hold one attempt open for the
life of the process, leaving a delivery neither retried nor exhausted and an
execution that finished minutes ago reported as still owing a webhook. An
attempt that runs out of time is a failed attempt like any other — recorded, and
followed by the next offset. Nothing configures it; a deployment that needed a
longer one is a deployment whose receiver is the thing to fix.

**`AGENT_COMPOSE_CALLBACK_RETRY` overrides the schedule** with a comma-separated
list of `docs/grammar.md` §4.4 durations — `AGENT_COMPOSE_CALLBACK_RETRY=0s,1s,2s`
is three attempts, the first at once. It is a **diagnostic and test surface**
rather than a deployment knob: a schedule is a promise to a receiver, and the
one this document states is the promise. `0` is admitted where §4.4 admits none
because the schedule's own first offset is `+0s`. A value that is not such a
list — a duration this grammar does not spell, or an empty list — is a **usage
error refused at launch**, naming what could not be read, rather than a setting
nobody read (`docs/grammar.md` Decision D50).

**Two ends are recorded and neither is the execution's failure.**

* **`refused`** — the callback URL matched no `callback_allow:` entry. The URL
  comes out of the request payload and is attacker-controlled by construction,
  so it is matched **when it is read**, at the delivery rather than at the start
  (`docs/grammar.md` §13.3, Decision D110, D127). Nothing is sent, nothing is
  retried, and the refusal is on the status route. It is a `callback` row's
  outcome and **only** a `callback` row's: a sink's address is the operator's,
  written in the deploy file and admitted by no list, so there is nothing for it
  to fail to match (`docs/grammar.md` §14.5). The ledger says so twice — the
  journal call that opens a refused row takes a callback intent and no other,
  and the statement that refuses a recorded row matches a callback row and no
  other.
* **`exhausted`** — the schedule ran out. A webhook is a courtesy the status
  route backstops, not a contract worth an unbounded queue. A row is exhausted
  the moment the schedule holds no offset it has not already had an attempt at,
  which is ordinarily the last attempt failing and is also what a restart under a
  **shorter** `AGENT_COMPOSE_CALLBACK_RETRY` than the one that wrote the row
  meets: there is no attempt left to make, so the row is ended rather than left
  `pending` for a start that would read and skip it for ever.

Neither reopens or fails the execution: a run that produced its outputs produced
them, and `status` on the lifecycle row says nothing about who was told.

**A restart resumes what is `pending`** (§6.1), beside the executions it
recovers and separately from them — a delivery reports on an execution that may
have *ended* in the process that died, so there is nothing to recover and
something still to send.

One case is left undelivered rather than sent, and it is the mirror of §6.1's
open execution of a flow this build no longer declares: a delivery whose
**trigger** the composition no longer declares. This build cannot know what
identity that trigger's `callback_auth:` promised its receiver, and delivering
without it is a request a receiver written against the promise refuses — or
worse, accepts. So the row stays `pending` for a build that declares the
trigger, and the reason is written on stderr — a wait for a trigger the row
itself names, which is why waiting is enough. The row is **written** either way:
an event that happened and was recorded nowhere would be both unrecoverable and
invisible — no restart could find it and the status route would show a settle
nobody was ever told about — so the intent goes down under its ordinal like any
other and only the sending waits.

Which is also where such a row meets `callback_allow:`. The allowlist belongs to
the trigger, so a row journaled by a build that had none of it has never been
matched against one; the build that declares the trigger again matches it as it
picks the row up, and a URL the list admits nowhere becomes `refused` **on that
row** rather than as a second event. The ordinal counts lifecycle events, and the
event did not happen twice.

**The `settled` intent is recorded before the lifecycle row closes.** It is the
same rule as "before the first attempt", one step further back, and it is what
`recover` needs to be able to help: an execution whose row is already
`completed` is one no start will replay, so a delivery first journaled *after*
the close and interrupted before it lands is owed to a caller who — having been
handed a `202` under `respond: async` — is not polling. The emitted `runFlow`
takes a `closing` hook for it, called on exactly the paths that close the row,
which is also what keeps §7's divergence silent: a row that stays open never
reaches one.

**And `serve` is not the only process that closes one.** `agent-compose resume`
(§6.2) finishes an execution an `http` trigger started just as readily, so it
supplies the same hook and journals the same intent: without it a hand-resumed
execution would close with no delivery row beside it, which is the one shape no
later start can repair — `recover` finds no open execution and the ledger holds
no `pending` row, so the push is lost silently. What that command does *not* do
is send: the schedule above outlives a command that exits when its run does, and
matching `callback_allow:` and signing with the trigger's `callback_auth:` are
the app's. So the row goes down and the next `serve` start delivers it, exactly
as it does for the row a build that no longer declares the trigger left behind.

**And a trace sink is delivered from a command too.** PRD resolved q50 puts the
sink "wherever executions settle under a target that declares it — `run`
included, not just `serve`", and unlike a callback there is nothing about a sink
delivery that only an app can do: no allowlist to match, and an identity that is
the deployment's rather than a trigger's. So `agent-compose run` journals the
intent from the same closing hook and then works **every offset already due** —
with the schedule above, the first one — before it exits, leaving the rest on the
row for whichever `serve` start picks it up. It sends after the run has reported
and never before it: a sink that is unreachable costs the delivery a retry and
the command nothing, which is resolved q50's "never blocks or fails the run it
describes".

**Where it is implemented.** `workDelivery`, `journaling`, `shipTrace` and
`attemptDelivery` in the emitted `src/delivery.ts`, over the delivery interface
of `src/journal.ts`; `deliver` and `opening` in `src/serve.ts` for the half that
is a trigger's — resolving a callback URL and holding it to `callback_allow:` —
and `shipping` there for the trace; `owed` and `settled` in `src/cli.ts` for the
`resume` and `run` above; and `runtime.executionReport` writing the report body a
callback and the status route both publish. A detached delivery's envelope is
`shipDetachedTrace` and `journalDetachedTrace` in `src/delivery.ts`, reached from
`runtime.watchDetachedSettlements` — which `src/serve.ts`'s `exportingDetached`
and `src/cli.ts`'s `execute` subscribe to — and `runtime.traceSinkClass` is the
one reader that tells an execution's own export from one of those: a settle's
guard asks it through `executionExport` there, and `runtime.executionReport`
asks it to leave a delivery's envelope out of the report. `attemptDelivery` is the one
declaration in the emitted app that reaches the network for a delivery, which is
why §3's primitive walk names it as an exemption and
`crates/compose-core/src/codegen/journal.rs`'s
`a_delivery_is_journaled_before_it_is_attempted` binds the order this section
states. What the deliveries *report* — one webhook per parking, listing every
pause then open — is `serve.ts`'s `parking` over `runtime.quiescent`, and PRD
resolved q34 is where that rule is stated.

### 3.8 A placement dispatch

**The third ledger, and the second thing here that is not an effect.** A
composition whose deploy target declares `placements:` runs some of its nodes in
another process, and `docs/distributed.md` is the document for that wire. What
belongs *here* is the one sentence of it this journal is the subject of: §8 rule
3 of that document says dispatch state lives in the journal and nowhere else, so
a placed node's dispatch is a row beside the effects rather than a scheduler's
memory.

One row per **placement wait** — a placed node instance the hub is waiting on —
holding what a worker is handed and how the wait ended:

| field | meaning |
|---|---|
| `execution`, `wait` | the identity, and the primary key. `wait` is `<instance path>/<ordinal>`, the deterministic identity `docs/distributed.md` §6.1 fixes, derived exactly the way §3.4's re-parking wait id is — so a resumed generation reaching the node again finds the row its predecessor left instead of opening a second dispatch for work a worker may already be running |
| `id` | `dsp_…`, the handle the wire attributes a result by. The issuing hub's own, opaque to a worker, and **not** the identity: a re-park after a supersede is a new `id` at the same `wait` |
| `placement`, `node`, `site` | where the work is queued — a placement, never a worker — and what it is |
| `inputs`, `itemIndex`, `history`, `policy` | the dispatch payload: what the node's input phase built, plus the three facts about the node execution the hub holds that a worker cannot derive. On the row rather than in memory, so a hub restarted mid-dispatch hands over what its predecessor would have |
| `status` | `parked`, `dispatched`, `settled` or `superseded` — the vocabulary below |
| `session`, `parkedAt`, `dispatchedAt`, `settledAt`, `detail` | which worker session is holding it while one is, and the three instants of the wait, for the reason §3.4 keeps all three of a human wait's |
| `outcome` | what the worker answered, once one did: a value or a failure, in the shape §3.1's outcomes take — or, on a dispatch a **pause** ended, the wait itself (`docs/distributed.md` §3.4) |

**What `status` means** is the same kind of statement §3.6 makes about an
execution, and the four are not interchangeable:

* **`parked`** — on the board, and no session is holding it. This is the pause:
  the node is waiting for a worker that claims its placement.
* **`dispatched`** — a session was handed it and has not settled it.
* **`settled`** — a worker's result ended it, and the outcome on the row is that
  result. A resumed generation consumes it rather than dispatching again, which
  is §5's replay discipline reaching this ledger. **A pause is one of the three
  ways a result ends a dispatch** (`docs/distributed.md` §3.4), so the outcome
  here may be the wait a `human:` node opened rather than the node's answer — and
  a resumed generation consumes that too, by re-deriving the wait onto its own
  board unless §3.4's answer record is already in the journal, in which case the
  wait is over and the next dispatch at that instance path is what replays past
  it. **The two instants on such an outcome are the worker's**, and this row is
  the only place they are kept: `paused_at` is when that machine reached the
  `human:` node and `expires_at` is what its own trace entry showed, both read
  off its clock. Neither is a deadline this hub ever armed and neither is the
  wait's — §3.4's record and the status route both date the wait where the hub
  planted it — so this row answers "when did that machine ask", and the human
  wait record answers "what was the question under".
* **`superseded`** — the hub ended it *without* a result, which is the only way a
  dispatch ends that a worker did not end. A resumed generation replays the
  failure, so the node's `retry:`/`on_error:` chain does now what it did then.

**It holds no §4 key and is never consumed by a replay of the effect frontier.**
That is the property it shares with the delivery ledger and the reason it is a
table of its own rather than a fifth `EffectKind`: the effects a placed node
issues are journaled under **its own instance path**, by the ordinary rule, out
of the batches the worker sends home — so the frontier is exactly where it would
be had the node run on the hub, and a redispatch replays to it. The dispatch row
is the *bookkeeping* around that, not a step in it. What a resumed execution
reads off the row is which of the four states above the wait is in.

**Where it is implemented.** The `dispatches` table and the `park`,
`dispatchAt`, `dispatchOf`, `unsettledDispatches`, `claimDispatch`,
`releaseDispatch`, `settleDispatch` and `supersedeDispatch` methods of the
emitted `src/journal.ts`, read and written by `src/mesh.ts` alone — a worker
never touches this journal, which is what keeps SQLite a valid backend for a
mesh (`docs/distributed.md` §3.3, §8). `effectsUnder` is the one read the wire
adds beside them: every effect at or inside one instance path, which is the
`effect_history` a redispatched node replays to the frontier before going live.

### 3.9 A coding-harness run

One record per run a `coder:` node made (`docs/grammar.md` §8.9, PRD resolved
q57). **One effect per run**, not one per turn and not one per tool call the
harness made inside it: what the graph asked for is an answer, and what it gets
back is the boundary of somebody else's agent loop.

The record holds three things, and the split between them is the whole of this
section:

| part | what it is | who reads it |
|---|---|---|
| the **answer** | the run's structured output, already through the node's `output:` gate | a replay, which consumes it and does not run the harness again |
| the **record** | the trace's account of the run — its harness and pinned SDK, its top-level turns and tool events, its cost rollup (`docs/trace.md` §7.6) | a resumed generation, which writes a fresh trace document whole (§9) and would otherwise report a run that never happened |
| the **payload** | the run's whole event stream: every turn, tool call, reasoning item and subagent transcript the SDK yielded | nothing, in a normal run. It is the private half §8 is about |

**The payload is where a harness's transcript lives, and where it stays.** It is
the largest private thing this journal holds, and it is the reason `docs/trace.md`
§7.6 draws its envelope at top-level turns and tool events: a harness that runs
subagents yields their whole conversations, and those belong here beside a
model's completions rather than in a document a run hands out.

**A run that failed is recorded too, and it replays as the same failure** —
§3.1's rule for a model call, one construct along, with one thing more kept
beside the message. A harness run fails in ways that are the node's business
rather than the machine's: the harness reported a fatal error of its own, it
produced no structured output at all, or its answer failed the node's `output:`
gate. Each of those really ran, and `docs/trace.md` §7.6 promises one record per
attempt across a `retry:` ladder — so the run's **record** and its **payload**
are kept with the sentence that ended it, and the resumed generation raises a
failure reading word for word as the first one did, carrying that record into
its own fresh trace. Keeping only the message would make §7.6's promise hold on
the generation that ran and quietly fail on the one that resumed.

**A crash mid-run recorded nothing.** The slot is claimed when the run starts and
written when it ends, so a process that died in between left no record at all —
which makes the next attempt an ordinary attempt: the node's `retry:` re-runs the
harness from its start, and a resume that reaches the node with no record
performs the effect. The cost is real and is the accepted one: whatever the run
had done to the workspace before the crash stays done, and the second run meets
it. That is the same statement §3.2 makes about a subprocess that wrote a file
before its host died, one agent loop larger.

**`workspace: fresh` is the one node that does not pay it**, and paying it is
the reason: the runtime remakes that directory **empty** at the start of every
attempt (PRD resolved q61 ruling c), because a half-clobbered workspace is
routinely why the attempt being retried failed. Nothing about this section moves
with it — a run's record is its structured output and its payload, and the
directory's contents never were — so a replay neither empties the directory nor
needs it: the emptying happens inside the effect's own slot, which a replay does
not enter.

**Harness-native resume is a named exclusion** (PRD resolved q57 ruling b). Both
v1 harnesses can resume their *own* sessions from a store each vendor keeps, and
this runtime does not use one. Two invariants forbid it and both are §12's
subject rather than a preference: the journal is the complete hub state
(PRD resolved q44 invariant 1), so a recovery that depended on a directory
beside it would be recoverable only on the machine that crashed; and
`docs/distributed.md`'s redispatch hands a worker a node's journaled effect
history and expects it to replay to the frontier, which a session store on
another host cannot answer. The vendors' stores are debugging backstops.

**What the request identity holds**, which is what §7 compares on a resume: the
**whole binding**, as §3.2 means that word about an `exec:` call. The node, the
harness, the `model.*`, the id it resolved to, the model `settings:` that crossed
into the harness and the **connection** behind that model — its provider's
address, its `base_url:`, its credential and its `headers:`, each as written;
the run's instructions and its rendered input; the workspace **as the author
wrote it** — which since PRD resolved q61 is the *expression* the directory is
evaluated from, or the word `fresh`, rather than a path (`docs/grammar.md` §8.9,
Decision D147); the access preset, the tool allowlist, the declared `env:` —
names and values **as written** — and whether the run inherits the process
environment; the harness `settings:`; and the node's `output:` schema.

The workspace being the expression rather than its answer is the right identity
for the same reason the `env:` values are: a node repointed at another checkout
between a crash and its resume is a different run and diverges, while the same
expression resolving to a different directory on a different machine is not —
and a resume returns the recorded answer without re-entering the directory at
all. A `workspace: fresh` node is the clearest case: the directory it names is
derived from the instance path, so the dispatches a replay is catching up on are
told apart by the effect key rather than by anything in this request.

The connection is in the list for the same reason the schema below it is, one
turn sharper: a harness run pointed at a gateway and the same run pointed at the
vendor are two different runs, and nothing else in the binding says which one
happened. Repointing `providers.yml` between a crash and its resume therefore
diverges rather than returning an answer from an endpoint this build no longer
calls (`docs/grammar.md` §8.9, Decision D143).

The **resolved** environment is deliberately absent: those are the values
`docs/trace.md` §11.1 keeps out of every artifact this project writes, and the
workspace and the `env:` values go in unresolved for the same reason. What is
recorded is `${OPENAI_API_KEY}`, never what it resolved to — so one composition
derives one identity whatever machine it runs on, which is also why an execution
journaled on one host is not reported as divergent on another.

The `output:` schema is in the list for a reason worth stating on its own: **a
replay does not re-gate the answer.** A resume that finds this slot returns the
recorded output and never performs the run, so the gate that parses a harness's
answer against the node's contract does not execute. Were the schema left out,
narrowing `output:` and resuming would hand the graph an answer the current
contract refuses, silently and with no divergence. Editing a coder node's
prompt, its `env:`, or its `output:` therefore diverges a resume, which is the
answer §7 gives everywhere: the recorded answer is an answer to a question this
build is no longer asking.

## 4. Keys

An effect's journal key is

```text
<site> "#" <kind> "/" <ordinal>
```

* `<site>` is the effect site's instance path (`docs/grammar.md` §9.4),
  flattened the way `docs/trace.md` §8 flattens it — **without** the execution
  id, which is a column of its own. A node's activity uses the node's own path;
  a `map` dispatch uses the dispatch's, `<node>/<traversal>/<index>`; anything
  inside a `flow:` node or a flow-as-tool call uses that instance's.
* `<kind>` is one of `model`, `tool`, `store`, `human`, `harness` — §3's five,
  and the whole vocabulary a key can carry.
* `<ordinal>` counts effects of that kind at that site, from `0`.

`#` separates the two halves because it appears in neither: a node id is an
identifier (`docs/grammar.md` §2.1) and every other frame component is a
decimal.

**The ordinal is not `docs/grammar.md` §9.4's**, and the difference is
deliberate. An idempotency key is *positional*: a repeated attempt at one effect
reuses its key, which is what a receiver dedupes on. A replay has to reproduce
the **sequence**, the attempt that failed included — so a journal ordinal counts
every effect of a kind ever issued at a site **within one execution**, and never
restarts. A node whose first attempt's model call was refused and whose second
answered records two effects, and a replay consumes them in that order. The
counters live with the execution rather than with a node execution, which is
what makes that true across a `retry:` that re-runs a whole `map` fan-out or a
whole `flow:` instance.

Two properties follow, and they are what a replay depends on:

* **distinct effects get distinct keys**, because a site is distinct per
  instance and the ordinal is distinct per issue;
* **one execution replayed twice derives the same keys**, because everything
  between effects — routing, CEL evaluation, traversal ordinals, item indices —
  is deterministic. resolved q29 says this outright: that determinism "becomes a
  durability-correctness requirement".

## 5. Replay, and the frontier

A resumed execution **re-runs the graph from its entry**. It does not restore a
snapshot; there is none. What makes that cheap and safe is that every effect
site consults the journal first:

* the journal **holds** a record for this key → it is returned, byte for byte,
  and nothing reaches the network, the filesystem, a store or a person;
* the journal **does not** → this is the frontier. The effect is performed live
  and recorded, and every effect after it is live too.

"The frontier" is per key rather than global, which is what a graph with
concurrent branches needs: a `map` whose third item had run and whose fourth had
not resumes with three items replayed and the fourth live, and neither branch
has to know about the other.

**What is not consumed** is everything that is not an effect: the graph is
re-executed, so routers run again, guards are evaluated again, reducers fold
again, and traversal ordinals are assigned again. They must answer what they
answered — see §4's second property — and the failure mode when they do not is
§7.

**A live effect past the frontier acts on the world the recorded prefix left
behind**, and that is a real premise rather than a restatement. It holds for
every world an effect can reach *except one*: a store whose data lives in the
process. A `scope: execution` `kv`/`vector` store — and any store a target bound
to `provider: memory` — is an in-memory database opened per execution
(`src/stores.ts`), so its rows died with the crash; and because the prefix's
writes are answered out of the journal they are never re-applied. A live read
past the frontier would find an empty store, answer `found: false` about
something the execution wrote, route down a branch the original would never have
taken, and report `completed`. Nothing compares unequal, so neither divergence
in §7 sees it.

So it is **refused**: a live op on an in-process store whose recorded prefix
wrote to it fails the resume with a diagnostic naming the store and the effect
site, raised and travelling exactly as §7's divergences do — past every policy,
without closing the execution's row. The repair is the composition's: a store a
resumed execution reads across the frontier has to be `scope: session` or
`scope: global`. A `scope: execution` **`blob`** store is not affected; it is a
directory on disk, so its contents are still there, and the resumed run removes
the partition when it ends exactly as the crashed one would have.

A **dialled** store is not affected either, at any scope, and for the same
reason read one step further out: a `kv` store bound to `postgres` or `mysql`
(`docs/grammar.md` §14.3, PRD resolved q63) keeps its rows on a server, so
`scope: execution` fixes when they are deleted rather than where they live. The
generation that **ends** the execution deletes that partition, a generation that
only parked deletes nothing, and a resumed run reads exactly the world its
recorded prefix left behind.

A generation that only **parked** removes nothing. `agent-compose run` reaching
a `human` pause with nobody to answer it is not a crash — it exits `3` and
leaves the row open (§3.6) — so it lets go of what it holds *in this process*
and leaves what is on disk for the generation that finishes the execution. The
alternative is the failure above with nothing to catch it: a partition deleted
on the way out, a replayed `put` that is never applied again, and a live `get`
past the frontier answering `found: false` about something the record says the
execution wrote.

**Time is not replayed.** A `timeout:` budget runs against the resumed
generation's clock, and a `human` node's own budget restarts when the wait
re-parks — a wait the journal *holds* is not re-parked at all, and replays with
the instants the recording generation measured (§3.4). A backoff's jitter is
re-rolled. None of it changes what an effect answers, and a recorded effect is
answered out of the record whatever the clock says.

**Both kinds of wait re-park with a fresh budget**, including the one the journal
could date. A pause a worker settled its dispatch with
(`docs/distributed.md` §3.4) is on the dispatch row, so a hub that re-derives it
*knows* when the worker took the question — and does not spend that on the
timer, or on the date it publishes: the wait is planted again, planting is what
arms it, and **both** instants the planting publishes and later journals are that
arming's own, since a deadline a reader is shown has to be the deadline that
fires and a wait's `pausedAt` is when the generation holding it began holding it.
A local pause has no such row and could not do otherwise. The two therefore cost
a person's remaining time exactly the same and are dated the same way, which is
what PRD resolved q46 requires of them — "timeout, retry, on_error semantics …
are the single-process ones" — and a restart is downtime nobody could have
answered through, not budget somebody spent.

What a resumed generation's clock can change is *whether* a deadline fires, and
that is a change in the **shape** of the run rather than in the identity of any
effect. Replaying an attempt costs no wall clock, so a node whose `timeout:`
ended its `retry:` ladder on the recording generation can have budget left to go
round again on the resumed one. That next attempt claims a key the journal does
not hold — which is the frontier, exactly as §5 defines it — so the effect is
issued **live** and recorded, and the resumed execution has done something the
recording one did not.

That is not a divergence and is not reported as one: nothing compares unequal,
and neither does the reverse case, where a resumed generation's deadline fires
*earlier* and leaves records at that site unconsumed. It is the same
at-least-once compromise §2 states for an effect that answered with no row for
it, reached through a policy instead of through a crash, and it is bounded the
same way — a repeat carries `docs/grammar.md` §9.4's idempotency key wherever the
effect has one, which a store write and a detached dispatch do and an `exec:`
does not. A composition that cannot afford the repeat is one whose node should
not carry both a `timeout:` and a `retry:` over a non-idempotent effect.

## 6. Recovery: `serve` and `resume`

resolved q28 gives the two surfaces different defaults, for a reason it states:
"a CLI invocation ending is not evidence the user wants it re-run".

### 6.1 `serve` auto-recovers

On process start — in a Fastify `onReady` hook, so it completes **before the
server accepts a connection** — the app enumerates every execution the journal
holds open and re-runs each under replay. Three consequences a caller may rely
on:

* an execution parked on a `human` wait **re-parks under the same wait id**, so
  a `POST /executions/:id/resume` prepared against the process that died still
  finds its wait — and, because the row records which trigger started the
  execution (§3.5), that resume is verified against **that trigger's** `auth:`
  in the new process exactly as it was in the old one
  (`docs/grammar.md` §13.3, PRD resolved q32);
* every delivery the journal holds `pending` is **picked up** (§3.7), beside the
  executions and separately from them: a delivery reports on an execution that
  may have ended in the process that died, and its remaining schedule is
  computed from the recorded intent rather than started again — each row on its
  own, so a read that fails while they are enumerated leaves **one delivery**
  owed rather than an `onReady` hook that rejects and a process that never binds
  its port. What failed is written on stderr and the row stays `pending` for the
  next start, which is §3.7's posture for a webhook and this section's for a
  journal it cannot read;
* an execution that re-parks under wait ids a `parked` delivery already reported
  fires **no** parking webhook: re-parking is what recovery is, and nothing was
  asked that had not been asked (PRD resolved q35);
* the status route answers for a recovered execution exactly as it answers for
  one this process started, and a recovered execution that finishes **delivers
  the `settled` webhook** its request asked for — the URL is on the lifecycle
  row (§3.5), because a caller who was handed a `202` and is waiting for a push
  is not polling the status route. *Finishes* is the word: the webhook fires on
  exactly the outcomes that **close the row**, so a replay that leaves the
  execution open — a divergence (§7), a pause nobody can answer, a start that
  refused the recorded invocation before it opened anything — pushes nothing and
  the process that eventually closes the row is the one that delivers, once. It
  is decided by **reading the row**, not by classifying the error: a recovery
  can fail before the execution is opened at all — inputs this build's `inputs:`
  no longer accepts, a `session_key:` it has since started requiring, a journal
  written by another compiler release (§11) — and every one of those leaves the
  row open while looking like an ordinary failure. A caller is told an execution
  failed only where the run really ended;
* recovery **does not wait** for the replays to finish. The executions it
  recovers are by definition ones that were still running, and the commonest of
  them is parked on a question nobody has answered yet. Registering them is what
  has to happen before the first request; finishing them is what the resume
  route is for.

Because it does not wait, a resume can arrive while the replay is still on its
way back to the wait. That request is refused with a refusal **of its own** — a
`409` carrying `recovering: true` and a sentence that says to send it again —
rather than with the one that means there is no pause here, which is what a
*settled* wait is refused with and reads as final.

The window is per **pause**, not per execution. One execution can hold several
(grammar 8.6) and its branches reach them independently — a branch whose
recorded prefix the crash left an effect of has to run that effect live before
it re-parks, while a branch whose prefix is whole is back at once — so the first
pause published says nothing about the second. A request that **names** a wait
the board does not know is inside the window for as long as the replay runs,
because a pause stays on the board once it opens: an id the board has never held
is one this generation has not reached. A request that names no wait is inside
it only while the board has held nothing at all. Either way the window closes
when the run ends.

A replay that fails is recorded on that execution and reported by the status
route, and recovery of one execution never stops the process from serving the
others. A **divergence** (§7) is reported the same way and leaves the journal
row **open**: what this process reports is what this build saw, while the row
records the execution. An open execution of a flow the current build no longer
declares is reported on stderr and left open for the same reason.

Because recovery takes every open execution, an execution worth resuming by hand
is one no `serve` is running — see §2 on what is outside the promise.

### 6.2 `run` journals; `resume` replays

```text
agent-compose resume <path> <execution> [--target <name>] [--out <dir>]
                                        [--format human|json]
```

It builds the project exactly as `run` does and then replays one execution to
completion. It takes **no `--input` and no `--session`**: the invocation it
replays is the one the lifecycle row recorded, and a resume that took different
inputs would be one execution's record replayed into another execution's run —
the divergence resolved q29 refuses.

It composes with the interactive surface (`docs/grammar.md` §8.7, PRD §9.21): a
resumed execution that reaches a wait the journal does not hold re-parks, and a
terminal — or `AGENT_COMPOSE_INTERACTIVE=1` — answers it there, with the same
prompts, the same schema check and the same refusals a `run` uses. Its exit
codes are `run`'s: `0`, `1`, `2`, and `3` for a resumed execution that reached a
pause with nobody to ask.

**And it settles a `serve` execution's callback.** An execution an `http` trigger
started carries the webhook its request asked for on the lifecycle row (§3.5),
and this command can be the process that reaches the end of one — a `serve` that
died, an operator finishing it by hand. So when the row closes here the `settled`
intent is journaled first, under the next ordinal, exactly as §3.7 requires of
the process that closes a row. It is **recorded rather than sent**: the schedule
outlives this command, so the next `serve` start delivers it (§6.1). A resume
that re-parks, or that meets a divergence, journals nothing — the row stays open
and the settle has not happened.

**Where the execution id comes from.** A `run` prints it on stderr as its first
line, before anything can fail:

```text
execution: exec_9f1c…
```

`--format json` carries it as `execution_id` on both of a run's ways out, and
`serve` answers it as `execution_id` and logs `recovered <id> (<flow>)` for each
execution it puts back.

**What `resume` refuses**, each by name (PRD G3):

| situation | what it says |
|---|---|
| no journal | this project has never journaled an execution, naming the path that does not exist |
| unknown id | the id is not one this journal holds, and the executions it does hold open |
| already ended | the execution has already `completed` or `failed`, when, and why re-running it is not what a reader wants |
| flow no longer declared | which flow the execution was running and which flows this build has |

### 6.3 Triggers fire once

Nothing in recovery reads the lifecycle row's `trigger` as an instruction. A
recovered `http` execution is the one that existed; no request is re-delivered
and no schedule is re-armed.

## 7. Divergence

A replay that reaches a recorded effect it cannot honour **fails the resume**.
resolved q29: "a divergence discovered at replay time […] fails the resume with
a diagnostic naming the divergent step, rather than silently re-executing an
effect the journal claimed to hold."

**Two things** can be that. The first is the effect's *request identity* — §3's
per-kind descriptions say what goes into each — canonicalized as JSON with
object keys in sorted order, so two identical requests cannot compare unequal
for having been built by different branches of one function. It is compared at
the effect seam, before anything is answered.

The second is q29's other clause: **a recorded answer that fails the current
contract.** A composition may keep a binding exactly as it was and tighten what
it will accept back — a `min_length:` added to a field, an `enum:` narrowed —
and the recorded answer then satisfies the request check and fails the node's
declared `output:`. That is decided where the answer is parsed rather than at
the seam, because the contract belongs to the node: at the node's result parse
for a model or tool answer, and at the replayed wait itself for a `human` one
(§3.4), which is the single kind that reaches no result parse. It is a divergence
rather than the ordinary "this answered off-contract" failure for the reason
below: an ordinary one is a node failure, a `retry:` absorbs it, and the second
attempt claims an ordinal past the frontier and **re-issues the effect live** —
the double side effect this whole document exists to prevent, reported as a bad
answer.

One case sits inside that and is **not** a divergence: a recorded answer can
fail a contract that has not moved at all, because it failed it on the
generation that recorded it too — a flaky `exec:` under a `retry:` whose first
attempt answered off-contract and whose second did not. That generation's own
ladder decided it, and a resume has to do what it did rather than call the
disagreement new.

**The record says which happened, because the generation that refused the answer
wrote it down.** A parse that refuses a live answer marks that answer's record
`refused` (§3) as it raises its mismatch — and, since the parse is synchronous
and the mark is a write, the ordering §3 states is the load-bearing one: the
mark is down before the record of the attempt that mismatch set off. A resume
that meets a refused record raises the ordinary mismatch too: the ladder does
now what it did then, and the attempt it spends is a replay rather than a call.
A record with no mark is one this build is the first to refuse, which is the
divergence.

Nothing is inferred from the records *around* it, and two readings that tried to
be are both wrong for one reason — what the ordinals hold is the **sequence** of
effects at a site, and "the site retried this call" and "the site made this call
again" are the same sequence:

* **counting** — treating the next record at the site as the retried attempt —
  is wrong wherever a site issues two or more effects of one kind: an agent
  whose loop calls `tool.alpha` and then `tool.beta` records `…#tool/0` and
  `…#tool/1`, and a tightened contract on *alpha* would find beta's record
  sitting where a second attempt would have been;
* **counting plus request identity** — requiring that next record to repeat this
  one's request — is wrong wherever a site legitimately issues the same request
  twice, which a model's loop does whenever it looks something up again. The
  divergence would be downgraded to a mismatch, and an `on_error:` or a `retry:`
  would absorb what q29 says no policy may absorb.

**What the failure names** is the divergent step: the effect site's instance
path, the effect kind, the ordinal at that site, the whole key, and — for a
request identity — both sides of the disagreement truncated to 200 characters
each. A recorded answer that failed the current contract names what the contract
said instead.

```text
ReplayDivergence: this execution's journal does not describe this run at
`review/0` (model effect #1, key `review/0#model/1`): the journal recorded a
request this run does not make. Recorded: {…}. This run: {…}
```

A `ReplayDivergence` is deliberately **not** a node failure. It is raised past
the node's `retry:` ladder and past its `on_error:`, because a composition's
error policy decides what to do about the world misbehaving and a journal that
does not describe this graph is not the world misbehaving — `skip` would carry
the run past an effect the record claims to hold, and a `fallback:` would route
on a disagreement rather than on anything the composition declared. Retrying it
would be worse still: each attempt would consume the next ordinal at that site
and report the last disagreement rather than the first.

**No policy at any nesting depth absorbs it**, which is q29's own phrasing.
There are four ladders and policies a failure can be absorbed by, and each lets
this one through: a node's `retry:`, a node's `on_error:` (`skip` and
`fallback:` alike), a `map`'s `on_item_error:`, and the item retry the `retry`
form of that key gives each item. The two boundaries a failure is *restated* at
— a `flow:` node's instance and a dispatched item — are looked through rather
than tested for, so a divergence inside a subflow, inside a `map` item, or
inside a flow-as-tool child is the same divergence when it arrives. `detach:` is
the fifth and is §3.2's.

The resume exits nonzero, the trace entry of the node it stopped at is written,
and **the execution stays open**. That last is not a detail: a divergence says
this build's composition and this execution's record disagree, which is not a
statement about the execution. Recording it as the execution's own failure would
make it unresumable for ever — `resume` refuses a `failed` row by name — even
after the composition is put back, and since `serve` replays *every* open
execution at start (§6.1), one deploy that moved a prompt would close every open
execution of that project in a single restart.

The commonest cause is a composition that moved under a journal — a changed
prompt, a renamed tool, a different `url:`. That is the diagnosis the message is
written for.

## 8. Privacy posture

**The journal is private recovery data with the same sensitivity as this
project's stores.** It holds, by construction:

* model completions in full;
* what a model sent a tool, and what the tool answered;
* the **whole event stream of every coding-harness run** (§3.9) — its turns, its
  tool calls, its reasoning items and the transcripts of any subagents it ran,
  which is the largest single payload this journal holds;
* what a store read and what it wrote;
* **what a person answered a `human` node**;
* the **body of every callback delivery** (§3.7), which is the status route's
  report and so carries an execution's outputs and its open questions. Kept
  because a retry must send the bytes the first attempt signed, and no longer:
  what `callback_auth:` resolved — the token a delivery carried, the key it was
  signed with — is never written down, and neither is the request the callback
  URL was read out of.

`docs/trace.md` §11 keeps every one of those out of the trace, and this document
does not weaken that rule by a word: the two artifacts are separate files with
separate contents, and nothing here is derived from the trace or written into
it.

So:

* it is **never** an observability artifact. Nothing ships it anywhere, no
  command prints it, and no field of the trace format carries any part of it;
* it lives under the project's data directory with the stores, and inherits
  whatever protects them;
* deleting it is deleting the file, and the cost of doing so is exactly that
  open executions can no longer be resumed.

A composition whose `${ENV}` values reach a request identity (§3.2 keeps a
binding's own references unresolved, but an *input* the graph built may hold
anything the composition put there) has put them there itself, which is the
standing `docs/trace.md` §11.1 gives `StoreRecord.answer`.

## 9. What the trace of a resumed execution looks like

A resumed generation writes a **fresh trace document**, whole, as if the
execution had run in one process:

* its `execution_id` is the original execution's, so the two generations'
  documents are joinable;
* every identity in it — node paths, traversal ordinals, instance paths,
  idempotency keys, wait ids — is the same one the crashed generation derived,
  because all of them are deterministic (§4);
* a replayed model call carries the `ModelCall` records the original ladder
  filed (§3.1), a replayed store op carries the `StoreRecord` the original op
  filed (§3.3), a replayed harness run carries the `HarnessRecord` the original
  run filed — the attempts that failed included (§3.9) — and a replayed wait
  carries the original `pausedAt`, `expiresAt` and `settledAt` (§3.4). A reader
  of the resumed document sees what the execution did, not what this process
  did.

**Replayed and live entries are not distinguishable** in the trace, and that is
a decision rather than an omission. `docs/trace.md` §10.3 requires a version
bump for "adding a field to an existing record type" only in the sense that
§10.2 makes it compatible — but a `replayed: true` flag would be a new
presence rule a reader would have to learn, on every record type at once, to
express something no consumer of the trace has asked for: the trace answers
"what did this execution do", and the answer is the same whichever process did
it. A reader who needs the other question — "what did *this process* do" —
reads the journal, which timestamps every record.

**`TRACE_VERSION` is unchanged at `4`.** Nothing in `docs/trace.md` moved: no
field was added, removed or renamed, no presence rule widened or narrowed, no
closed enumeration gained a member, no fixed order changed, and neither the
derivation of an idempotency key nor where instance paths appear moved (§10.3's
list, item by item). The journal is a separate artifact with a version of its
own, which is the whole reason it can carry what §11 forbids the trace.

## 10. Backends, and what binds what

resolved q27 makes the journal a **deploy-target slot**, exactly as
`storage_backends` are: "the composition says nothing, the target binds it".
resolved q62 fills it. The slot is `docs/grammar.md` §14.7's `journal:` block,
and it takes three providers:

| `provider:` | where the record lives | `url:` | driver |
|---|---|---|---|
| `sqlite` | `<project>/.agent-compose/journal.sqlite`, opened in this process | takes none | `node-sqlite3-wasm` |
| `postgres` | a Postgres server this process dials | REQUIRED, `${ENV}` only | `pg` |
| `mysql` | a MySQL server this process dials | REQUIRED, `${ENV}` only | `mysql2` |

What does **not** vary:

* **journaling is unconditional.** Every invocation of every flow is journaled,
  with no key to turn it off and none to turn it on. What §14.7 chooses is where
  the record goes, never whether there is one (Decision D121).
* **the interface is one interface.** Everything in this document but §2 is
  backend-invariant: the record vocabulary, the keys of §4, the frontier of §5,
  the recovery verbs of §6, divergence, the two ledgers, `JOURNAL_VERSION`. The
  runtime cannot tell which backend it got, and there is one implementation of
  these statements rather than three.
* **the default is stated, everywhere.** A target that declares no `journal:`
  binds SQLite — `local` always, and every named target until it says otherwise.
  Zero configuration is the point, and it is not a local-only concession
  (resolved q27, q62).
* **`deploy/local.yml` may not declare the block**, exactly as it may not declare
  `storage_backends:` (Decisions D87, D148).

**What a remote journal buys** is the property resolved q27 could not give a
single host: the hub's record survives the hub's disk, so a `serve` restarted on
a fresh machine recovers every open execution from the database. §12's
cross-process exclusion is therefore narrowed rather than lifted — it is about a
**SQLite** journal, which is a file on one machine, and it always was.

**What it costs.** An operator's `DELETE` instead of one file to delete (§2.3);
a decision about where the payloads of §8 live, since a journal holds what a
trace deliberately does not; and a dependency, which the target that binds it
carries and no other does — a project whose journal is SQLite pins neither `pg`
nor `mysql2` and carries neither arm in `src/journal.ts`.

**The SQLite driver is `node-sqlite3-wasm`** — the one `src/stores.ts` already
opens. PRD §9.18 makes Node a supported fallback beside Bun and gates it
statically over the whole golden corpus, so a generated module may name neither
`bun:sqlite` nor `node:sqlite`; the shared driver is also one fewer pinned
dependency.

**Schema creation is idempotent, and only SQLite migrates.** Every backend
creates its tables on first open with `IF NOT EXISTS`, so a second open of a
journal this release wrote does nothing. SQLite additionally *probes* for the
columns §11.2 records as later arrivals, because files written by earlier
releases exist; the two remote arms carry no such probes and need none, since no
journal older than they are exists. A column added to a remote schema in a later
release is the case to read §11.2 twice for, exactly as it is on the file.

**A column holds the same values on all three.** The types differ — that is what
a per-backend schema is for — but what fits does not. SQLite and Postgres spell
every string column `TEXT`, which is unbounded on both. MySQL cannot: its `TEXT`
is 64 KiB, and it will not index a column with no bound at all. So its schema
splits the columns in two, and the split is the rule to read before widening any
value this project writes.

* **A value the caller decides the size of is `LONGTEXT`** — every payload,
  every request, an execution's `inputs`, a delivery's `body` and `attempts`, a
  dispatch's `inputs`, `history` and `policy`, a node address, a placement, and
  an instance `site`. So is `executions.error`, which is a provider's whole
  failure body or a harness run's quoted transcript: a `failed` outcome the
  server refused would leave the lifecycle row `open` for ever, so every later
  `serve` start would re-recover an execution that has already finished.
* **A value whose shape this project fixes is a bounded `VARCHAR`**, and there
  are four bounds. `VARCHAR(2048)` on `effects.key` and `dispatches.wait` — the
  halves of a compound primary key that are not fixed-shape ids, and the pair
  whose arithmetic has to stay inside InnoDB's 3072-byte index limit.
  `VARCHAR(255)` on every id the runtime mints, which is a four-character prefix
  and a UUID: `executions.id`, `effects.execution`, `deliveries.execution`,
  `dispatches.execution`, `dispatches.id` and `dispatches.session`.
  `VARCHAR(16)` on the `status`, `kind`, `event` and `outcome` columns, each a
  closed set of words this compiler emits. `VARCHAR(32)` on every instant, which
  is an ISO-8601 string of 24.

The two classes meet at one seam worth naming: a `site` is `LONGTEXT` while the
`effects.key` and `dispatches.wait` derived from it are bounded at 2048 bytes, so
an instance path long enough to matter is refused at the key rather than at the
site. §4's keys are `<site>#<kind>/<ordinal>` over an instance path, and 2048
ASCII bytes of path is the depth this schema supports.

**Widening one of those values is a change to the MySQL schema too**: a longer
session id, an instant in another format, a fifth status word past sixteen
characters. MySQL is the backend that refuses the write; SQLite and Postgres
take it, which is what makes the divergence worth stating here rather than only
in the schema. The refusal is a refusal rather than a silent right-truncation
because the MySQL arm sets `STRICT_TRANS_TABLES` on its session — MySQL 8 ships
it on, a managed or legacy server with `sql_mode=''` does not, and a truncated
`effects.key` would be two effects collapsing onto one primary key and a replay
handing the first one's answer back at the second one's site.

**The same session clears `NO_BACKSLASH_ESCAPES`**, which is the mode in the
other direction: the arm binds its parameters through `mysql2`'s `query`, which
escapes them on the client with backslashes, so a server that reads a backslash
as an ordinary character would refuse any payload carrying an apostrophe and
store every JSON payload with its escapes left in — a write refused mid-run on
one backend, or a `JSON.parse` that throws on replay. Both directions are the
one rule: a session setting this journal's statements rest on is stated by the
arm rather than assumed of the server.

**That rule reaches past `sql_mode` to what "committed" means**, which is §2.1's
promise rather than a schema detail. MySQL's `autocommit` and Postgres'
`synchronous_commit` decide whether a statement is its own transaction and
whether a commit is a write; neither driver sends either, and both are
configurable per server, per database and per role. So each arm sends its own —
`SET SESSION autocommit = 1`, `SET synchronous_commit = on` — before any record,
and a server that refuses is refused the journal. §2.3 says what each costs when
it is missing. What makes them belong beside the modes above is that they are
invisible to every case the conformance suite can run: CI's containers ship both
the right way round, and a MySQL journal with `autocommit` off would report every
case in the suite green and hold nothing.

Key ordering and case sensitivity are stated per backend in the same place:
SQLite's `BINARY`, Postgres' `COLLATE "C"`, MySQL's `ascii_bin`, all of which
make §4's key order the order a reader derives. That binding covers every column
a statement compares, not only the keys — `dispatches.session` is compared by
`releaseDispatch`, so a case-insensitive collation there would let one worker put
another worker's in-flight dispatch back on the board.

**How each backend is proved.** The contract of this document is a conformance
suite — `crates/compose-core/tests/journal_backend_conformance.rs` — which drives
the **same** cases against SQLite always, and against each remote provider when
its server is reachable. CI runs all three against real servers as service
containers, so "CI green" keeps meaning "actually done" for a backend whose
failure modes are a network's (PRD resolved q62, CLAUDE.md's Validation
strategy). Both halves of §2.3's guard are among the cases: a *live* second
opener is refused by name, and a **dead** one is not — the suite ends the
holder's session from another connection, which is the path the server's own
reap takes without the five-minute wait, and then requires the takeover to be
admitted and the process that lost the connection to still be running.

**Every verb of the interface is driven**, and that is held by a test which reads
the interface off the module rather than off a list beside it. An undriven verb
is a `WHERE` no server ever answers: each of `refuseRecorded`, `exhaustRecorded`,
`refuseDelivery` and `supersedeDispatch` is a settle predicate whose whole job is
to be exact, `refuseRecorded`'s is the only comparison of `deliveries.kind`
anywhere in the journal, and a placeholder mis-numbered in any of them shifts
every later parameter on Postgres, where `?` is counted positionally into `$n`.
The settings §2.3 states on each session are the complement of this: they are
what no case can see, because a server that has them is the only kind CI runs.

**Durability is not checkpointing.** `docs/grammar.md` §14's rule that "`local`
is not durably checkpointed; every other target is" — the rule
`unsupported-detach` keys off — is about a LangGraph **checkpointer**, which
resolved q26 rules out as this project's durability mechanism. The two are
orthogonal: `--target local` is still the un-checkpointed target, `detach: true`
is still legal only there, and it is now also a durable target.

## 11. Stability

`JOURNAL_VERSION` is what a reader pins, and every lifecycle row carries the
version it was written at. A journal written at a version this build does not
read is refused by version, naming both, rather than misread.

The rules are `docs/trace.md` §10's applied to this artifact.

### 11.1 What a reader may rely on

At a given `JOURNAL_VERSION`:

* every field this document names, under the name and with the meaning given
  here;
* the key derivation of §4, and the vocabulary of `EffectKind`;
* the `status` vocabulary of §3.6, the `DeliveryEvent` and `DeliveryStatus`
  vocabularies of §3.7, and the dispatch `status` vocabulary of §3.8;
* the delivery id derivation of §3.7 — `<execution_id>:<ordinal>` — and that a
  retry of one delivery carries the id and the metadata its first attempt did;
* that a record's payload round-trips: a value written by one generation is the
  value the next one is handed. Both ends of that: a payload is stored as
  canonical JSON, so the generation that *recorded* it goes on with the round
  trip rather than with the value it happened to build — otherwise the two
  generations would carry differently ordered copies of one value, and the
  difference would surface as a divergence the composition never made the moment
  either was folded into a later request identity;
* that replay is read-only up to the frontier (§5), and that a divergence fails
  the resume rather than re-executing (§7).

A reader MUST NOT rely on the text of the divergence diagnostic (§7), which is
written for a person and is improved between releases, nor on the physical
schema any backend holds the record in — the SQLite file's tables, or the ones a
remote journal creates on first open — which is this implementation's rather than
this format's. Nor on **which** backend a deployment bound: §10 makes that the
target's, and every promise in this section is the same behind all three.

### 11.2 What is a compatible change

Made **without** a version bump: adding a field to a record type; adding a new
`EffectKind` whose absence in an older journal is simply a frontier; improving
the text of a diagnostic; changing the physical SQLite schema in a way that
reads older files.

The **delivery ledger** (§3.7) arrived under that last clause, and it is worth
saying why rather than leaving it to be inferred from the version this document
still heads with. A journal written before it opens unchanged — the table is
created on first open, as `CREATE TABLE IF NOT EXISTS` — and an execution open
in such a file replays *identically*, because a delivery is not an effect, holds
no §4 key, and is never consumed by a replay: nothing about the frontier moves,
which is the failure §11.3's last bullet is about. What such an execution does
not have is a record of the deliveries an older build never made, so its first
parking under this build announces the pauses it is holding. That is a webhook a
receiver dedupes or ignores, not a replay that re-issues an effect.

That claim is **executed rather than asserted**: `crates/agent-compose`'s
`a_journal_written_before_the_delivery_ledger_opens_and_serves_under_this_build`
strips the `deliveries` table and the lifecycle row's `callback` column from a
real journal — the file a build before this one wrote — and restarts `serve`
over it, which is the only way to find out that a compatible change stayed
compatible. A column added to an existing table is the case that is *not*
covered by `CREATE TABLE IF NOT EXISTS`, and needs the `PRAGMA table_info` probe
the migrations beside it use. Three columns have arrived that way and each is
nullable for the same reason — no file written before one says what it would have
held: the delivery's own `trigger`, the delivery's `kind` (a row written before
it is a `callback`, which was the only kind there was), and the lifecycle row's
`traceparent` (an execution started before it carried no header this file could
hold, and one started without one carries none either).

**The dispatch board (§3.8) arrived under the same clause and owes the same
test**, and it has one: `a_journal_written_before_the_dispatch_board_opens_and_serves_under_this_build`
strips the `dispatches` table from a real journal and starts the hub over it.
The argument is the delivery ledger's, made about the other ledger: the table is
created on first open, a dispatch row holds no §4 key and is never consumed by a
replay, so an execution open in a file written before the board replays
identically and its frontier does not move. What such an execution does *not*
have is a row for a placed node its own generation never reached — and reaching
one under this build parks a fresh wait rather than finding a settled answer,
which is the same thing a `retry:` does and not a re-issued effect. The clause a
future change here has to read twice is the one above: a **column** added to
`dispatches` is not covered by `CREATE TABLE IF NOT EXISTS` either.

**The third of them is the `paused` dispatch outcome** (§3.8,
`docs/distributed.md` §3.4), and it arrived under no clause at all — it is a new
*shape* of an existing field, a settled row whose `outcome` carries a pause
instead of an output, which is none of the four literally. It needed no bump
anyway, and the direction is the whole of why: §11.3's criterion is a journal
written by the **previous** version replaying *wrongly* under this one, and no
such journal holds a paused outcome. Every settled row an older build wrote
carries an output or an error, and this build reads both exactly as its
predecessor did. The hazard runs the other way — an older build meeting a paused
row it has no reading for — and that is the case §11's opening sentence already
answers: a journal is refused by version, and a build is never asked to read a
file a *newer* one wrote.

That is the difference from the wire, and it is worth stating beside the bump
rather than leaving a reader to derive it. `docs/distributed.md` §10.4 bumps
`PROTOCOL_VERSION` to `2` for this same shape, because a peer is met **live**: a
worker of this release can hand a hub of `1` a result that hub will read as a
node which answered nothing, and no refusal stands between them until the version
says so. A journal has one reader per file and one arrow of time, so the same
new shape is compatible here and breaking there.

### 11.3 What requires a version bump

`JOURNAL_VERSION` MUST be incremented for any change that would make a journal
written by the previous version replay **wrongly** rather than fail:

* changing the key derivation of §4, or the ordinal discipline behind it;
* changing what an effect's request identity is composed of, or how it is
  canonicalized;
* changing what a record's payload means, or how it is revived;
* removing an `EffectKind`, or a `status` value;
* changing which sites are journaled at all, in either direction. Journaling
  **more** is as much a bump as journaling less: an older journal would be
  missing keys the new build derives, and the frontier would open in the middle
  of work the old generation had already done.

A bump is a statement that executions open in an older journal cannot be
resumed by the new build, and the refusal at §3.5's `journalVersion` is what
enforces it.

## 12. Out of v1 scope

Stated by resolved q28, and restated here so a reader is not left to infer it:

* **cross-process migration on a SQLite journal** — a file written by one host
  and resumed on another is M3-distribution's problem, and nothing here promises
  a file is portable. A **remote** journal is the narrowing resolved q62 makes:
  the record is on a server both hosts dial, so a `serve` restarted on a fresh
  machine recovers every execution the one that died left open (§10). The
  exclusion is about the backend rather than about the format.
* **journal compaction** — nothing is pruned. The file grows with the effects a
  project has ever issued, exactly as the stores' idempotency ledger does, and
  retention is deleting it.

And one this document states rather than defers, because it is a property of the
backend v1 binds rather than a feature left out:

* **two processes on one project's journal at once.** Every backend's guard
  keeps them out of each other's writes (§2), but a guard is a lock rather than a
  plan: neither process can see what the other's replay is doing past the
  frontier, so one process at a time writes a project's journal —
  `agent-compose resume` beside a live `serve` included.

  What a second process *meets* differs, and is worth knowing. On **SQLite** a
  held lock is broken after a deadline (§2.2), so a second process does not
  merely queue: it eventually goes in. On **Postgres and MySQL** the session-scoped
  guard is refused outright and the second process is told so by name (§2.3) —
  which is the stronger behaviour, and is possible only because the server drops
  that lock when a connection ends and a dead owner therefore holds nothing.

Three more, all consequences of §5 rather than deferrals:

* an effect that happened with **no row** for it (§2) is re-executed on replay;
* a resumed execution's deadlines are the **resumed generation's** (§5). A node
  budget that ended a `retry:` ladder on the recording generation may not end it
  on the replayed one, where the recorded attempts cost no wall clock, so a
  resume can spend an attempt the original never spent. What that attempt issues
  is a live effect past the frontier rather than a re-issue of a recorded one,
  and nothing about it compares unequal — so it is not §7's divergence and is
  not reported as one;
* an execution that reads a **store whose data lives in the process** across the
  frontier cannot be resumed (§5). Nothing reconstructs the rows a
  `scope: execution` `kv`/`vector` store held, so the resume is refused rather
  than answered out of an empty one. Making such a store durable is the same
  problem as journal compaction — it needs a lifetime that outlives the run — and
  the composition-level answer is `scope: session`.

---

Normative source: this document, for the journal a compiled project writes.
`docs/grammar.md` §9.4 is normative for the instance paths it keys off, and
`docs/trace.md` §8 for where those paths appear.
