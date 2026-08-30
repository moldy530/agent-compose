# agent-compose — Distributed Execution

**Protocol version:** 1 — carried on every join, and §10 fixes what it pins and
when it bumps
**Status:** Normative for the hub/worker protocol a compiled project speaks. The
static surface it describes — `hub:` and `placements:` — is enforced today; the
runtime is being built against this document.
**Companion artifacts:** [`docs/durability.md`](durability.md) (the journal this
writes into, and the replay it extends over the wire),
[`docs/grammar.md`](grammar.md) §14.1, §14.2 (the deploy-layer surface), §9.4
(idempotency keys), [`prd.md`](../prd.md) §5.10, §5.12, resolved questions 37–44

A graph does not have to run in one process. This document defines how it runs
in several: what a hub is, what a worker is, the wire they speak, what happens
when a placed node has nobody to run it, and what v1 may never foreclose.

PRD resolved q37 fixes the topology and rules out the obvious alternative:

> Hub-and-spoke: one hub owns the graph — scheduler, journal, wait board,
> triggers — and workers execute the nodes placed on them, dialing out only.

resolved q38 fixes the binding surface — placements are logical names a worker
claims at an authenticated join, and addresses appear in no spec file; resolved
q40 fixes how code reaches a worker; resolved q42 fixes where remote effects are
journaled; resolved q43 rules out `RemoteGraph` in favour of this protocol; and
resolved q44 fixes what v1 ships and the four invariants it may not break.

**Conformance language.** MUST / MUST NOT / REQUIRED / SHOULD / MAY are used in
the RFC 2119 sense.

**What is not here.** Everything about a single-process execution:
[`docs/durability.md`](durability.md) is normative for the journal, replay, and
recovery, and this document extends it rather than restating it. Where the two
meet — an effect a worker issued, a wait a placement parked on — this document
says only what is *different* about the distributed case.

---

## Table of contents

1. [Topology and roles](#1-topology-and-roles)
2. [Transport](#2-transport)
3. [The wire contract](#3-the-wire-contract)
4. [The artifact](#4-the-artifact)
5. [Sessions and reconnection](#5-sessions-and-reconnection)
6. [Parking and wake](#6-parking-and-wake)
7. [Dispatch and replay](#7-dispatch-and-replay)
8. [Leases and single-writer-ness](#8-leases-and-single-writer-ness)
9. [Environment manifests, and the trust model](#9-environment-manifests-and-the-trust-model)
10. [Protocol stability](#10-protocol-stability)
11. [Out of v1 scope](#11-out-of-v1-scope)
12. [What is built today](#12-what-is-built-today)

---

## 1. Topology and roles

There are exactly two roles and one shape: **hub and spoke**.

The **hub** owns the graph. It runs the scheduler, writes the journal, holds the
wait board, and serves the triggers. Every execution belongs to exactly one hub
for its whole life, and the hub is the only process that decides what runs next.

A **worker** executes the nodes placed on it and does nothing else. It holds no
checkout, no YAML, no journal, and no scheduler. It learns what to run by asking.

**Workers dial out; the hub never dials in.** This is the clause that makes a
personal machine a first-class placement: a Mac behind NAT with a dynamic IP
joins the way a CI runner joins, and a placement therefore never needs an
address. It is also why `placements:` is keyed by a *name* a worker asserts
rather than by a host a spec would have to know (grammar §14.1, PRD resolved
q38).

**One execution lives on one hub, always.** Executions share nothing by
construction — global-scope stores are already external backends — so the unit of
horizontal scaling is the execution, never the node. Scaling out later means
sharding executions across hubs over the Postgres journal slot (resolved q27),
which §8 is written to keep possible.

Peer partition — each machine owning a subgraph and its own journal — is
**rejected**, and named here so it is not re-proposed as an optimisation. It
imports distributed replay, split brain, and cross-journal identity, against
resilience nobody asked for.

### 1.1 What a placement is

A placement is **capability affinity**: the machine with the signing keys, the
GPU, the one licensed tool. It is not load assignment. Which physical machine
satisfies a claim is decided by whoever shows up, and several workers claiming
one name form a **pool** the hub dispatches across.

The static surface is grammar §14.1 and §14.2, and its rules are enforced by
`validate` today: a placement is a named entry whose `members:` are distinct
`agent.*` and `tool.*` addresses that resolve; placements are disjoint; whatever
an agent attaches colocates with it — an attached tool, and everything an
attached flow reaches, since a flow-as-tool call runs inside the same tool loop;
`hub.join_token:` is required wherever placements are. **A component in no
placement executes on the hub.** That is the default, and it is why a project
with no `placements:` is an ordinary single-process deployment.

### 1.2 What the hub's load is made of

Hub load scales with **effect throughput and live state size**, not with graph
definition size: the heavy work — model calls, tool execution — happens on
workers. The walls, named so a deployment knows what it is watching:

- **journal growth.** Compaction stays the named exclusion of resolved q28.
- **fan-out state in hub memory.** A `map` holds its instances' state on the hub.
- **one JS event loop serialising large states.**

Hundreds of concurrent executions are comfortable. The answer past that is more
hubs, not a smarter one.

### 1.3 The cloud shapes are the same shape

Kubernetes and ECS are this topology with workers that never sleep on a flat
network. A container running the worker verb against a hub URL is the entire
integration: no operator, no CRDs, no second control plane to drift against the
first (PRD resolved q44).

---

## 2. Transport

**The protocol is HTTP/1.1 long-polling on the serve surface. There is no
WebSocket, and no new runtime dependency.**

The hub already serves HTTP — triggers, the status route, the resume route — and
the worker routes are more of that surface, under the same server, the same
process, and the same auth story. A worker holds one outstanding `GET` at a time
and the hub answers it when there is work for that worker or when the hold
expires.

**A worker polls while it is busy, and runs one dispatch at a time.** Both
halves are normative, and together they are the whole of the concurrency story:

- A worker keeps exactly one poll in flight from the moment it joins until it
  stops. It does **not** suspend polling while a node runs: a four-minute build
  is four minutes of holds that return empty, and the session stays inside the
  liveness window the whole time. Without this clause the window below would
  presume a working worker gone, re-park its queue, and leave it with a result
  to post against a session the hub has forgotten — a path no section of this
  document describes because no implementation may reach it.
- The hub MUST NOT answer a session's poll with a dispatch while that session
  has a dispatch it has not settled. A busy worker's polls are therefore
  heartbeats and nothing else, however much work is queued for the placement it
  claims.

Concurrency comes from **more sessions, never from more dispatches on one**:
several workers claiming one name form a pool (§1.1), and a machine that should
run two nodes at once runs two workers. That keeps a worker's own model of
itself down to one node, which is what makes `dispatch_id` idempotency (§3.4)
and the mid-node disconnect rule (§7.3) statements about a session rather than
about a scheduler nobody wrote.

A worker's `POST`s do not wait behind its poll: effect batches (§3.3) and
results (§3.4) are issued as they happen, concurrently with the held `GET`. "One
outstanding `GET`" bounds the polling, not the connection count.

The alternative was a WebSocket or an SSE stream, and long-poll wins on three
counts that matter more than elegance here. It survives every proxy, load
balancer and corporate egress path that HTTP survives, which is the population a
personal mesh actually runs on. It needs nothing the runtime does not already
have. And its failure mode is the one the rest of this document is built around:
a poll that does not come back is a worker that is gone, which is exactly the
signal §6 needs — so the liveness mechanism and the work-delivery mechanism are
the same mechanism, and there is no second one to get out of step.

The cost is a small latency floor on dispatch — one round trip after the hold is
answered — and a poll that returns empty every hold interval on an idle mesh.
Both are acceptable for a graph whose nodes are model calls.

**Timings, normative:**

| | value |
|---|---|
| poll hold | 25 seconds (the hub answers `204` at the end of a hold with no work for this session) |
| session liveness window | 90 seconds since the last request on a session |
| worker re-poll after `204` | immediately, whether or not it is executing a dispatch |
| worker re-join after a transport failure | bounded exponential backoff, starting at 1 second and capped at 30 |

A hold shorter than most intermediary idle timeouts and a liveness window
several holds wide are the two properties those numbers have; an implementation
MAY make them configurable and MUST keep that relationship. Note what the window
is measured against: **the last request on a session**, not the last dispatch.
A node that runs longer than the window is ordinary and costs nothing, because
the polls continue underneath it.

---

## 3. The wire contract

Five routes. Every one of them is authenticated, and every one after the join
carries the session the join returned.

| route | method | what it is |
|---|---|---|
| `/workers/join` | `POST` | claim placements, agree the handshake triple, receive a session |
| `/workers/poll` | `GET` | long-poll for a dispatch |
| `/workers/effects` | `POST` | hand journaled effects back to the hub |
| `/workers/result` | `POST` | settle a dispatch |
| `/workers/artifact/{hash}` | `GET` | fetch the generated project by hash |

**Authentication.** A worker presents the deploy target's join token as
`Authorization: Bearer <token>`, read from the environment variable
`hub.join_token:` names (grammar §14.2, §4.3). The token is the same on both
sides: the hub reads it to verify, the worker reads it to offer, and neither
holds it in the artifact. There is one scheme and one kind.

**The session.** Every route after the join carries the `worker_session` the
join returned, in the `X-Worker-Session` header, **in addition to** the bearer
token: the token says which mesh, the session says which worker. The one
exception is stated at §3.5 and stated there because it is an exception.

**An unknown session is `410`.** A request on a session-carrying route whose
`X-Worker-Session` the hub does not recognise — one it never issued, or one it
forgot when the process behind its name was replaced (§5) — is answered
`410 Gone`, at `/workers/poll`, `/workers/effects` and `/workers/result` alike.
That
is the **only** status meaning "the session is unknown", and on those three
routes it is the only one a worker answers by joining again (§5). Two statuses a
reader might take for it are not it, and the difference is behavioural rather
than cosmetic:

- a refusal at the **join** — `400`, `401`, `403`, `409` (§3.1) — is
  **terminal**. A second join would be refused identically, so a worker MUST NOT
  answer one by joining again;
- a `409` at **`/workers/result`** (§3.4) is about a `dispatch_id`, not about a
  session. The worker keeps the session it has, discards the result, and goes on
  polling.

§3.5's `404` is a third thing again and is stated there: a worker that meets it
re-joins, but over a *hash* it can no longer fetch rather than a session the hub
has forgotten.

### 3.1 `POST /workers/join`

```json
{
  "protocol": 1,
  "compiler": "0.4.1",
  "runtime": "bun 1.1.34",
  "claims": ["mac", "gpu"],
  "artifact_hash": "<the hash this worker already holds, if any>",
  "env_ok": ["SIGNING_KEY", "NOTARY_PASSWORD"]
}
```

The credential is **not** in the body. It is the `Authorization` header above,
and only there: a wire contract with a field that both exists and does not is
two implementations waiting to disagree, and a token in a request body is a
token in whatever logs that body.

Answer:

```json
{
  "protocol": 1,
  "compiler": "0.4.1",
  "worker_session": "wrk_9f1c8a3e…",
  "artifact": { "hash": "sha256:…", "url": "/workers/artifact/sha256:…" },
  "poll_url": "/workers/poll"
}
```

Field by field, because every refusal below is decided by one of them:

- **`protocol`** is the version of *this document* the worker speaks, and it is
  REQUIRED. It is not part of the handshake triple and is checked before it:
  the triple is about the code both sides run, `protocol` is about the wire
  carrying it (§10).
- **`compiler`** and **`runtime`** are the worker's two halves of that triple —
  the `agent-compose` release the worker was built from, and the JavaScript
  runtime it will execute the artifact under, as `"<name> <version>"` exactly as
  that runtime reports itself. Both are REQUIRED, because a refusal that names
  both sides (§4.1) cannot name a value the request never sent. The
  answer echoes the hub's `protocol` and `compiler` on the joins it accepts, so
  a worker's logs name the mesh it is *in* and not only one it was refused from.
- **`claims`** are the deploy layer's placement names.
- **`artifact_hash`** is what the worker already has on disk, which lets an
  unchanged worker skip the download.
- **`env_ok`** is the worker's report of which variables **of the manifest in
  the artifact it holds** are set in its environment — **names only, never
  values**, and never the environment's other names (§9). Its presence is
  decided by `artifact_hash`, and the next paragraph is that rule.

**When `env_ok` is sent, and why it is not always computable.** The manifest
travels *inside* the artifact — it is the per-placement partition §9.1 fixes,
emitted into the generated project, so both ends read one answer out of one
artifact hash. A worker therefore knows its manifest exactly when it holds the
artifact the hub is serving, which gives one rule with two cases:

- a join whose `artifact_hash` is the artifact the hub currently serves MUST
  carry `env_ok`, and it is checked;
- any other join — a cold start with no artifact at all, or one holding a stale
  hash — MUST omit it. A worker does not guess a manifest it has not read.

A join that omits `env_ok` is a **provisioning join**: it is answered normally,
with a session and the current artifact, and the hub **MUST NOT dispatch to that
session**. The worker fetches (§3.5), materialises (§4), and joins again — and
*that* join carries the hash and the report, and is where the `403` fires. The
check therefore still catches the machine it exists for, the one whose keychain
was never set up (§9.2), one round trip later and still before any node runs.

Nothing about this is a privileged first connection (§5): both are the same
request on the same route, either may be the first a hub ever sees, a worker
that already holds the current artifact makes only the second kind, and the hub
remembers nothing of the first — the second re-derives all of it.

**Refusals**, and the shape of each:

| condition | status | body |
|---|---|---|
| the token does not verify | `401` | no detail. A refused credential is told nothing about why |
| `protocol` names a version this hub does not speak | `409` | names both versions, and which end is behind (§10) |
| the compiler version or runtime does not match | `409` | names both sides of whichever half differs (§4.1) |
| a claim names no placement in the active target | `400` | names the claim, and lists the target's placement names |
| `env_ok` is present and a claimed placement's manifest is unsatisfied | `403` | names the **variables**, never their values, never whether the hub holds them |
| `env_ok` is present on a join whose `artifact_hash` is not the current one, or absent on one whose is | `400` | names the rule above: the report is against the manifest in the artifact the worker holds |
| the worker's artifact hash is stale | — | not a refusal: the join succeeds and the answer carries the current artifact for the worker to fetch (§3.5, §4) |

The order matters, and it is the order of the rows: a worker that cannot be
authenticated is told nothing, a worker whose wire this hub does not speak is
told that before anything about the deployment, and the placement and manifest
answers — which describe the target — are given only to a worker that has got
that far.

**A refused join is terminal.** Every row above names a condition another join
would meet identically — a credential that does not verify, a wire this hub does
not speak, a release that does not match, a claim that names no placement, a
manifest that is not satisfied, a report made against the wrong artifact. So a
worker refused at join **stops**: it exits non-zero, naming the refusal as it was
given, and starting it again is an operator's act — or its supervisor's, whose
backoff is that supervisor's business. The bounded backoff of §2 covers the
other case and only it: a join that never *completed* — a connection refused, a
socket closed, a `5xx` — which is a transport failure and says nothing about
whether this worker belongs here.

Nothing in this document tells a worker to answer a refused join with another
join, and two gates depend on that. A worker that met `409` with a re-join would
spin against a hub that has told it, correctly and permanently, that it is the
wrong release — which is precisely what §10.3 relies on `409` to enforce. A
worker that met `401` with a re-join would do the same to a hub whose join token
was rotated out from under it, hammering it with a credential that cannot start
working. The disposable-session rule of §5 is about `410` and about nothing
else.

The token check MUST be a constant-time comparison, for the reason grammar §13.3
gives about inbound trigger credentials: a caller who can time a refusal
otherwise recovers the token byte by byte.

### 3.2 `GET /workers/poll`

Carries the session. Long-polls for up to one hold (§2).

- **`200`** with a dispatch:

  ```json
  {
    "dispatch_id": "dsp_…",
    "execution_id": "exec_…",
    "node": "flow.release.sign",
    "instance_path": "flow.release#0.sign",
    "inputs": { "…": "…" },
    "effect_history": [ { "…": "…" } ]
  }
  ```

  `instance_path` is the flattened path grammar §9.4 fixes as the idempotency
  key, so the worker computes the same keys the hub would. `effect_history` is
  the journaled record of effects this node instance already issued, and is what
  a redispatched node replays to the frontier before going live (§7).

- **`204`** when the hold expired with no work **for this session** — which
  includes every hold while the session has a dispatch it has not settled (§2).
  A busy worker keeps polling and keeps being answered `204`, however deep the
  queue for the placement it claims; that is the mechanism, not a degenerate
  case of it.

- **`410`** when the session is unknown (§3): the worker joins again and resumes
  polling under the session that join returns. A hub that has just been replaced
  behind its name answers every worker this way, once each, and that is the whole
  of what a hub restart costs (§5).

- **`401`** when the token does not verify, with no detail, as everywhere
  (§3.1) — and terminal in the same sense: the credential is the one the join
  used, so re-joining cannot improve it.

**Polling is the heartbeat.** There is no separate liveness route, and there MUST
NOT be one: the property §6 needs is "is this worker still there", and a poll is
the only request that answers it — for an idle worker and a busy one alike,
which is why §2 requires the poll to continue while a node runs. A session with
no request inside the liveness window is presumed gone; §6.3 fixes what that
costs, which is nothing at all unless the session was holding a dispatch, and
that dispatch's node fails an attempt.

### 3.3 `POST /workers/effects`

Carries the session. A batch of effect records the worker produced while
executing a dispatch, in the order it produced them.

**The hub is the single writer, and this route is what preserves that: workers
SEND, the hub INSERTS.** No worker ever touches the journal, which is why SQLite
stays a valid backend for a personal mesh — there is still exactly one writer
(PRD resolved q42).

Each record carries its **effect key**, derived by the execution-derived rule
grammar §9.4 fixes, and insertion is **idempotent by that key**: a record the
journal already holds is accepted and dropped. A batch is therefore safe to
re-send after a transport failure, and a worker SHOULD re-send rather than
guess.

| condition | status | body |
|---|---|---|
| the batch is journaled | `204` | empty. Every record in it was inserted or was already held |
| the token does not verify | `401` | no detail, as everywhere (§3.1) |
| the session is unknown | `410` | names the rule of §3: join again, and send this batch again under the new session |

**A batch answered `410` is re-sent, never dropped.** The records are keyed by
effect key and scoped to their execution (§7.1) — not by session, and not by
dispatch — so the journal takes them from whichever session hands them over, and
the hub is still the single writer that inserts them. A worker that discarded
the batch instead would hand the redispatch of §7.2 an `effect_history` short of
the frontier, and the node would re-issue an effect the journal was owed: the
model call §7.3 promises is not paid for twice, paid for twice.

A worker SHOULD send a batch as soon as an effect completes rather than
accumulating until the node ends, because an effect that never reached the hub is
an effect the replay of §7 cannot skip.

### 3.4 `POST /workers/result`

Carries the session and the `dispatch_id`. The node's outcome: its output, or
its failure.

**Idempotent by `dispatch_id`.** A result for a dispatch the hub has already
settled is accepted and dropped, which is what makes at-least-once dispatch (§7)
safe on the return path as well as the outbound one.

A result the hub cannot attribute — an unknown or already-superseded
`dispatch_id` — is answered `409` and the worker discards it: the execution has
moved on, and re-driving it from a stale result is exactly the divergence
`docs/durability.md` §7 refuses. The commonest way a `dispatch_id` becomes
superseded is §6.3: the hub gave up on the session this dispatch was issued to,
and the node has been through its `retry:` chain since.

| condition | status | body |
|---|---|---|
| the dispatch settles, or was already settled | `204` | empty |
| the token does not verify | `401` | no detail, as everywhere (§3.1) |
| the session is unknown | `410` | names the rule of §3: join again, and post this result again under the new session |
| the `dispatch_id` is unknown or already superseded | `409` | names the dispatch. The result is discarded |

**`410` and `409` are different failures and a worker MUST NOT treat them
alike.** `410` says *the hub does not know you*, and the result is still owed:
join, and post it again under the new session — the hub attributes it by
`dispatch_id`, which the new session does not change. `409` says *the hub knows
you and does not want this*, and the result is dead. A worker whose re-posted
result meets `409` has its answer and stops re-posting; a worker that read the
two as one would either abandon a result the hub was waiting for or re-drive an
execution that has moved past it.

### 3.5 `GET /workers/artifact/{hash}`

The tarball §4 describes, at the `url` the join returned. `{hash}` is the
content hash, written the way §3.1's answer writes it (`sha256:…`), and the
route is **hash-addressed**: one hash names one body, forever, so a proxy or a
worker may cache it without a validator.

**Authentication is the bearer token alone.** This is the one route that does
*not* require a session, and the exception is deliberate: a worker whose session
has aged out re-joins and fetches, and a fetch is a large, resumable, cacheable
transfer that should not be coupled to a session's lifetime. A worker SHOULD
present its session when it has one; a hub MUST NOT require it.

| condition | status | body |
|---|---|---|
| the token does not verify | `401` | no detail, as everywhere (§3.1) |
| the hash is the artifact this hub serves | `200` | the tarball, `Content-Type: application/gzip`, `Content-Length` set |
| the hash is a **previous** artifact this hub still holds | `200` | the same, so a worker mid-rollback is not stranded |
| the hash is well-formed and unknown here | `404` | names the hash, and the hash this hub currently serves |
| the hash is malformed | `400` | names what a hash looks like |

A `401` here is terminal in the sense §3.1 gives it: this route takes the join's
own credential, so joining again cannot improve it. There is no `410` on this
route, because there is no session on it to be unknown.

A hub MUST keep serving an artifact while any execution that was dispatched
under it is unfinished, and MAY drop it afterwards. A worker meeting `404` for
the hash its join returned re-joins rather than retrying the fetch: the join is
what re-derives the current hash, and re-deriving it anywhere else would be a
second answer to what this deployment is running. That re-join is over a *hash*,
not a session — it is the one re-join in this document that no `410` asked
for — and the join it makes is an ordinary one, refused terminally if it is
refused at all.

Range requests are OPTIONAL. A hub that serves them MUST honour
`Accept-Ranges: bytes` semantics; a worker MUST work without them.

---

## 4. The artifact

**The hub ships the whole generated project.** Workers hold no checkout and no
YAML; redeployment is automatic on the next join, and version skew is a refusal
rather than a drift (PRD resolved q40).

The artifact is a **tarball of the generated project** — the same tree
`build` writes — served **hash-addressed** from the hub under worker
authentication, on the route §3.5 fixes. Its hash is over the tree's content, so
two hubs built from one composition serve one artifact.

A worker:

1. compares the hash the join returned with what it holds;
2. fetches the tarball when they differ, and verifies the hash it computed
   against the hash it asked for before unpacking anything;
3. materialises it under its own data directory, keyed by hash, so the previous
   artifact survives a rollback;
4. installs dependencies with `bun install`;
5. **joins again**, now carrying that hash and — read out of the tree it just
   unpacked — the `env_ok` report §3.1 requires of a worker holding the current
   artifact;
6. executes dispatches out of that tree.

Step 5 is why a fetch is followed by a join rather than by a poll: the manifest
lives in the artifact (§9.1), so the report the hub checks is one the worker
could not make until this moment. A worker that already held the current hash
finds step 1 equal, skips 2 through 4, and is dispatchable on the join it made
in the first place — which is the ordinary steady state, one join and no
download.

### 4.1 The handshake triple

A join agrees on three values, and all three are compared — so all three are on
the wire, in the join §3.1 fixes:

| | what it pins | where it is written |
|---|---|---|
| **artifact hash** | the generated project, exactly | `artifact_hash` in the request; `artifact.hash` in the answer |
| **compiler version** | the `agent-compose` release that generated it | `compiler` in the request, and in the answer |
| **runtime** | Bun, and its major version | `runtime` in the request, as `"<name> <version>"` |

A field the request omits is a comparison the hub cannot make, which is why all
three are REQUIRED rather than helpful: PRD resolved q40 makes the refusal the
point of the handshake, and a refusal that names both sides is only writable
from a request that carries one of them.

A hash mismatch is not a refusal — the answer carries the current artifact and
the worker fetches it.

That is narrower than resolved q40 reads at a glance — "a mismatch is a refused
join naming both" — and the reconciliation is in q40's own next clause:
*redeployment is automatic on the next join*. The two are one decision. An
artifact hash is the single member of the triple the hub can **fix in the answer
it is already sending**, so fixing it is what "automatic" means, and refusing it
would break the resolution rather than honour it — it would also strand §3.1's
provisioning join, which is a join made with no artifact at all. The two members
the hub cannot fix — the release a worker binary was built from, the runtime
installed on its machine — are the refusals, and they are what "naming both" is
about. Read it the way §4.2 reads Bun against resolved q18: a *scoped reading*
of the resolution, written down so the next reader does not have to re-derive
it.

A **compiler-version or runtime mismatch is a refused join** (`409`, §3.1), and
the refusal names both sides:

> refused: this hub was built by agent-compose 0.4.1 and the worker runs 0.3.9 —
> upgrade the worker, or point it at a hub of its own release

> refused: a worker executes the generated artifact under Bun 1.x and this one
> runs node v22.3.0 — install Bun, or run this placement on a machine that has it

The hub compares `runtime`'s **name and major version** and nothing finer, and
reports the whole string it was given: a worker on Bun 1.2 where the hub's
artifact was built against Bun 1.1 is not a mismatch, and a worker on Node is
one however new it is. What pins the required major is the compiler release —
the same release that pins the LangGraph version — so the two halves of the
triple's second and third rows move together and a worker satisfying `compiler`
satisfies the major it implies.

The **protocol version is not one of these three**, and §10 is where it lives.
The triple is a statement about the code both ends run; `protocol` is a
statement about the wire that carries it, and a hub may go on speaking to
workers of an older release long after it stops accepting their artifacts, or
the other way round.

### 4.2 Bun, and resolved q18

**Workers are Bun-only.** That is a *scoped exception* to resolved q18's Node
fallback rather than a contradiction of it, and the scope is the point: the
worker is a surface **around** the artifact, which is q18's own reading of where
the fallback stops. Pinning one runtime keeps the handshake a single (artifact
hash, compiler version, runtime) triple instead of a matrix of them.

The fallback remains the whole story for a generated project somebody runs by
hand: `build` still emits a project that runs under Node, and nothing in this
document changes that.

### 4.3 Slicing is deferred, named

A worker receives the whole artifact, including the code and prompts of nodes it
will never run. Per-placement **slicing** is deferred and named in PRD resolved
q40: on a personal mesh every machine is the owner's, and slicing is an
optimisation with a real complexity bill — a per-placement build, a per-placement
hash, and a reachability analysis that has to agree with the scheduler's.

One consequence is load-bearing elsewhere and is stated here because this is
where it comes from: **the whole artifact is everywhere**, so a placement decides
which *process* runs a node rather than which code exists where. Grammar §14.1's
attached-tool rule follows directly from it.

---

## 5. Sessions and reconnection

**Sessions are advisory.** A `worker_session` is a handle for correlating a
poll, an effect batch and a result; it is not state the protocol depends on.

Three rules make it so, and together they are PRD resolved q44's second
invariant:

1. **Every re-join re-derives everything from the journal.** What is dispatched,
   what is parked, what a redispatched node has already done — all of it is read
   out of the journal at the moment it is needed, never out of session memory.
2. **There is no privileged first connection.** A worker's tenth join is
   identical to its first. No handshake step may be "only done once", and no hub
   may hold state a reconnecting worker cannot re-establish by joining.
3. **Workers address a hub *name*, not a process.** A worker is configured with a
   URL, and replacing the process behind that URL is invisible to it.

The reason is failover. Any process pointed at the journal recovers every
execution — that is the failover primitive `docs/durability.md` §6 already
proves — and these three rules are what stop the *worker protocol* from being
the thing that makes it impossible. A hub restarted under a stable name loses
sessions and nothing else: workers' next requests are answered `410` — the
status §3 gives an unknown session at every session-carrying route — they join
again, and dispatch resumes from the journal.

A worker MUST therefore treat a session as disposable: **a `410` is answered by
joining again and retrying the request that met it**, not by failing. The
converse is equally binding, and is what keeps that rule from becoming a loop: a
**refused join** is not a session problem and MUST NOT be answered by another
join (§3.1). The two cases are told apart by the status and by nothing else,
which is why §3 gives `410` exactly one meaning and gives that meaning to no
other status.

---

## 6. Parking and wake

**A placed node with no live worker parks.** It is a third kind of wait on the
existing board — journaled, restart-surviving, status-visible — beside a `human`
pause and an undelivered callback (PRD resolved q39).

### 6.1 The identity

A placement wait carries the **same deterministic wait identity** a `human` wait
carries: the node's instance path plus an ordinal (grammar §9.4,
`docs/durability.md` §4). Deterministic, because a resumed execution has to
re-park under the identity it parked under before — the property that lets a
resume prepared against a dead process still find its wait.

### 6.2 The wake is a join

A joining worker's claims are scanned against the open placement-waits, and
dispatch resumes **in park order**. This is the same scan-and-resume that
recovery runs for `human` waits (resolved q28) and that resolved q35 runs for
undelivered callbacks.

The join that wakes is a join the hub may **dispatch** to, which excludes the
provisioning join of §3.1: a worker that has not yet reported against its
manifest has not been confirmed able to run the work, so its wake is the re-join
a moment later, after it has the artifact. Nothing is lost by waiting — the wait
is on the board, and park order is read from the journal at the join that takes
it, not held from the one before.

**There is no polling anywhere in this**, and that is a property rather than an
implementation detail: the wake is an event the hub already receives, so a mesh
with nothing to do costs one held request per worker and nothing else.

### 6.3 Heartbeat loss ends the session, and its dispatch fails

**The hub declares a session gone, and the declaration is the liveness window of
§2 expiring**: 90 seconds with no request on that session — no poll, no effect
batch, no result. That is the only detector this protocol has, and the hub is
the only side that may fire it: a worker never declares itself gone, it re-joins
(§5).

What the declaration costs depends on the one thing a session can be holding.
§2's pull model queues work to a **placement**, never to a worker, and hands it
over one dispatch at a time, so a session holds at most one piece of work: the
dispatch it has not settled. Two cases, and they are the whole rule.

- **The session had no unsettled dispatch.** Nothing re-parks, because nothing
  was ever this worker's. The placement's open placement-waits stay on the board
  untouched, and the next join claiming that placement takes them in park order
  (§6.2). This is the sleeping laptop, and it costs the execution nothing.
- **The session had an unsettled dispatch.** The hub **abandons** it: the
  dispatch is settled as failed, and the node's attempt fails with it, under that
  node's `retry:`/`on_error:` chain exactly as §7.3 says — the liveness window is
  what *detects* the mid-node disconnect §7.3 describes, and this paragraph is
  where that detection is filed. Effects the worker had already streamed home
  stay in the journal and are handed to the next attempt as its `effect_history`
  (§7.2). Where `retry:` grants that attempt, it re-enters dispatch and **parks
  on the board if no worker is claiming the placement** (§6.2) — which is the
  sense in which heartbeat loss re-parks.

A revived worker that comes back and posts a result for an abandoned dispatch is
answered `409` and discards it (§3.4): that is the "already-superseded" case, and
abandoning the dispatch is what supersedes it. Effect batches it still holds are
a different matter and are still accepted — they are keyed by effect key and
scoped to their execution, not to a session or a dispatch (§3.3) — so a batch
in flight when the lid closed is journaled rather than lost.

PRD resolved q39 puts this as "heartbeat loss re-parks what was queued to the
vanished worker", and §2's pull model is what makes that phrase concrete rather
than ambiguous: the only work ever queued *to a worker* is the dispatch it is
holding, so the clause resolves to the second case above and leaves nothing over.

### 6.4 Undispatched is a pause; mid-node is a failure

The distinction is the one PRD resolved q39 and q42 draw together, and it decides
what an author writes:

| | what it is | what governs it |
|---|---|---|
| **no worker has taken the node yet** | a **pause** — this is what parking is | nothing, until the node's `timeout:` chain fires from dispatch |
| **a worker took it and disconnected mid-node** | an **attempt failure**, declared by the hub when the liveness window passes (§6.3) | the node's `retry:` / `on_error:` chain, like any execution failure (§7) |

A closed laptop is a pause, not a failure — for the undispatched case. A
placement worth sleeping on therefore wants a `retry:`, because a worker that
vanishes *while running the node* fails that attempt. Which row a given silence
falls in is decided by nothing but whether that session was holding a dispatch,
and §6.3 is where that is read off.

### 6.5 Bounding needs no new grammar

"Fail if the machine is not up in ten minutes" is already spellable: the node's
`timeout:`/`on_error:` chain applies from dispatch (grammar §9). No key is added
for placements.

### 6.6 The lifecycle webhook

A parking MAY fire the `parked` lifecycle webhook (resolved q34), so a subscribed
system learns "waiting for the Mac" exactly the way it learns "waiting for a
human". The delivery is an ordinary journaled callback: at-least-once, bounded
retry, ordered by event ordinal rather than by arrival
(`docs/durability.md` §3.7).

---

## 7. Dispatch and replay

**Dispatch is at-least-once.** A hub that cannot tell whether a dispatch arrived
re-issues it, and correctness comes from idempotency rather than from delivery
guarantees — the same posture every other effect in this project takes.

### 7.1 The idempotency rule

Effect keys are derived by the **execution-derived rule** grammar §9.4 fixes:
the flattened instance path, scoped to the execution. Its application here is the
next after detach sinks, event dedupe, store writes and callback delivery
(resolved q26, q35), and it means a redelivered dispatch re-issues nothing the
journal already holds.

### 7.2 Redispatch hands over the history

A redispatched node is given its **journaled effect history** in the dispatch
payload (§3.2), and the worker **replays to the frontier**: it re-runs the node
function, consuming recorded effects in order instead of issuing them, and goes
live at the first effect the journal does not hold.

This is `docs/durability.md` §5's replay discipline, extended over the wire, and
it is the reason this protocol exists rather than `RemoteGraph` (resolved q43):
a served-subgraph seam makes the whole remote call **one opaque effect** from the
hub's view, so replay-to-frontier dies at the boundary unless the remote keeps a
journal of its own — which is the two-journal shape §1 rejects.

### 7.3 Mid-node disconnect

A worker that disconnects mid-node fails that **attempt**, under the node's
`retry:`/`on_error:` chain like any execution failure. The effects it had already
streamed home are in the journal, so the retry replays them rather than
re-issuing them: a model call already paid for is not paid for twice.

**What detects the disconnect is the liveness window of §2, and the hub is the
side that declares it** — §6.3 states that rule and this section is its
consequence. A disconnect is therefore never observed as such: it is a session
that stopped making requests while holding a dispatch, which the hub abandons,
which fails the attempt. A worker that comes back afterwards learns so from the
`409` its result meets (§3.4), and takes part in the retry only by joining like
anyone else.

### 7.4 What LangGraph is here

LangGraph remains the per-process execution engine on **both** sides. The hub
runs the graph; a worker runs the node function the graph would have run. Nothing
about this protocol is a second execution engine.

---

## 8. Leases and single-writer-ness

PRD resolved q44's third invariant, and the one most easily broken by accident:

> single-writer-ness is the journal backend's **lease**, never a protocol or
> filesystem assumption — and the lease's **granularity is an execution set, not
> the journal**.

Three rules follow, and they are binding on the journal schema and on this
protocol equally.

1. **A lease is over a set of executions.** SQLite's file lock happens to grant
   the whole journal as one set, and that is a property of that backend rather
   than a definition. The Postgres backend (resolved q27's slot) **MUST** offer
   per-execution or per-shard leases before multi-hub exists.
2. **Effect-record identity is execution-scoped**, and MUST stay so. No key, no
   index, and no uniqueness constraint may be journal-global.
3. **No schema or protocol may assume one writer per journal.** Concretely: no
   monotonic counter shared across executions, no "last row wins" read that spans
   them, no dispatch state held anywhere but the journal.

The assumption these forbid is the one that would make resolved q37's
execution-sharded scale-out a *migration* instead of "run more hubs". Keeping it
out is cheap now and expensive later, which is why it is an invariant rather than
a plan.

**Ingress obeys the same discipline** (resolved q44's fourth invariant): URLs
derive from a configurable public base — `hub.public_url:`, grammar §14.2 — and
routes resolve ownership through the journal, so a load balancer in front of
one-or-many hubs changes nothing about what a URL means.

**The hub is a singleton by deployment choice, not by design assumption.** HA
later is running the standby and arbitrating the lease, not a rewrite.

---

## 9. Environment manifests, and the trust model

**Each worker satisfies its own placements' env manifest locally. The hub holds
only its own. Spoke secrets never transit the wire** (PRD resolved q41).

### 9.1 The partition rule

The per-placement manifest is **computable statically**: placements bind
components, components bind `${ENV}` references, and the walk that collects them
is the one `src/env.ts` is already built from (grammar §4.3, PRD 5.9).

Because it is computed at build time it **ships in the artifact**, and that is
load-bearing rather than incidental. One partition is emitted into the generated
project, so the hub reading it and a worker reading it are reading one answer
under one artifact hash — there is no second derivation to disagree, and the
handshake that pins the artifact (§4.1) pins the manifest with it. It is also
what makes §3.1's `env_ok` computable at all: a worker knows which variables to
report exactly when it holds the artifact naming them.

**The partition follows where a component *executes*, not what `members:`
says.** That distinction is the whole of this section, because the two differ:
grammar §14.1 rule 4 makes an attached tool run in its agent's process whether or
not it names a placement, so `members:` under-describes the set of things a
worker runs. A manifest derived from raw membership would ask the hub to hold a
secret it never uses and would leave a worker's manifest missing one it does.

A process's manifest is therefore the variables referenced by every component
that **can execute in it**, and a component executes in a process exactly when:

- **it is a member of that placement** — the direct case;
- **it is dispatched with no placement of its own, and the process is the
  hub** — the default of §1.1;
- **an agent that runs in that process attaches it** — an attached `tool.*`, and
  every `agent.*` and `tool.*` an attached `flow.*` reaches, all of which run
  inside the agent's tool loop (grammar §5.4, §14.1 rule 4).

Which gives, concretely:

- a variable referenced by a member of placement `P` belongs to `P`'s manifest;
- a variable referenced by a tool that only ever runs inside placed agents'
  processes belongs to **their placements'** manifests — and to the hub's only
  if some unplaced agent or `function:` node also reaches it;
- a variable referenced by a component that runs on the hub — in no placement,
  and reached by nothing placed — belongs to the **hub's** manifest;
- a variable referenced from the deploy layer itself — a storage backend, an
  event source, `hub.join_token:` — belongs to the hub's;
- a variable reachable in two processes belongs to both. Two placed agents
  attaching one unplaced tool is the ordinary case, and the tool's secrets go to
  both placements.

Worked, because this is the case the rule exists for: `tool.sign` carries
`KEYCHAIN_PASSWORD` in its `exec.env`, `agent.signer` attaches it, and the deploy
file places `agent.signer` in `mac` and nothing else. `validate` accepts that —
§14.1 rule 4's first row, the tool claims nothing and runs where the agent runs.
`KEYCHAIN_PASSWORD` belongs to **`mac`'s** manifest and **not** to the hub's: the
hub never runs `tool.sign`, so requiring the secret there would be a false
requirement, and omitting it from `mac`'s would let a machine without a keychain
join clean (§9.2) and fail on its first dispatch.

This is §7 M3's least-privilege line in its sharpest form: blast-radius
containment falls out of the manifest, because **the hub cannot leak what it
never held** — which is a claim about what the hub *runs*, and is only true if
the partition is computed that way.

### 9.2 The join-time check is self-reported presence

A worker claiming `mac` without `SIGNING_KEY` set is refused at join with the
variable named (§3.1) — the same posture the serve launch check takes
(resolved q32).

**What that check is, precisely: self-reported presence, not authenticated
capability.** The worker sends, of the manifest §9.1 partitions, the *names* it
has; the hub compares them against the manifest it computed. Proving the worker
holds the right *value* would mean sending the value, which is the one thing this
whole section exists to forbid. The check catches the misconfiguration it is
for — a machine joined before its keychain was set up — and claims nothing more.

**The report is scoped to the manifest at both ends**, and that is a privacy
property as much as a protocol one: `env_ok` is a subset of a list the hub
already computed, never an inventory of what else the machine happens to hold.
A worker that shipped the names of its whole environment would be telling the
hub about every unrelated credential on the box — the opposite of what §9.1
buys.

Which is what fixes *when* the report can be made. The manifest is emitted into
the artifact, so a worker can name its own placements' variables exactly when it
holds the artifact the hub serves, and §3.1's rule follows from that and not
from a preference: a join carrying the current hash carries `env_ok` and is
checked; a join without one omits it, is answered with the artifact, and is
dispatched nothing until the worker has fetched, materialised and joined again.
The refusal therefore lands on the second join of a cold start and on the first
of every join after — never on a machine that could not yet have known what to
report.

### 9.3 The trust model, stated plainly

**Holding the join token is being trusted with the mesh.** That means, in v1 and
without qualification:

- **the whole artifact** — every prompt, every tool implementation, every node's
  code, because slicing is deferred (§4.3);
- **the right to claim any placement** — the token is one credential for the
  target, not one per placement;
- **the journal's effect stream** — a worker sees the effect history of every
  node it is dispatched, which is what replay requires (§7.2).

Containment of **secrets** is what §9.1 buys, and mesh **membership** is what the
single token costs. The two claims should not be read as one. Per-placement
credentials are an additive later hardening — a second credential kind, scoped at
join — and are not something this key pretends to be.

This is stated here, in grammar §14.2, and in the `missing-join-token`
explanation, because a security property a reader could assume wrongly is worse
than one they have to look up.

---

## 10. Protocol stability

`PROTOCOL_VERSION` is what a join pins. Every join carries the version the worker
speaks (§3.1), and a hub that does not speak it refuses the join by version,
naming both, rather than accepting a worker it will misunderstand. This is
`docs/durability.md` §11's discipline applied to a wire instead of a file, and
the one difference is worth stating: a journal is read by a later build of the
same program, while this protocol has **two** programs on it at once, so the
compatible-change list below is what lets a hub and a worker of adjacent
releases talk at all.

### 10.1 What an implementation may rely on

At a given `PROTOCOL_VERSION`:

* the routes of §3 — their paths, their methods, and every field this document
  names, under the name and with the meaning given here;
* where the credential goes (`Authorization: Bearer`) and what the session
  header is called;
* that a join is unprivileged and repeatable (§5): any join may be the first,
  and a worker may re-join at any time without losing anything it has not been
  told about;
* that effect insertion is idempotent by effect key (§3.3) and that result
  settlement is idempotent by `dispatch_id` (§3.4) — the two properties that
  make at-least-once dispatch safe on both directions of the wire;
* that a dispatch's `instance_path` is the flattened path grammar §9.4 keys
  effects by, so a worker derives the keys the hub would;
* the status this document gives each refusal, and the rule that a refusal
  naming a variable names **names** (§9);
* that `410` at a session-carrying route means the session is unknown and a join
  is what fixes it, and that a refusal at `/workers/join` means the opposite —
  another join is refused identically (§3, §3.1, §5). These two are the whole of
  what a worker's error handling has to decide, and both are load-bearing: an
  implementation that read them the other way round would either fail a mesh a
  hub restart should have healed, or hammer a hub that has refused it.

An implementation MUST NOT rely on the *text* of any refusal, which is written
for a person and is improved between releases; nor on the internal shape of a
`worker_session` or a `dispatch_id`, which are the issuing hub's and are opaque
to everyone else; nor on the layout of the artifact, which belongs to the
compiler release and is pinned by the triple (§4.1) rather than by this version.

### 10.2 What is a compatible change

Made **without** a version bump: adding an OPTIONAL field to a request or an
answer, which a peer of the previous version ignores; adding a route a worker
may decline to use; widening a refusal body; improving refusal text; lengthening
the poll hold (§2), which a worker learns by waiting.

And the clause that carries the most weight: **a change to the artifact is not a
change to this protocol.** This wire moves a payload it does not define — node
inputs, effect records, the journal's shape — and all of it is pinned by the
handshake triple and refused at join when it differs. Such a change is
`docs/durability.md` §11's business and the compiler version's, not
`PROTOCOL_VERSION`'s, which is exactly why the triple is checked separately from
the version (§4.1).

### 10.3 What requires a version bump

`PROTOCOL_VERSION` MUST be incremented for any change that would make a peer of
the previous version behave **wrongly** rather than be refused:

* removing or renaming a field, or changing what one means;
* changing a route's path or method, or *when* it may legally be called —
  including whether a poll may be outstanding while a dispatch is unsettled
  (§2), which an older worker would answer with silence the hub would read as
  death;
* changing which side writes the journal (§3.3), or where an idempotency key
  comes from (§3.3, §3.4);
* changing what a status code means at a route — a `410` that stopped meaning
  "re-join" is the sharpest case, because §5 tells workers to act on it, and a
  join refusal that started meaning it is the same case from the other side;
* shortening the **liveness window** of §2. Lengthening it is compatible;
  shortening it is not, because the first thing an older worker learns about the
  new one is that its work was re-parked underneath it.

A bump is a statement that workers of the older release cannot join this hub,
and §3.1's `409` is what enforces it. Because a worker and a hub are built from
one compiler release, the ordinary upgrade path never meets this: the triple
refuses the mixed pair first, and `PROTOCOL_VERSION` is what remains for the
case the triple stops covering — a worker binary somebody upgrades separately,
or a second implementation of this document.

---

## 11. Out of v1 scope

Named, so that each is a decision rather than a gap (PRD resolved q44):

| | why, and what would change |
|---|---|
| **multi-hub** | §8's invariants keep it possible; shipping it needs the Postgres lease and an arbitration story |
| **worker-to-worker edges** | every edge goes through the hub, which is what keeps one scheduler and one journal |
| **per-placement artifact slicing** | §4.3: an optimisation with a per-placement build, hash and reachability bill |
| **Windows workers** | owner call, 2026-08-29. Linux and macOS in v1 |
| **containment beyond the process boundary** | a worker runs the artifact with its own privileges, exactly as a hand-rolled `exec:` tool does today. Containers and seccomp are a deployment's business, not this protocol's |

Two of those are worth reading as a pair: **no worker-to-worker edges** and
**one execution on one hub** are the same decision seen from two sides, and
together they are why there is exactly one scheduler and exactly one journal to
reason about.

---

## 12. What is built today

**The static surface is live. The protocol is not built yet.**

What `validate` enforces now: everything grammar §14.1 and §14.2 state — a
placement's name and members, the `flow.*` deferral, disjointness, repeated
members, the colocation rule for an attached tool and for what an attached flow
reaches, the conditional join token, and the `public_url:` shape. A deploy file
that breaks one of those is a compile error today.

What does not exist yet: the worker verb, the five routes of §3, the artifact
server of §3.5, placement waits on the board, and effect streaming. **Nothing a
placement or a `hub:` block declares reaches the project a build emits.**

That middle state is deliberate and it is bound rather than remembered:
`crates/compose-core/tests/placement_surface_inertness.rs` asserts that no
placement or hub material reaches a generated project, pins the sentences in this
document and in the grammar that say so, and enumerates what the runtime pass has
to unwind — including teaching the environment manifest about §9.1's partition,
which is the half of the surface with no sentence of its own.

This document is what that runtime will be held to.
