# agent-compose — Execution Journal

**Journal version:** 1
**Status:** Normative for the journal a compiled project writes and the replay it reads back
**Companion artifacts:** [`docs/trace.md`](trace.md) (the keying this shares), [`docs/grammar.md`](grammar.md) §9.4 (idempotency keys), [`prd.md`](../prd.md) §5.8, §5.11, resolved questions 26–29

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
what replay executes — "read-only up to the frontier".

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense.

**Where the fields are implemented.** Every record type here is a TypeScript
interface in the emitted `src/journal.ts`, which is byte-identical in every
project a given compiler release builds — like `src/runtime.ts` and
`src/stores.ts` beside it.

---

## Table of contents

1. [The journal is not the trace](#1-the-journal-is-not-the-trace)
2. [Where it lives, and what a crash can leave](#2-where-it-lives-and-what-a-crash-can-leave)
3. [What is recorded](#3-what-is-recorded)
4. [Keys](#4-keys)
5. [Replay, and the frontier](#5-replay-and-the-frontier)
6. [Recovery: `serve` and `resume`](#6-recovery-serve-and-resume)
7. [Divergence](#7-divergence)
8. [Privacy posture](#8-privacy-posture)
9. [What the trace of a resumed execution looks like](#9-what-the-trace-of-a-resumed-execution-looks-like)
10. [Backends, and what v1 binds](#10-backends-and-what-v1-binds)
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

One SQLite file per project, beside the project's stores:

```text
<project>/.agent-compose/journal.sqlite
```

`AGENT_COMPOSE_DATA_DIR` moves the whole directory, exactly as it moves the
stores. The path is derived from the emitted project's own location rather than
from the process's working directory, and it is **stable across `run`, `serve`
and `resume`** of one project and one target — which is what makes
`agent-compose resume <execution>` find the execution a crashed `run` left
behind. Retention is deleting the file (resolved q27: "one file to delete").

**Atomicity is per record.** One effect is one `INSERT`, which SQLite runs in an
implicit transaction of its own, so a process that dies mid-write leaves the row
absent and never half present: nothing in the file can parse as a complete
record that is not one. `PRAGMA synchronous = FULL` is what makes "committed"
mean "on the disk". SQLite's **rollback journal** is what backs that; a
write-ahead log is deliberately not asked for, because this driver's virtual
file system does not implement one — the pragma is accepted and leaves the mode
at `delete`, so asking would be a line that reads like a guarantee and is not
one. Atomicity per statement is the same either way.

**Concurrency.** A `serve` process runs many executions at once, and every
statement here is synchronous: the driver blocks the event loop for the duration
of a call, so two executions can never interleave inside one statement and no
intra-process locking is needed.

Across *processes* this release keeps the boundary PRD 5.10 draws and
`src/stores.ts` already keeps — `--target local` is one process. **One process
at a time writes a project's journal**, and that stays a rule rather than a
promise; what the driver adds beneath it is a real lock. `node-sqlite3-wasm`
takes SQLite's exclusive lock by creating `<file>.lock` as a directory and gives
it back by removing it, so a second process on one journal meets `SQLITE_BUSY`
rather than interleaving into pages the first has not committed. `PRAGMA
busy_timeout` is set before any statement that can contend, and the open is
retried under a deadline for the statements the pragma does not cover.

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
there are exactly six of them, in four kinds:

| kind | site in the emitted project | what the record holds |
|---|---|---|
| `model` | `callModel` in `src/runtime.ts` | §3.1 |
| `tool` | `runExec`, `runHttp`, `callFunction` in `src/runtime.ts` | §3.2 |
| `store` | `runStoreOp` in `src/stores.ts` | §3.3 |
| `human` | `runHuman` in `src/runtime.ts` | §3.4 |

Three constructs record nothing of their own, and need none: a `flow:` node, a
`map` dispatch, and a flow-as-tool call are *instantiations*, not effects. What
they run is a graph whose nodes reach the four sites above under the
instantiation's own instance path — which is exactly how they trace
(`docs/trace.md` §8) — so their effects are journaled without a record for the
boundary itself.

That inventory is held **mechanically**, and by two tests in
`crates/compose-core/src/codegen/journal.rs` that read it from opposite ends.

`every_effect_site_reaches_the_journal_and_the_document_names_them_all` reads it
from the seams: each of the six functions above is read out of the emitted
modules, and the test fails when one does not reach the journal, when this table
does not name it, or when a seventh site exists that this table does not.

`nothing_in_the_emitted_runtime_calls_the_world_except_under_a_journaled_seam`
reads it from the **primitives**, which is the direction the first cannot see.
A surface added to `src/runtime.ts` that calls the world and is not journaled is
a replay that issues it twice — and it is not one of the six, contains no
`journaled(` and no `.claim(`, and is in no table, so nothing else in the
repository would notice. So every `fetch`, every `spawn`, every filesystem call
and every SQLite statement in the two emitted modules is required to sit in a
function the six transitively reach. One site is exempt and is named in the test:
`releaseExecution`, which is grammar 11.1's `scope: execution` lifetime rather
than an effect the graph issues.

### 3.1 A model call

One record per call `callModel` made, whichever way it ended. It holds the
answer the loop accepted **and** every `ModelCall` record the ladder filed on
the way to it — the failovers a route spent (`docs/trace.md` §7), and the
refusal that ended a call no member answered. Both halves are needed: without
the answer a replay cannot continue, and without the records the resumed
generation's trace would say the node called no model at all.

A call that **failed** is recorded as a failure and replays as one: the resumed
generation re-raises an error with the recorded class name and message, so the
node's own `retry:` ladder and `on_error:` decide exactly what they decided
before. What a replay cannot restore is the platform `cause` chain beneath it,
which `docs/trace.md` §11 already keeps out of the trace and which only the
human report prints.

The **request identity** (§7) is the `model.*` addressed, the system prompt, the
conversation as it stood, the tool names offered, and the pinned tool where
there is one.

### 3.2 A tool execution

One record per `exec:`, `http:` or `function:` invocation, at either of the two
surfaces a tool has — a node's own activity, and a tool an agent's model called.
The record holds the value the binding answered, before the declared `output:`
schema parses it; a failure is recorded and replayed as §3.1 replays one.

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

A **detached** `map` delivery (`docs/grammar.md` §8.6 rule 7) is journaled like
any other effect under the dispatch's own instance path. Its outcome is never
observed by the join (Decision D94) and nothing about the trace changes; what
the record buys is that a replay does not deliver it twice. A divergence raised
inside one is the exception to rule 7's "nothing it does can delay the enclosing
flow instance" — §7 makes a divergence un-absorbable by any policy, and
`detach:` is a policy — so it is held against the execution and fails it: at the
next effect any branch of the run reaches, or on the way out of `runFlow` for a
run with none left. What the run does not do is *wait* for a detached delivery,
which rule 7 forbids: a divergence raised after the run has already quiesced has
no run left to fail, and is written to stderr instead.

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

An **unsettled** wait records nothing, and that is the whole of re-parking: a
resumed execution reaching a wait the journal does not hold parks under the same
wait id — the id is the node's instance path (`docs/grammar.md` §9.4), so it is
deterministic and identical across process generations — and a resume request
that arrives after the restart finds it.

This is the record that makes §1 concrete. `docs/trace.md` §11 says outright
that what a human answered "is not here"; it is here.

### 3.5 The execution's lifecycle row

One row per execution, written before the graph is streamed:

| field | meaning |
|---|---|
| `id` | the execution id — what `resume` takes and what the trace's envelope carries |
| `flow` | the flow's typed address |
| `trigger` | what started it: `manual` for `agent-compose run` and for a `manual` trigger, an `http` trigger's own name where one did. **Recorded and never dispatched on** — resolved q28: recovery replays executions that exist, it does not re-fire the trigger that created them |
| `inputs` | the invocation's inputs, as the flow's `inputs:` parsed them |
| `sessionKey` | the session identity `scope: session` stores key off (`docs/grammar.md` §11.3) |
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
* `<kind>` is one of `model`, `tool`, `store`, `human` (§3).
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

**Time is not replayed.** A `timeout:` budget runs against the resumed
generation's clock, and a `human` node's own budget restarts when the wait
re-parks — a wait the journal *holds* is not re-parked at all, and replays with
the instants the recording generation measured (§3.4). A backoff's jitter is
re-rolled. None of it changes what an effect
answers; it can change *whether* a deadline fires, and a resumed execution whose
budgets fire differently is a resumed execution whose effect sequence diverges —
reported as §7 rather than silently accepted.

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
  finds its wait;
* the status route answers for a recovered execution exactly as it answers for
  one this process started;
* recovery **does not wait** for the replays to finish. The executions it
  recovers are by definition ones that were still running, and the commonest of
  them is parked on a question nobody has answered yet. Registering them is what
  has to happen before the first request; finishing them is what the resume
  route is for.

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
the seam, because the contract belongs to the node, and it is a divergence
rather than the ordinary "this answered off-contract" failure for the reason
below: an ordinary one is a node failure, a `retry:` absorbs it, and the second
attempt claims an ordinal past the frontier and **re-issues the effect live** —
the double side effect this whole document exists to prevent, reported as a bad
answer.

One case sits inside that and is **not** a divergence: a recorded answer can
fail a contract that has not moved at all, because it failed it on the
generation that recorded it too — a flaky `exec:` under a `retry:` whose first
attempt answered off-contract and whose second did not. The journal says which
happened. The ordinal counts every effect of a kind ever issued at a site (§4),
so if that ladder went round again the **next** record at the site is its second
attempt — and what identifies it as that attempt rather than as whatever the
site did next is that a retry repeats the *identical request*. So the next
record is compared by request identity, not merely counted: where it repeats
this one, the mismatch is one this composition already had and already decided,
the ladder does now what it did then, and the attempt it spends is a replay
rather than a call. Where the journal holds nothing there — or holds a different
request, which is the site going on to its next effect rather than retrying this
one — the original never went round again, so the contract is one this build
brought.

Counting alone would be wrong at every site that issues two or more effects of
one kind: an agent whose loop calls `tool.alpha` and then `tool.beta` records
`…#tool/0` and `…#tool/1`, and a tightened contract on *alpha* would find beta's
record sitting where a second attempt would have been.

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
* what a store read and what it wrote;
* **what a person answered a `human` node**.

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
  filed (§3.3), and a replayed wait carries the original `pausedAt`,
  `expiresAt` and `settledAt` (§3.4). A reader of the resumed document sees what
  the execution did, not what this process did.

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

## 10. Backends, and what v1 binds

resolved q27 makes the journal a **deploy-target slot**, exactly as
`storage_backends` are: "the composition says nothing, the target binds it".
`--target local` binds a SQLite journal file beside the project; a distributed
target will bind Postgres; both sit behind one interface so the runtime cannot
tell which it got.

In v1:

* **journaling is unconditional.** Every invocation of every flow is journaled,
  with no key to turn it off and none to turn it on. Zero configuration is the
  point (resolved q27).
* **every target this compiler can currently build is process-local**, so every
  one of them binds SQLite. There is nothing for a deploy file to choose
  between.
* **there is therefore no new grammar or schema surface.** A deploy-level
  journal configuration block arrives with the first non-local backend, which is
  the release where a choice exists to express. `docs/grammar.md` Decision D121
  records this.

**The driver is `node-sqlite3-wasm`** — the one `src/stores.ts` already opens.
PRD §9.18 makes Node a supported fallback beside Bun and gates it statically
over the whole golden corpus, so a generated module may name neither
`bun:sqlite` nor `node:sqlite`; the shared driver is also one fewer pinned
dependency.

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
* the `status` vocabulary of §3.6;
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
schema of the SQLite file, which is this implementation's rather than this
format's.

### 11.2 What is a compatible change

Made **without** a version bump: adding a field to a record type; adding a new
`EffectKind` whose absence in an older journal is simply a frontier; improving
the text of a diagnostic; changing the physical SQLite schema in a way that
reads older files.

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

* **cross-process migration** — a journal written by one host and resumed on
  another is M3-distribution's problem. The journal is a local file, and nothing
  here promises it is portable.
* **journal compaction** — nothing is pruned. The file grows with the effects a
  project has ever issued, exactly as the stores' idempotency ledger does, and
  retention is deleting it.

And one this document states rather than defers, because it is a property of the
backend v1 binds rather than a feature left out:

* **two processes on one project's journal at once.** The driver's lock keeps
  them out of each other's pages (§2), but it is a lock rather than a plan:
  neither process can see what the other's replay is doing past the frontier, so
  one process at a time writes a project's journal — `agent-compose resume`
  beside a live `serve` included. Because a held lock is broken after a deadline
  (§2), a second process does not merely queue: it eventually goes in.

Three more, all consequences of §5 rather than deferrals:

* an effect that happened with **no row** for it (§2) is re-executed on replay;
* a resumed execution whose deadlines fire differently from the original's
  diverges (§5) and is reported as §7, rather than silently re-executing;
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
